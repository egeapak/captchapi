//! Configuration layers and the code that reads them.
//!
//! Every layer resolves the *same* canonical keys (the `env` column of
//! [`PARAMS`](super::params::PARAMS)), which is what lets the CLI plug in as just another
//! [`EnvProvider`] and leaves [`Config::from_env_provider`](super::Config::from_env_provider)
//! doing the parsing and validation for all of them.

use super::params::{by_field, by_toml, Kind, Param, PARAMS};
use super::EnvProvider;
use std::collections::BTreeMap;
use std::env;
use std::path::Path;

/// A single layer of configuration: canonical key -> raw string value.
pub type Layer = BTreeMap<String, String>;

/// Which layer supplied a value. Reported by `config show` so a layered setup stays debuggable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A command-line flag.
    Cli,
    /// A process environment variable.
    Env,
    /// An env file (`.env` or `--env-file`).
    EnvFile,
    /// The TOML config file.
    File,
    /// Carried over from the running config because the field is boot-only (reload only).
    Carried,
    /// No layer supplied it; the built-in default applies.
    Default,
}

impl Source {
    /// Short label used in `config show` output.
    pub fn label(&self) -> &'static str {
        match self {
            Source::Cli => "cli",
            Source::Env => "env",
            Source::EnvFile => "env-file",
            Source::File => "file",
            Source::Carried => "carried",
            Source::Default => "default",
        }
    }
}

/// The stack of configuration layers, in precedence order.
///
/// `cli` > `env` (process) > `env_file` > `file` (TOML) > `carried` > built-in default.
///
/// The env file sits *below* the process environment to preserve today's behaviour: the current
/// `dotenvy::dotenv()` call does not overwrite variables that are already set.
pub struct LayeredEnv<'a, E: EnvProvider> {
    pub cli: &'a Layer,
    pub env: &'a E,
    pub env_file: &'a Layer,
    pub file: &'a Layer,
    /// Boot-only values carried over from the running config during a reload, so a reload cannot
    /// fail just because a secret file was rotated away after startup.
    pub carried: &'a Layer,
}

impl<'a, E: EnvProvider> LayeredEnv<'a, E> {
    /// Build a stack from the four explicit layers.
    pub fn new(
        cli: &'a Layer,
        env: &'a E,
        env_file: &'a Layer,
        file: &'a Layer,
        carried: &'a Layer,
    ) -> Self {
        Self {
            cli,
            env,
            env_file,
            file,
            carried,
        }
    }

    /// Report which layer would answer `key`, probing in the same order as [`EnvProvider::get`].
    ///
    /// This is a separate pure query rather than something recorded during `get`, because
    /// recording would need interior mutability, and a `RefCell` here would make `LayeredEnv`
    /// `!Sync` — which breaks the moment a reload runs inside an async task.
    pub fn source_of(&self, key: &str) -> Source {
        if self.cli.contains_key(key) {
            Source::Cli
        } else if self.env.get(key).is_ok() {
            Source::Env
        } else if self.env_file.contains_key(key) {
            Source::EnvFile
        } else if self.file.contains_key(key) {
            Source::File
        } else if self.carried.contains_key(key) {
            Source::Carried
        } else {
            Source::Default
        }
    }
}

impl<E: EnvProvider> EnvProvider for LayeredEnv<'_, E> {
    fn get(&self, key: &str) -> Result<String, env::VarError> {
        self.cli
            .get(key)
            .cloned()
            .or_else(|| self.env.get(key).ok())
            .or_else(|| self.env_file.get(key).cloned())
            .or_else(|| self.file.get(key).cloned())
            .or_else(|| self.carried.get(key).cloned())
            .ok_or(env::VarError::NotPresent)
    }
}

/// Read a secret from a file, trimming surrounding whitespace.
///
/// Trimming matters because Docker and Kubernetes secret files almost always end in a newline.
pub fn read_secret_file(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read secret file {}: {}", path.display(), e))?;
    let value = raw.trim().to_string();
    if value.is_empty() {
        return Err(format!("secret file {} is empty", path.display()));
    }
    Ok(value)
}

