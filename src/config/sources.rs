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
    /// Set at runtime through the admin API. Ephemeral: cleared by the next reload.
    ///
    /// Distinct from [`Source::Cli`] even though it outranks it, because the two must not be
    /// confused: a value the admin API set is by definition one the admin API may set again,
    /// while a command-line value is pinned for the life of the process.
    Admin,
    /// A command-line flag.
    Cli,
    /// A process environment variable.
    Env,
    /// The `config_settings` table in the database.
    ///
    /// Below the command line and the environment, which are the recovery path: a stored value
    /// that misbehaves must be overridable with `captchapi --port 3000` without anyone having
    /// to open SQLite. Above the files, which are the deployment baseline that durable operator
    /// intent is meant to override.
    Stored,
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
            Source::Admin => "admin",
            Source::Cli => "cli",
            Source::Env => "env",
            Source::Stored => "stored",
            Source::EnvFile => "env-file",
            Source::File => "file",
            Source::Carried => "carried",
            Source::Default => "default",
        }
    }

    /// Whether this layer is fixed for the life of the process.
    ///
    /// The command line and the process environment are handed to the server at exec time and
    /// cannot change while it runs — not by editing a file, not by SIGHUP. Every other layer
    /// can be re-read. That difference is what makes the admin API refuse to override these
    /// two: the override would work, but only until the next reload discarded it, and there
    /// would be no way to make it durable short of restarting with different arguments. An
    /// override that cannot be made to stick is drift between the running server and the
    /// deployment that declared it, which is exactly what the reload path already refuses to
    /// create when it reports boot drift instead of applying it.
    pub fn is_pinned(&self) -> bool {
        matches!(self, Source::Cli | Source::Env)
    }
}

/// Which layer answered for each parameter, captured when the configuration was resolved.
///
/// Keyed by `Param::field`, matching what the admin API speaks.
#[derive(Debug, Clone, Default)]
pub struct Sources(BTreeMap<&'static str, Source>);

impl Sources {
    /// Record the layer that answers for every parameter in [`PARAMS`].
    pub fn capture<E: EnvProvider>(layered: &LayeredEnv<'_, E>) -> Self {
        Self(
            PARAMS
                .iter()
                .map(|param| (param.field, layered.source_of(param.env)))
                .collect(),
        )
    }

    /// The layer that supplied `field`, or [`Source::Default`] for anything unrecorded.
    ///
    /// Defaulting rather than returning an `Option` is deliberate: an unknown field is one no
    /// layer set, and the one caller that matters — the pinning check — must fail open to
    /// "editable" rather than locking a field it has no information about.
    pub fn get(&self, field: &str) -> Source {
        self.0.get(field).copied().unwrap_or(Source::Default)
    }

    /// Whether `field` came from a layer that cannot change while the process runs.
    pub fn is_pinned(&self, field: &str) -> bool {
        self.get(field).is_pinned()
    }
}

/// Empty layers, so `LayeredEnv::new` can leave the optional ones unset without every caller
/// having to invent them.
static NO_OVERLAY: Layer = Layer::new();
static NO_STORED: Layer = Layer::new();

/// The stack of configuration layers, in precedence order.
///
/// `overlay` (admin API) > `cli` > `env` (process) > `stored` (database) > `env_file` >
/// `file` (TOML) > `carried` > built-in default.
///
/// The env file sits *below* the process environment to preserve today's behaviour: the current
/// `dotenvy::dotenv()` call does not overwrite variables that are already set.
pub struct LayeredEnv<'a, E: EnvProvider> {
    /// Values set at runtime through the admin API. Outranks everything, because an operator
    /// changing a value through the API means it now — but it is tracked as its own layer
    /// rather than merged into `cli`, so [`source_of`](Self::source_of) can still tell the two
    /// apart. Merging them would make a field report itself as command-line-set the moment it
    /// was patched once, and so pin itself against ever being patched again.
    pub overlay: &'a Layer,
    pub cli: &'a Layer,
    pub env: &'a E,
    /// Values persisted in `config_settings`. Durable, unlike the overlay, and overridable by
    /// the two process-level layers above it, which is what keeps them a recovery path.
    pub stored: &'a Layer,
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
            overlay: &NO_OVERLAY,
            cli,
            env,
            stored: &NO_STORED,
            env_file,
            file,
            carried,
        }
    }

    /// Put the admin API's runtime overrides on top of the stack.
    pub fn with_overlay(mut self, overlay: &'a Layer) -> Self {
        self.overlay = overlay;
        self
    }

    /// Insert the persisted settings, below the process-level layers and above the files.
    pub fn with_stored(mut self, stored: &'a Layer) -> Self {
        self.stored = stored;
        self
    }

    /// Report which layer would answer `key`, probing in the same order as [`EnvProvider::get`].
    ///
    /// This is a separate pure query rather than something recorded during `get`, because
    /// recording would need interior mutability, and a `RefCell` here would make `LayeredEnv`
    /// `!Sync` — which breaks the moment a reload runs inside an async task.
    pub fn source_of(&self, key: &str) -> Source {
        if self.overlay.contains_key(key) {
            Source::Admin
        } else if self.cli.contains_key(key) {
            Source::Cli
        } else if self.env.get(key).is_ok() {
            Source::Env
        } else if self.stored.contains_key(key) {
            Source::Stored
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
        self.overlay
            .get(key)
            .cloned()
            .or_else(|| self.cli.get(key).cloned())
            .or_else(|| self.env.get(key).ok())
            .or_else(|| self.stored.get(key).cloned())
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
        let (key, value) = item.map_err(|e| describe_env_file_error(path, e))?;
        layer.insert(key, value);
    }
    Ok(layer)
}

/// Describe an env-file parse failure **without quoting the offending line**.
///
/// `dotenvy::Error::LineParse` carries the raw source line, which for an env file is
/// `KEY=VALUE` — so rendering it with `Display` would put a secret into the boot log, the
/// reload log, and the body of `POST /api/v1/admin/config/reload`. Only the line index is
/// safe to report. `Io` and `EnvVar` carry no file content and are rendered normally.
fn describe_env_file_error(path: &Path, err: dotenvy::Error) -> String {
    match err {
        dotenvy::Error::LineParse(_, index) => format!(
            "invalid line in env file {} at index {} (line content withheld: it may contain a secret)",
            path.display(),
            index
        ),
        dotenvy::Error::Io(e) => format!("cannot read env file {}: {}", path.display(), e),
        dotenvy::Error::EnvVar(e) => format!("invalid value in env file {}: {}", path.display(), e),
        // `dotenvy::Error` is #[non_exhaustive]; a future variant might carry file content, so
        // the fallback must stay content-free.
        _ => format!("invalid env file {}", path.display()),
    }
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
        assert_eq!(Source::Admin.label(), "admin");
        assert_eq!(Source::Cli.label(), "cli");
        assert_eq!(Source::Env.label(), "env");
        assert_eq!(Source::EnvFile.label(), "env-file");
        assert_eq!(Source::File.label(), "file");
        assert_eq!(Source::Carried.label(), "carried");
        assert_eq!(Source::Default.label(), "default");
    }

    /// Exactly two layers are fixed for the life of the process, and the whole pinning rule
    /// rests on that list being right — so it is spelled out rather than spot-checked.
    #[test]
    fn test_only_the_process_level_layers_are_pinned() {
        assert!(Source::Cli.is_pinned());
        assert!(Source::Env.is_pinned());

        // Files can be edited and re-read, so an override there is something a reload can be
        // made to agree with.
        assert!(!Source::EnvFile.is_pinned());
        assert!(!Source::File.is_pinned());
        assert!(!Source::Default.is_pinned());
        assert!(!Source::Carried.is_pinned());
        // And a value this API set is by definition one it may set again.
        assert!(!Source::Admin.is_pinned());
    }

    #[test]
    fn test_the_overlay_outranks_the_command_line() {
        let overlay = layer(&[("CAPTCHA_COMPRESSION", "90")]);
        let cli = layer(&[("CAPTCHA_COMPRESSION", "70")]);
        let (env, env_file, file, carried) =
            (MockEnv::new(), Layer::new(), Layer::new(), Layer::new());

        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried).with_overlay(&overlay);

        assert_eq!(l.get("CAPTCHA_COMPRESSION").unwrap(), "90");
        assert_eq!(l.source_of("CAPTCHA_COMPRESSION"), Source::Admin);
    }

    #[test]
    fn test_without_an_overlay_the_command_line_still_answers() {
        let cli = layer(&[("CAPTCHA_COMPRESSION", "70")]);
        let (env, env_file, file, carried) =
            (MockEnv::new(), Layer::new(), Layer::new(), Layer::new());

        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        assert_eq!(l.get("CAPTCHA_COMPRESSION").unwrap(), "70");
        assert_eq!(l.source_of("CAPTCHA_COMPRESSION"), Source::Cli);
    }

    /// The whole stack, top to bottom, in one assertion.
    ///
    /// Precedence is the thing most likely to be broken by a careless edit to `get`, and the
    /// per-pair tests below each cover only one boundary. This covers every boundary at once by
    /// removing layers from the top and watching the next one take over.
    #[test]
    fn test_every_layer_yields_to_the_one_above_it() {
        let key = "CAPTCHA_COMPRESSION";
        let overlay = layer(&[(key, "1")]);
        let cli = layer(&[(key, "2")]);
        let env = MockEnv::new().with(key, "3");
        let stored = layer(&[(key, "4")]);
        let env_file = layer(&[(key, "5")]);
        let file = layer(&[(key, "6")]);
        let carried = layer(&[(key, "7")]);

        let full = LayeredEnv::new(&cli, &env, &env_file, &file, &carried)
            .with_overlay(&overlay)
            .with_stored(&stored);
        assert_eq!(full.get(key).unwrap(), "1");
        assert_eq!(full.source_of(key), Source::Admin);

        let no_overlay =
            LayeredEnv::new(&cli, &env, &env_file, &file, &carried).with_stored(&stored);
        assert_eq!(no_overlay.get(key).unwrap(), "2");
        assert_eq!(no_overlay.source_of(key), Source::Cli);

        let empty = Layer::new();
        let no_cli = LayeredEnv::new(&empty, &env, &env_file, &file, &carried).with_stored(&stored);
        assert_eq!(no_cli.get(key).unwrap(), "3");
        assert_eq!(no_cli.source_of(key), Source::Env);

        let none = MockEnv::new();
        let no_env =
            LayeredEnv::new(&empty, &none, &env_file, &file, &carried).with_stored(&stored);
        assert_eq!(no_env.get(key).unwrap(), "4", "stored answers below env");
        assert_eq!(no_env.source_of(key), Source::Stored);

        let no_stored = LayeredEnv::new(&empty, &none, &env_file, &file, &carried);
        assert_eq!(
            no_stored.get(key).unwrap(),
            "5",
            "env-file answers below stored"
        );
        assert_eq!(no_stored.source_of(key), Source::EnvFile);

        let no_env_file = LayeredEnv::new(&empty, &none, &empty, &file, &carried);
        assert_eq!(no_env_file.get(key).unwrap(), "6");
        assert_eq!(no_env_file.source_of(key), Source::File);

        let only_carried = LayeredEnv::new(&empty, &none, &empty, &empty, &carried);
        assert_eq!(only_carried.get(key).unwrap(), "7");
        assert_eq!(only_carried.source_of(key), Source::Carried);

        let nothing = LayeredEnv::new(&empty, &none, &empty, &empty, &empty);
        assert!(nothing.get(key).is_err());
        assert_eq!(nothing.source_of(key), Source::Default);
    }

    /// The two boundaries that define what "stored" means, stated on their own.
    ///
    /// Above the files, so durable operator intent overrides the deployment baseline. Below the
    /// environment, so `SERVER_PORT=3000 captchapi` remains a way back from a stored value that
    /// prevents the service from working.
    #[test]
    fn test_stored_beats_files_but_yields_to_the_environment() {
        let key = "SERVER_PORT";
        let (empty, stored) = (Layer::new(), layer(&[(key, "8080")]));
        let file = layer(&[(key, "9090")]);

        let none = MockEnv::new();
        let no_env = LayeredEnv::new(&empty, &none, &empty, &file, &empty).with_stored(&stored);
        assert_eq!(no_env.get(key).unwrap(), "8080");

        let env = MockEnv::new().with(key, "3000");
        let with_env = LayeredEnv::new(&empty, &env, &empty, &file, &empty).with_stored(&stored);
        assert_eq!(
            with_env.get(key).unwrap(),
            "3000",
            "the recovery path holds"
        );
        assert_eq!(with_env.source_of(key), Source::Env);
    }

    #[test]
    fn test_a_stored_value_is_not_pinned() {
        // The point of the layer: unlike cli and env, a stored value is one the admin API may
        // change. If `Stored` were ever pinned, storing a value would lock it.
        assert!(!Source::Stored.is_pinned());

        let stored = layer(&[("CAPTCHA_COMPRESSION", "70")]);
        let empty = Layer::new();
        let none = MockEnv::new();
        let l = LayeredEnv::new(&empty, &none, &empty, &empty, &empty).with_stored(&stored);

        assert!(!Sources::capture(&l).is_pinned("captcha_compression"));
    }

    #[test]
    fn test_capture_records_a_layer_for_every_parameter() {
        let cli = layer(&[("SERVER_PORT", "9999")]);
        let (env, env_file, file, carried) =
            (MockEnv::new(), Layer::new(), Layer::new(), Layer::new());
        let l = LayeredEnv::new(&cli, &env, &env_file, &file, &carried);

        let sources = Sources::capture(&l);

        assert_eq!(sources.get("server_port"), Source::Cli);
        assert!(sources.is_pinned("server_port"));
        assert_eq!(sources.get("captcha_compression"), Source::Default);
        assert!(!sources.is_pinned("captcha_compression"));
        // Fails open: a field no layer mentioned must not be treated as pinned.
        assert_eq!(sources.get("nonexistent_field"), Source::Default);
        assert!(!sources.is_pinned("nonexistent_field"));
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
    fn test_malformed_env_file_never_quotes_the_offending_line() {
        // dotenvy's LineParse error Displays the whole raw `KEY=VALUE` line. Rendering it
        // would put the secret into the boot log, the SIGHUP reload log, and the body of
        // POST /admin/config/reload. Only the index may be reported.
        let sentinel = "s3cr3t-that-must-never-be-logged";
        let f = temp_file(
            &format!("MASTER_API_KEY={sentinel}\"unterminated\n"),
            ".env",
        );

        let err = load_env_file(f.path()).unwrap_err();

        assert!(
            !err.contains(sentinel),
            "env file parse error leaked a secret: {err}"
        );
        assert!(!err.contains("MASTER_API_KEY="), "{err}");
        // It still has to be actionable.
        assert!(err.contains("invalid line in env file"), "{err}");
        assert!(err.contains(&f.path().display().to_string()), "{err}");
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