/// Load an env file (`KEY=VALUE` lines) into a layer.
///
/// Unlike `dotenvy::dotenv()`, this does not touch the process environment. Keeping it as a
/// layer means it participates in precedence explicitly, it is re-read on reload, and tests
/// never race on global state.
pub fn load_env_file(path: &Path) -> Result<Layer, String> {
    let iter = dotenvy::from_path_iter(path)
        .map_err(|e| format!("cannot read env file {}: {}", path.display(), e))?;
    let mut layer = Layer::new();
    for item in iter {
        let (key, value) =
            item.map_err(|e| format!("invalid line in env file {}: {}", path.display(), e))?;
        layer.insert(key, value);
    }
    Ok(layer)
}

/// Load and validate a TOML config file into a layer of canonical keys.
///
/// Unknown paths are a hard error so a typo such as `[server] prot = 8080` fails loudly at boot
/// instead of silently doing nothing.
pub fn load_toml_file(path: &Path) -> Result<Layer, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read config file {}: {}", path.display(), e))?;
    parse_toml(&raw).map_err(|e| format!("{}: {}", path.display(), e))
}

/// Parse TOML text into a layer. Split out from [`load_toml_file`] so it is testable without I/O.
pub fn parse_toml(raw: &str) -> Result<Layer, String> {
    let table: toml::Table = toml::from_str(raw).map_err(|e| e.message().to_string())?;
    let mut layer = Layer::new();

    for (section, value) in &table {
        let items = value.as_table().ok_or_else(|| {
            format!("`{section}` must be a section, for example `[{section}]` with keys under it")
        })?;
        for (key, item) in items {
            let path = format!("{section}.{key}");
            let param = by_toml(&path).ok_or_else(|| unknown_path_message(&path))?;
            layer.insert(param.env.to_string(), coerce(param, &path, item)?);
        }
    }

    Ok(layer)
}

/// Turn a TOML value into its canonical string form, enforcing the parameter's declared kind.
fn coerce(param: &Param, path: &str, item: &toml::Value) -> Result<String, String> {
    match param.kind {
        Kind::Num => item
            .as_integer()
            .map(|v| v.to_string())
            .ok_or_else(|| format!("`{path}` must be an integer")),
        Kind::Bool => item
            .as_bool()
            .map(|v| v.to_string())
            .ok_or_else(|| format!("`{path}` must be true or false")),
        Kind::Str => item
            .as_str()
            .map(|v| v.to_string())
            .ok_or_else(|| format!("`{path}` must be a string")),
        // Unreachable in practice: secrets have `toml: None`, so `by_toml` never returns one.
        Kind::SecretFile => Err(format!(
            "`{path}` cannot be set in a config file; use {} or the {} environment variable",
            param.flag, param.env
        )),
    }
}

/// Build an error for an unknown TOML path, suggesting the closest known key in that section.
fn unknown_path_message(path: &str) -> String {
    let section = path.split('.').next().unwrap_or("");
    let mut known: Vec<&str> = PARAMS
        .iter()
        .filter_map(|p| p.toml)
        .filter(|p| p.starts_with(&format!("{section}.")))
        .collect();
    known.sort_unstable();

    if known.is_empty() {
        format!("unknown config key `{path}`")
    } else {
        format!(
            "unknown config key `{path}` (known keys: {})",
            known.join(", ")
        )
    }
}

/// Render a value for display, redacting secrets.
pub fn redact(param: &Param, value: &str) -> String {
    if param.secret {
        format!("<redacted, {} bytes>", value.len())
    } else {
        value.to_string()
    }
}

/// Look up the canonical environment key for a `Config` field name.
///
/// Used by the admin API, whose request bodies are keyed by `Config` field name.
pub fn canonical_key(field: &str) -> Option<&'static str> {
    by_field(field).map(|p| p.env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::params::by_env;
    use std::collections::HashMap;
    use std::io::Write;

    /// Minimal `EnvProvider` that never reads real process environment variables.
    ///
    /// Tests must never call `std::env::set_var`: `cargo llvm-cov` runs the threaded
    /// `cargo test` harness rather than nextest's process-per-test, so global env mutation
    /// races with the tests in `src/telemetry.rs`.
    struct MockEnv(HashMap<String, String>);

    impl MockEnv {
        fn new() -> Self {
            MockEnv(HashMap::new())
        }
        fn with(mut self, k: &str, v: &str) -> Self {
            self.0.insert(k.to_string(), v.to_string());
            self
        }
    }

    impl EnvProvider for MockEnv {
        fn get(&self, key: &str) -> Result<String, env::VarError> {
            self.0.get(key).cloned().ok_or(env::VarError::NotPresent)
        }
    }

    fn layer(pairs: &[(&str, &str)]) -> Layer {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn temp_file(contents: &str, suffix: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(suffix)
            .tempfile()
            .expect("create temp file");
        f.write_all(contents.as_bytes()).expect("write temp file");
        f.flush().expect("flush temp file");
        f
    }

    // ── precedence ────────────────────────────────────────────────────────────

    #[test]
    fn test_cli_beats_every_other_layer() {
        let cli = layer(&[("SERVER_PORT", "1111")]);
        let env = MockEnv::new().with("SERVER_PORT", "2222");
        let env_file = layer(&[("SERVER_PORT", "3333")]);
        let file = layer(&[("SERVER_PORT", "4444")]);
        let carried = layer(&[("SERVER_PORT", "5555")]);
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert_eq!(l.get("SERVER_PORT").unwrap(), "1111");
        assert_eq!(l.source_of("SERVER_PORT"), Source::Cli);
    }

    #[test]
    fn test_env_beats_env_file_and_below() {
        let cli = Layer::new();
        let env = MockEnv::new().with("SERVER_PORT", "2222");
        let env_file = layer(&[("SERVER_PORT", "3333")]);
        let file = layer(&[("SERVER_PORT", "4444")]);
        let carried = Layer::new();
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert_eq!(l.get("SERVER_PORT").unwrap(), "2222");
        assert_eq!(l.source_of("SERVER_PORT"), Source::Env);
    }

    #[test]
    fn test_env_file_beats_toml_file() {
        let cli = Layer::new();
        let env = MockEnv::new();
        let env_file = layer(&[("SERVER_PORT", "3333")]);
        let file = layer(&[("SERVER_PORT", "4444")]);
        let carried = Layer::new();
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert_eq!(l.get("SERVER_PORT").unwrap(), "3333");
        assert_eq!(l.source_of("SERVER_PORT"), Source::EnvFile);
    }

    #[test]
    fn test_toml_file_beats_carried() {
        let cli = Layer::new();
        let env = MockEnv::new();
        let env_file = Layer::new();
        let file = layer(&[("SERVER_PORT", "4444")]);
        let carried = layer(&[("SERVER_PORT", "5555")]);
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert_eq!(l.get("SERVER_PORT").unwrap(), "4444");
        assert_eq!(l.source_of("SERVER_PORT"), Source::File);
    }

    #[test]
    fn test_carried_is_last_before_default() {
        let cli = Layer::new();
        let env = MockEnv::new();
        let env_file = Layer::new();
        let file = Layer::new();
        let carried = layer(&[("API_KEY_SALT", "carried-salt-value-16")]);
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert_eq!(l.get("API_KEY_SALT").unwrap(), "carried-salt-value-16");
        assert_eq!(l.source_of("API_KEY_SALT"), Source::Carried);
    }

    #[test]
    fn test_missing_everywhere_is_not_present() {
        let (cli, env_file, file, carried) =
            (Layer::new(), Layer::new(), Layer::new(), Layer::new());
        let env = MockEnv::new();
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert!(matches!(
            l.get("SERVER_PORT"),
            Err(env::VarError::NotPresent)
        ));
        assert_eq!(l.source_of("SERVER_PORT"), Source::Default);
    }

    #[test]
    fn test_layers_mix_per_key_independently() {
        let cli = layer(&[("SERVER_PORT", "1111")]);
        let env = MockEnv::new().with("DATABASE_URL", "sqlite::memory:");
        let env_file = layer(&[("CAPTCHA_COMPRESSION", "70")]);
        let file = layer(&[("MAX_VALIDATION_ATTEMPTS", "9")]);
        let carried = Layer::new();
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert_eq!(l.source_of("SERVER_PORT"), Source::Cli);
        assert_eq!(l.source_of("DATABASE_URL"), Source::Env);
        assert_eq!(l.source_of("CAPTCHA_COMPRESSION"), Source::EnvFile);
        assert_eq!(l.source_of("MAX_VALIDATION_ATTEMPTS"), Source::File);
        assert_eq!(l.source_of("SERVER_HOST"), Source::Default);
    }

    #[test]
    fn test_source_labels() {
        assert_eq!(Source::Cli.label(), "cli");
        assert_eq!(Source::Env.label(), "env");
        assert_eq!(Source::EnvFile.label(), "env-file");
        assert_eq!(Source::File.label(), "file");
        assert_eq!(Source::Carried.label(), "carried");
        assert_eq!(Source::Default.label(), "default");
    }

    // ── TOML parsing ──────────────────────────────────────────────────────────

    #[test]
    fn test_parse_toml_maps_sections_to_canonical_keys() {
        let l = parse_toml(
            r#"
            [server]
            host = "127.0.0.1"
            port = 8080

            [captcha]
            compression = 75

            [rate_limit]
            reverse_proxy = true
            "#,
        )
        .unwrap();

        assert_eq!(l.get("SERVER_HOST").unwrap(), "127.0.0.1");
        assert_eq!(l.get("SERVER_PORT").unwrap(), "8080");
        assert_eq!(l.get("CAPTCHA_COMPRESSION").unwrap(), "75");
        assert_eq!(l.get("RATE_LIMIT_REVERSE_PROXY").unwrap(), "true");
    }

    #[test]
    fn test_parse_toml_empty_is_ok() {
        assert!(parse_toml("").unwrap().is_empty());
    }

    #[test]
    fn test_parse_toml_rejects_unknown_key_and_suggests_known_ones() {
        let err = parse_toml("[server]\nprot = 8080\n").unwrap_err();
        assert!(err.contains("unknown config key `server.prot`"), "{err}");
        // The suggestion list makes the typo obvious.
        assert!(err.contains("server.port"), "{err}");
    }

    #[test]
    fn test_parse_toml_rejects_unknown_section() {
        let err = parse_toml("[nope]\nthing = 1\n").unwrap_err();
        assert!(err.contains("unknown config key `nope.thing`"), "{err}");
    }

    #[test]
    fn test_parse_toml_rejects_top_level_scalar() {
        let err = parse_toml("port = 8080\n").unwrap_err();
        assert!(err.contains("must be a section"), "{err}");
    }

    #[test]
    fn test_parse_toml_rejects_wrong_type_for_number() {
        let err = parse_toml("[server]\nport = \"8080\"\n").unwrap_err();
        assert!(err.contains("`server.port` must be an integer"), "{err}");
    }

    #[test]
    fn test_parse_toml_rejects_wrong_type_for_bool() {
        let err = parse_toml("[rate_limit]\nreverse_proxy = \"yes\"\n").unwrap_err();
        assert!(err.contains("must be true or false"), "{err}");
    }

    #[test]
    fn test_parse_toml_rejects_wrong_type_for_string() {
        let err = parse_toml("[server]\nhost = 5\n").unwrap_err();
        assert!(err.contains("`server.host` must be a string"), "{err}");
    }

    #[test]
    fn test_parse_toml_rejects_malformed_syntax() {
        assert!(parse_toml("[server\nport = 1").is_err());
    }

    #[test]
    fn test_parse_toml_cannot_set_secrets() {
        // Secrets have no TOML path at all, so they surface as unknown keys — a config file
        // is safe to commit by construction.
        let err = parse_toml("[security]\napi_key_salt = \"hunter2hunter2hunter2\"\n").unwrap_err();
        assert!(err.contains("unknown config key"), "{err}");
    }

    #[test]
    fn test_load_toml_file_reads_from_disk() {
        let f = temp_file("[server]\nport = 9090\n", ".toml");
        let l = load_toml_file(f.path()).unwrap();
        assert_eq!(l.get("SERVER_PORT").unwrap(), "9090");
    }

    #[test]
    fn test_load_toml_file_missing_names_the_path() {
        let err = load_toml_file(Path::new("/nonexistent/captchapi.toml")).unwrap_err();
        assert!(err.contains("cannot read config file"), "{err}");
        assert!(err.contains("/nonexistent/captchapi.toml"), "{err}");
    }

    #[test]
    fn test_load_toml_file_error_includes_path() {
        let f = temp_file("[server]\nprot = 1\n", ".toml");
        let err = load_toml_file(f.path()).unwrap_err();
        assert!(err.contains("unknown config key"), "{err}");
        assert!(err.contains(&f.path().display().to_string()), "{err}");
    }

    // ── env files ─────────────────────────────────────────────────────────────

    #[test]
    fn test_load_env_file_parses_pairs() {
        let f = temp_file(
            "SERVER_PORT=7000\n# a comment\nSERVER_HOST=1.2.3.4\n",
            ".env",
        );
        let l = load_env_file(f.path()).unwrap();
        assert_eq!(l.get("SERVER_PORT").unwrap(), "7000");
        assert_eq!(l.get("SERVER_HOST").unwrap(), "1.2.3.4");
    }

    #[test]
    fn test_load_env_file_keeps_unknown_keys() {
        // An env file may legitimately carry variables this service does not define.
        let f = temp_file("SOMETHING_ELSE=1\n", ".env");
        let l = load_env_file(f.path()).unwrap();
        assert_eq!(l.get("SOMETHING_ELSE").unwrap(), "1");
    }

    #[test]
    fn test_load_env_file_missing_names_the_path() {
        let err = load_env_file(Path::new("/nonexistent/.env")).unwrap_err();
        assert!(err.contains("cannot read env file"), "{err}");
    }

    #[test]
    fn test_load_env_file_does_not_touch_process_environment() {
        let f = temp_file("CAPTCHAPI_ENV_FILE_PROBE=set\n", ".env");
        let l = load_env_file(f.path()).unwrap();
        assert_eq!(l.get("CAPTCHAPI_ENV_FILE_PROBE").unwrap(), "set");
        assert!(env::var("CAPTCHAPI_ENV_FILE_PROBE").is_err());
    }

    // ── secret files ──────────────────────────────────────────────────────────

    #[test]
    fn test_read_secret_file_trims_trailing_newline() {
        let f = temp_file("super-secret-value-16\n", ".secret");
        assert_eq!(read_secret_file(f.path()).unwrap(), "super-secret-value-16");
    }

    #[test]
    fn test_read_secret_file_trims_surrounding_whitespace() {
        let f = temp_file("  super-secret-value-16  \n\n", ".secret");
        assert_eq!(read_secret_file(f.path()).unwrap(), "super-secret-value-16");
    }

    #[test]
    fn test_read_secret_file_rejects_empty() {
        let f = temp_file("   \n", ".secret");
        let err = read_secret_file(f.path()).unwrap_err();
        assert!(err.contains("is empty"), "{err}");
    }

    #[test]
    fn test_read_secret_file_missing_names_the_path() {
        let err = read_secret_file(Path::new("/nonexistent/salt")).unwrap_err();
        assert!(err.contains("cannot read secret file"), "{err}");
    }

    // ── redaction ─────────────────────────────────────────────────────────────

    #[test]
    fn test_redact_hides_secret_values() {
        let salt = by_env("API_KEY_SALT").unwrap();
        let out = redact(salt, "super-secret-value");
        assert!(!out.contains("super-secret-value"), "{out}");
        assert!(out.contains("18 bytes"), "{out}");
    }

    #[test]
    fn test_redact_passes_through_non_secrets() {
        let port = by_env("SERVER_PORT").unwrap();
        assert_eq!(redact(port, "8080"), "8080");
    }

    #[test]
    fn test_canonical_key_maps_field_names_to_env_keys() {
        assert_eq!(canonical_key("server_port"), Some("SERVER_PORT"));
        assert_eq!(
            canonical_key("captcha_compression"),
            Some("CAPTCHA_COMPRESSION")
        );
        assert_eq!(canonical_key("nope"), None);
    }

    #[test]
    fn test_every_toml_path_parses_back_to_its_param() {
        // Guards the table against a section/key typo that would silently orphan a parameter.
        for p in PARAMS.iter().filter(|p| p.toml.is_some()) {
            let path = p.toml.unwrap();
            let (section, key) = path.split_once('.').unwrap();
            let value = match p.kind {
                Kind::Num => "1".to_string(),
                Kind::Bool => "true".to_string(),
                Kind::Str => "\"x\"".to_string(),
                Kind::SecretFile => unreachable!("secrets have no toml path"),
            };
            let doc = format!("[{section}]\n{key} = {value}\n");
            let l = parse_toml(&doc).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert!(l.contains_key(p.env), "{path} did not map to {}", p.env);
        }
    }
}
