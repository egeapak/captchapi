//! Command-line surface.
//!
//! Parsing is a pure function over an argument vector so every branch is unit-testable without
//! touching the process environment or `std::env::args`. `main.rs` stays a thin wrapper that
//! calls [`parse`], then [`handle`].
//!
//! The CLI is deliberately *not* a second configuration system: each flag resolves to the same
//! canonical key as its environment variable (the `env` column of [`PARAMS`]), and the parsed
//! flags become the highest-precedence [`Layer`] in a [`LayeredEnv`]. All parsing, validation
//! and error messages continue to come from [`Config::from_env_provider`].

use crate::config::params::{Kind, Reload, PARAMS};
use crate::config::sources::{
    load_env_file, load_toml_file, read_secret_file, redact, Layer, LayeredEnv, Sources,
};
use crate::config::{Config, EnvProvider, RealEnv};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Exit code for a usage or configuration error, following the common CLI convention.
pub const EXIT_USAGE: i32 = 2;

/// The flags shared by every command that resolves configuration.
///
/// `Debug` is hand-written: `values` holds the *contents* of any secret file named on the
/// command line, so a derived impl would print the salt and master key in the clear.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Cli {
    /// Values supplied on the command line, keyed by canonical environment key.
    pub values: Layer,
    /// `--config <PATH>`
    pub config_file: Option<PathBuf>,
    /// `--env-file <PATH>`
    pub env_file: Option<PathBuf>,
    /// `--no-env-file`
    pub no_env_file: bool,
}

impl std::fmt::Debug for Cli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let values: Vec<(&str, String)> = self
            .values
            .iter()
            .map(|(key, value)| {
                let rendered = match crate::config::params::by_env(key) {
                    Some(param) => redact(param, value),
                    None => value.clone(),
                };
                (key.as_str(), rendered)
            })
            .collect();
        f.debug_struct("Cli")
            .field("values", &values)
            .field("config_file", &self.config_file)
            .field("env_file", &self.env_file)
            .field("no_env_file", &self.no_env_file)
            .finish()
    }
}

/// Where `captchapi reload` should send its signal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReloadTarget {
    pub pid: Option<i32>,
    pub pid_file: Option<PathBuf>,
    /// Config/env files to consult for `PID_FILE`, so a server started with `-c prod.toml`
    /// can be found by `captchapi reload -c prod.toml`.
    pub config_file: Option<PathBuf>,
    pub env_file: Option<PathBuf>,
    pub no_env_file: bool,
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Help,
    Version,
    /// Start the server. The default when no verb is given, which is what keeps a bare
    /// `cargo run` (and the CI api-tests job) working unchanged.
    Run(Box<Cli>),
    /// Print the effective configuration with provenance, then exit.
    ConfigShow(Box<Cli>),
    /// Validate the configuration and exit.
    ConfigCheck(Box<Cli>),
    /// Signal a running server to reload.
    Reload(ReloadTarget),
}

/// Parse an argument vector (excluding the binary name).
pub fn parse(args: Vec<OsString>) -> Result<Action, String> {
    let mut pargs = pico_args::Arguments::from_vec(args);

    if pargs.contains(["-h", "--help"]) {
        return Ok(Action::Help);
    }
    if pargs.contains(["-V", "--version"]) {
        return Ok(Action::Version);
    }

    let verb = pargs.subcommand().map_err(describe)?;
    match verb.as_deref() {
        None | Some("run") => Ok(Action::Run(Box::new(parse_shared(pargs)?))),
        Some("config") => {
            let sub = pargs.subcommand().map_err(describe)?;
            match sub.as_deref() {
                Some("show") => Ok(Action::ConfigShow(Box::new(parse_shared(pargs)?))),
                Some("check") => Ok(Action::ConfigCheck(Box::new(parse_shared(pargs)?))),
                Some(other) => Err(format!(
                    "unknown subcommand `config {other}` (expected `show` or `check`)"
                )),
                None => Err("`config` needs a subcommand: `show` or `check`".to_string()),
            }
        }
        Some("reload") => {
            let pid = pargs.opt_value_from_str("--pid").map_err(describe)?;
            let pid_file = pargs
                .opt_value_from_str::<_, String>("--pid-file")
                .map_err(describe)?
                .map(PathBuf::from);
            // The server's PID file location can come from a config or env file, so `reload`
            // has to be able to read the same sources in order to find it.
            let config_file = pargs
                .opt_value_from_str::<_, String>(["-c", "--config"])
                .map_err(describe)?
                .map(PathBuf::from);
            let env_file = pargs
                .opt_value_from_str::<_, String>("--env-file")
                .map_err(describe)?
                .map(PathBuf::from);
            let no_env_file = pargs.contains("--no-env-file");
            finish(pargs)?;

            if pid.is_some() && pid_file.is_some() {
                return Err(
                    "--pid and --pid-file are mutually exclusive; pass only one".to_string()
                );
            }

            Ok(Action::Reload(ReloadTarget {
                pid,
                pid_file,
                config_file,
                env_file,
                no_env_file,
            }))
        }
        Some(other) => Err(format!(
            "unknown command `{other}` (expected `run`, `reload`, or `config`)"
        )),
    }
}

/// Parse the configuration flags common to `run`, `config show` and `config check`.
fn parse_shared(mut pargs: pico_args::Arguments) -> Result<Cli, String> {
    let mut cli = Cli {
        config_file: pargs
            .opt_value_from_str::<_, String>(["-c", "--config"])
            .map_err(describe)?
            .map(PathBuf::from),
        env_file: pargs
            .opt_value_from_str::<_, String>("--env-file")
            .map_err(describe)?
            .map(PathBuf::from),
        no_env_file: pargs.contains("--no-env-file"),
        values: Layer::new(),
    };

    for param in PARAMS {
        match param.kind {
            // Booleans accept both forms: bare `--flag` means true, and `--flag=false` turns
            // off a setting that defaults to true.
            //
            // `contains` is checked first because it matches the bare flag exactly and removes
            // it. Trying the value form first would make `--flag --port 8080` consume `--port`
            // as the flag's value; and because `--flag=false` is not equal to `--flag`,
            // `contains` leaves the `=` form alone for `opt_value_from_str` below.
            Kind::Bool => {
                if pargs.contains(param.flag) {
                    cli.values.insert(param.env.to_string(), "true".to_string());
                } else if let Some(raw) = pargs
                    .opt_value_from_str::<_, String>(param.flag)
                    .map_err(describe)?
                {
                    let value = crate::config::parse_bool_lenient(&raw).ok_or_else(|| {
                        format!("{} expects true or false, not `{raw}`", param.flag)
                    })?;
                    cli.values.insert(param.env.to_string(), value.to_string());
                }
            }
            // The flag names a file; its trimmed contents become the value, so secrets stay
            // out of `ps aux`, shell history, and `docker inspect`.
            Kind::SecretFile => {
                if let Some(path) = opt_value(&mut pargs, param.flag, param.short)? {
                    let value = read_secret_file(Path::new(&path))?;
                    cli.values.insert(param.env.to_string(), value);
                }
            }
            Kind::Str | Kind::Num => {
                if let Some(value) = opt_value(&mut pargs, param.flag, param.short)? {
                    cli.values.insert(param.env.to_string(), value);
                }
            }
        }
    }

    finish(pargs)?;
    Ok(cli)
}

/// Read an optional value, honouring the parameter's short flag when it has one.
fn opt_value(
    pargs: &mut pico_args::Arguments,
    flag: &'static str,
    short: Option<&'static str>,
) -> Result<Option<String>, String> {
    match short {
        Some(short) => pargs.opt_value_from_str([short, flag]),
        None => pargs.opt_value_from_str(flag),
    }
    .map_err(describe)
}

/// Reject anything left over.
///
/// `pico-args` does not report unknown flags on its own — it simply leaves them behind — so
/// this call is the only thing standing between a typo and a silently ignored setting.
fn finish(pargs: pico_args::Arguments) -> Result<(), String> {
    let leftover = pargs.finish();
    if leftover.is_empty() {
        return Ok(());
    }
    let rendered: Vec<String> = leftover
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    Err(format!("unexpected argument `{}`", rendered.join("` `")))
}

/// Turn a `pico-args` error into a message that names what the user should fix.
fn describe(err: pico_args::Error) -> String {
    match err {
        pico_args::Error::MissingOption(key) => format!("missing required option {key:?}"),
        pico_args::Error::OptionWithoutAValue(key) => format!("{key} needs a value"),
        pico_args::Error::Utf8ArgumentParsingFailed { value, .. } => {
            format!("`{value}` is not valid UTF-8")
        }
        pico_args::Error::ArgumentParsingFailed { cause } => cause,
        other => other.to_string(),
    }
}

/// The four explicit layers, owned so a [`LayeredEnv`] can borrow them.
///
/// `Debug` is hand-written for the same reason as [`Cli`]'s: these maps hold resolved secret
/// values (`cli` from a `--*-file` flag, `carried` from `Config::boot_layer`).
#[derive(Default)]
pub struct Layers {
    /// Runtime overrides from the admin API. Above `cli`, but tracked separately so a patched
    /// field does not report itself as command-line-set and pin itself against further patches.
    pub overlay: Layer,
    pub cli: Layer,
    pub env_file: Layer,
    pub file: Layer,
    /// Boot-only values carried over from a running config during a reload.
    pub carried: Layer,
}

/// Render a layer with secret values redacted.
fn redact_layer(layer: &Layer) -> Vec<(&str, String)> {
    layer
        .iter()
        .map(|(key, value)| {
            let rendered = match crate::config::params::by_env(key) {
                Some(param) => redact(param, value),
                None => value.clone(),
            };
            (key.as_str(), rendered)
        })
        .collect()
}

impl std::fmt::Debug for Layers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layers")
            .field("cli", &redact_layer(&self.cli))
            .field("env_file", &redact_layer(&self.env_file))
            .field("file", &redact_layer(&self.file))
            .field("carried", &redact_layer(&self.carried))
            .finish()
    }
}

impl Layers {
    /// Borrow the layers as a resolution stack over `env`.
    pub fn stack<'a, E: EnvProvider>(&'a self, env: &'a E) -> LayeredEnv<'a, E> {
        LayeredEnv::new(&self.cli, env, &self.env_file, &self.file, &self.carried)
            .with_overlay(&self.overlay)
    }
}

/// Load every layer named by the CLI.
///
/// An explicit `--env-file` that does not exist is an error; the implicit `.env` is skipped
/// when absent, matching the behaviour of the `dotenvy::dotenv().ok()` call this replaces.
pub fn load_layers(cli: &Cli) -> Result<Layers, String> {
    let mut layers = Layers {
        cli: cli.values.clone(),
        ..Default::default()
    };

    if !cli.no_env_file {
        match &cli.env_file {
            Some(path) => layers.env_file = load_env_file(path)?,
            None => {
                let default = Path::new(".env");
                if default.exists() {
                    layers.env_file = load_env_file(default)?;
                }
            }
        }
    }

    if let Some(path) = &cli.config_file {
        layers.file = load_toml_file(path)?;
    }

    Ok(layers)
}

/// Resolve the effective configuration from the CLI, the process environment, and any files.
pub fn resolve(cli: &Cli) -> Result<(Config, Layers), String> {
    let layers = load_layers(cli)?;
    let config = Config::from_env_provider(&layers.stack(&RealEnv))?;
    Ok((config, layers))
}

/// Render `--help`, generated from [`PARAMS`] so a new parameter documents itself.
pub fn help() -> String {
    let mut out = String::new();
    out.push_str("captchapi — CAPTCHA challenge API\n\n");
    out.push_str("USAGE:\n");
    out.push_str("    captchapi [OPTIONS]                 Start the server\n");
    out.push_str("    captchapi run [OPTIONS]             Start the server (explicit)\n");
    out.push_str("    captchapi config show [OPTIONS]     Print the effective configuration\n");
    out.push_str("    captchapi config check [OPTIONS]    Validate the configuration and exit\n");
    out.push_str("    captchapi reload [--pid N | --pid-file PATH]\n");
    out.push_str("                                        Tell a running server to reload\n\n");

    out.push_str("GENERAL:\n");
    out.push_str("    -h, --help                          Print this help\n");
    out.push_str("    -V, --version                       Print the version\n");
    out.push_str("    -c, --config <PATH>                 TOML configuration file\n");
    out.push_str("        --env-file <PATH>               Env file to load (default: .env)\n");
    out.push_str("        --no-env-file                   Do not load any env file\n\n");

    for (title, reload) in [
        ("OPTIONS (applied at startup):", Reload::Boot),
        ("OPTIONS (re-read on reload):", Reload::Live),
    ] {
        out.push_str(title);
        out.push('\n');
        for param in PARAMS.iter().filter(|p| p.reload == reload) {
            let flag = match param.short {
                Some(short) => format!("{short}, {}", param.flag),
                None => format!("    {}", param.flag),
            };
            let placeholder = match param.kind {
                Kind::Bool => String::new(),
                Kind::SecretFile => " <PATH>".to_string(),
                Kind::Num => " <N>".to_string(),
                Kind::Str => " <VALUE>".to_string(),
            };
            out.push_str(&format!(
                "    {:<38}{}\n",
                format!("{flag}{placeholder}"),
                param.help
            ));
            let default = match param.default {
                Some(d) => format!("default: {d}"),
                None => "required".to_string(),
            };
            out.push_str(&format!(
                "    {:<38}[{}; env: {}]\n",
                "", default, param.env
            ));
        }
        out.push('\n');
    }

    out.push_str("NOTES:\n");
    out.push_str(
        "    Precedence: command line > environment > env file > config file > default.\n",
    );
    out.push_str("    A value set through PATCH /api/v1/admin/config outranks all of these,\n");
    out.push_str("    until the next reload clears it.\n");
    out.push_str("    Booleans take a bare flag for true, or an explicit --flag=false.\n");
    out.push_str("    Secrets are read from files only, never from flag values, so they stay\n");
    out.push_str("    out of `ps`, shell history, and `docker inspect`. They cannot be set in\n");
    out.push_str("    the TOML config file at all, which keeps a committed file safe.\n");
    out
}

/// Render the `--version` line.
pub fn version() -> String {
    format!("captchapi {}", env!("CARGO_PKG_VERSION"))
}

/// Render `config show`: every parameter with its effective value and the layer it came from.
pub fn render_config<E: EnvProvider>(config: &Config, layered: &LayeredEnv<'_, E>) -> String {
    let width = PARAMS.iter().map(|p| p.field.len()).max().unwrap_or(0);
    let mut out = String::new();
    for param in PARAMS {
        let value = config
            .field_value(param.field)
            .unwrap_or_else(|| "<unset>".to_string());
        out.push_str(&format!(
            "{:<width$} = {:<34} [{}]\n",
            param.field,
            redact(param, &value),
            layered.source_of(param.env).label(),
            width = width
        ));
    }
    out
}

/// Write the current process ID so `captchapi reload` can find this server.
pub fn write_pid_file(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
        }
    }
    std::fs::write(path, format!("{}\n", std::process::id()))
        .map_err(|e| format!("cannot write pid file {}: {}", path.display(), e))
}

/// Remove the PID file, ignoring failure.
///
/// Best-effort on shutdown: a stale file would make `captchapi reload` signal a recycled,
/// unrelated process.
pub fn remove_pid_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Read a process ID from a PID file.
pub fn read_pid_file(path: &Path) -> Result<i32, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "cannot read pid file {}: {} (is the server running? pass --pid to override)",
            path.display(),
            e
        )
    })?;
    raw.trim()
        .parse::<i32>()
        .map_err(|_| format!("pid file {} does not contain a process id", path.display()))
}

/// Resolve which PID file `captchapi reload` should consult.
///
/// Deliberately lighter than a full [`Config`] resolution: `reload` must work without the
/// secrets that `Config` requires. It still walks the same layers in the same order, so a
/// `PID_FILE` set in a config or env file is honoured.
pub fn reload_pid_file<E: EnvProvider>(target: &ReloadTarget, env: &E) -> PathBuf {
    if let Some(path) = &target.pid_file {
        return path.clone();
    }
    if let Ok(value) = env.get("PID_FILE") {
        return PathBuf::from(value);
    }

    let layers = load_layers(&Cli {
        config_file: target.config_file.clone(),
        env_file: target.env_file.clone(),
        no_env_file: target.no_env_file,
        values: Layer::new(),
    })
    .unwrap_or_default();

    for layer in [&layers.env_file, &layers.file] {
        if let Some(value) = layer.get("PID_FILE") {
            return PathBuf::from(value);
        }
    }

    PathBuf::from(
        crate::config::params::by_env("PID_FILE")
            .and_then(|p| p.default)
            .unwrap_or("./data/captchapi.pid"),
    )
}

/// Send the reload signal to a running server.
#[cfg(unix)]
pub fn send_reload_signal(pid: i32) -> Result<(), String> {
    // SAFETY: `kill` is async-signal-safe and takes no pointers. An invalid or dead pid is
    // reported through errno rather than being undefined behaviour.
    let rc = unsafe { libc::kill(pid, libc::SIGHUP) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!(
            "cannot signal process {pid}: {}",
            std::io::Error::last_os_error()
        ))
    }
}

/// Sending signals is not supported on this platform.
#[cfg(not(unix))]
pub fn send_reload_signal(_pid: i32) -> Result<(), String> {
    Err("`captchapi reload` requires a Unix platform; use POST /api/v1/admin/config/reload".into())
}

/// Everything a command can do short of running the server.
#[derive(Debug)]
pub enum Handled {
    /// The command completed; the process should exit successfully.
    Done,
    /// The server should start with this configuration, resolved from these layers.
    Serve(Box<Cli>, Box<Config>, Box<Sources>),
}

/// Execute an [`Action`], returning what `main` should do next.
///
/// Kept here rather than in `main.rs` so that the logic is covered by tests: `main.rs` is
/// effectively invisible to the coverage gate.
pub fn handle(action: Action) -> Result<Handled, String> {
    match action {
        Action::Help => {
            print!("{}", help());
            Ok(Handled::Done)
        }
        Action::Version => {
            println!("{}", version());
            Ok(Handled::Done)
        }
        Action::Run(cli) => {
            let (config, layers) = resolve(&cli)?;
            let sources = Sources::capture(&layers.stack(&RealEnv));
            Ok(Handled::Serve(cli, Box::new(config), Box::new(sources)))
        }
        Action::ConfigCheck(cli) => {
            resolve(&cli)?;
            println!("configuration is valid");
            Ok(Handled::Done)
        }
        Action::ConfigShow(cli) => {
            let (config, layers) = resolve(&cli)?;
            print!("{}", render_config(&config, &layers.stack(&RealEnv)));
            Ok(Handled::Done)
        }
        Action::Reload(target) => {
            let pid = match target.pid {
                Some(pid) => pid,
                None => read_pid_file(&reload_pid_file(&target, &RealEnv))?,
            };
            send_reload_signal(pid)?;
            println!("reload signalled (pid {pid})");
            Ok(Handled::Done)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn run_cli(list: &[&str]) -> Cli {
        match parse(args(list)).unwrap() {
            Action::Run(cli) => *cli,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    fn temp_file(contents: &str, suffix: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(suffix)
            .tempfile()
            .expect("create temp file");
        f.write_all(contents.as_bytes()).expect("write");
        f.flush().expect("flush");
        f
    }

    struct MockEnv(HashMap<String, String>);

    impl EnvProvider for MockEnv {
        fn get(&self, key: &str) -> Result<String, std::env::VarError> {
            self.0
                .get(key)
                .cloned()
                .ok_or(std::env::VarError::NotPresent)
        }
    }

    // ── verbs ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_no_arguments_runs_the_server() {
        // This is what keeps a bare `cargo run` — and the CI api-tests job — working.
        assert_eq!(run_cli(&[]), Cli::default());
    }

    #[test]
    fn test_explicit_run_verb() {
        assert_eq!(run_cli(&["run"]), Cli::default());
    }

    #[test]
    fn test_help_and_version_flags() {
        assert_eq!(parse(args(&["--help"])).unwrap(), Action::Help);
        assert_eq!(parse(args(&["-h"])).unwrap(), Action::Help);
        assert_eq!(parse(args(&["--version"])).unwrap(), Action::Version);
        assert_eq!(parse(args(&["-V"])).unwrap(), Action::Version);
    }

    #[test]
    fn test_help_wins_over_a_verb() {
        assert_eq!(
            parse(args(&["config", "show", "--help"])).unwrap(),
            Action::Help
        );
    }

    #[test]
    fn test_config_subcommands() {
        assert!(matches!(
            parse(args(&["config", "show"])).unwrap(),
            Action::ConfigShow(_)
        ));
        assert!(matches!(
            parse(args(&["config", "check"])).unwrap(),
            Action::ConfigCheck(_)
        ));
    }

    #[test]
    fn test_config_without_subcommand_is_an_error() {
        let err = parse(args(&["config"])).unwrap_err();
        assert!(err.contains("needs a subcommand"), "{err}");
    }

    #[test]
    fn test_unknown_config_subcommand_is_an_error() {
        let err = parse(args(&["config", "dance"])).unwrap_err();
        assert!(err.contains("config dance"), "{err}");
    }

    #[test]
    fn test_unknown_verb_is_an_error() {
        let err = parse(args(&["frobnicate"])).unwrap_err();
        assert!(err.contains("unknown command `frobnicate`"), "{err}");
    }

    #[test]
    fn test_reload_rejects_pid_and_pid_file_together() {
        // Silently preferring one would drop the other without a word.
        let err = parse(args(&["reload", "--pid", "42", "--pid-file", "/run/x.pid"])).unwrap_err();
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn test_reload_finds_the_pid_file_named_in_a_config_file() {
        // A server started with `-c prod.toml` writes its PID where that file says; `reload`
        // has to read the same file or it will look in the wrong place.
        let toml = temp_file("[server]\npid_file = \"/run/from-toml.pid\"\n", ".toml");
        let action = parse(args(&[
            "reload",
            "--no-env-file",
            "-c",
            toml.path().to_str().unwrap(),
        ]))
        .unwrap();

        let Action::Reload(target) = action else {
            panic!("expected Reload")
        };
        let empty = MockEnv(HashMap::new());
        assert_eq!(
            reload_pid_file(&target, &empty),
            PathBuf::from("/run/from-toml.pid")
        );
    }

    #[test]
    fn test_reload_verb_accepts_pid_and_pid_file() {
        assert_eq!(
            parse(args(&["reload", "--pid", "42"])).unwrap(),
            Action::Reload(ReloadTarget {
                pid: Some(42),
                ..ReloadTarget::default()
            })
        );
        assert_eq!(
            parse(args(&["reload", "--pid-file", "/run/x.pid"])).unwrap(),
            Action::Reload(ReloadTarget {
                pid_file: Some(PathBuf::from("/run/x.pid")),
                ..ReloadTarget::default()
            })
        );
        assert_eq!(
            parse(args(&["reload"])).unwrap(),
            Action::Reload(ReloadTarget::default())
        );
    }

    // ── flags ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_long_and_short_flags_map_to_canonical_keys() {
        let cli = run_cli(&["--port", "8080"]);
        assert_eq!(cli.values.get("SERVER_PORT").unwrap(), "8080");

        let cli = run_cli(&["-p", "9090"]);
        assert_eq!(cli.values.get("SERVER_PORT").unwrap(), "9090");
    }

    #[test]
    fn test_equals_form_is_accepted() {
        let cli = run_cli(&["--port=8080"]);
        assert_eq!(cli.values.get("SERVER_PORT").unwrap(), "8080");
    }

    #[test]
    fn test_every_non_secret_parameter_has_a_working_flag() {
        // Exercises the table-driven loop for every row at once, so a malformed entry cannot
        // slip through with only the hand-written cases below for cover.
        for param in PARAMS.iter().filter(|p| p.kind != Kind::SecretFile) {
            let argv: Vec<&str> = match param.kind {
                Kind::Bool => vec![param.flag],
                _ => vec![param.flag, "7"],
            };
            let cli = run_cli(&argv);
            let expected = if param.kind == Kind::Bool {
                "true"
            } else {
                "7"
            };
            assert_eq!(
                cli.values.get(param.env).map(String::as_str),
                Some(expected),
                "flag {} did not populate {}",
                param.flag,
                param.env
            );
        }
    }

    #[test]
    fn test_bare_boolean_flag_sets_true() {
        let cli = run_cli(&["--rate-limit-reverse-proxy"]);
        assert_eq!(cli.values.get("RATE_LIMIT_REVERSE_PROXY").unwrap(), "true");

        let cli = run_cli(&[]);
        assert!(!cli.values.contains_key("RATE_LIMIT_REVERSE_PROXY"));
    }

    #[test]
    fn test_boolean_flag_accepts_an_explicit_value() {
        // Needed for settings that default to true — a bare flag could never turn one off.
        let cli = run_cli(&["--admin-config-write=false"]);
        assert_eq!(cli.values.get("ADMIN_CONFIG_WRITE").unwrap(), "false");

        let cli = run_cli(&["--admin-config-write=true"]);
        assert_eq!(cli.values.get("ADMIN_CONFIG_WRITE").unwrap(), "true");
    }

    #[test]
    fn test_boolean_flag_rejects_a_non_boolean_value() {
        let err = parse(args(&["--admin-config-write=maybe"])).unwrap_err();
        assert!(err.contains("expects true or false"), "{err}");
    }

    #[test]
    fn test_bare_boolean_flag_does_not_swallow_the_next_argument() {
        // `--flag --port 8080` must not read `--port` as the boolean's value.
        let cli = run_cli(&["--rate-limit-reverse-proxy", "--port", "8080"]);
        assert_eq!(cli.values.get("RATE_LIMIT_REVERSE_PROXY").unwrap(), "true");
        assert_eq!(cli.values.get("SERVER_PORT").unwrap(), "8080");
    }

    #[test]
    fn test_unset_flags_do_not_appear_in_the_layer() {
        // Absence matters: an empty CLI layer is what lets lower layers win.
        assert!(run_cli(&["--port", "1"]).values.len() == 1);
    }

    #[test]
    fn test_unknown_flag_is_rejected() {
        let err = parse(args(&["--frobnicate"])).unwrap_err();
        assert!(err.contains("unexpected argument"), "{err}");
        assert!(err.contains("--frobnicate"), "{err}");
    }

    #[test]
    fn test_stray_positional_is_rejected() {
        let err = parse(args(&["run", "extra"])).unwrap_err();
        assert!(err.contains("unexpected argument"), "{err}");
    }

    #[test]
    fn test_flag_without_a_value_is_rejected() {
        let err = parse(args(&["--port"])).unwrap_err();
        assert!(err.contains("--port"), "{err}");
    }

    #[test]
    fn test_non_numeric_value_is_caught_by_config_validation_not_the_parser() {
        // The parser stores raw strings; Config::from_env_provider owns type validation, so
        // there is exactly one place that produces "Invalid SERVER_PORT".
        let cli = run_cli(&["--port", "abc"]);
        assert_eq!(cli.values.get("SERVER_PORT").unwrap(), "abc");
    }

    // ── secret files ──────────────────────────────────────────────────────────

    #[test]
    fn test_secret_flags_read_file_contents() {
        let salt = temp_file("salt-from-a-file-16chars\n", ".secret");
        let key = temp_file("master-from-a-file-16chars\n", ".secret");

        let cli = run_cli(&[
            "--api-key-salt-file",
            salt.path().to_str().unwrap(),
            "--master-api-key-file",
            key.path().to_str().unwrap(),
        ]);

        assert_eq!(
            cli.values.get("API_KEY_SALT").unwrap(),
            "salt-from-a-file-16chars"
        );
        assert_eq!(
            cli.values.get("MASTER_API_KEY").unwrap(),
            "master-from-a-file-16chars"
        );
    }

    #[test]
    fn test_debug_of_cli_never_leaks_a_secret_read_from_a_file() {
        let salt = temp_file("salt-from-a-file-16chars\n", ".secret");
        let cli = run_cli(&["--api-key-salt-file", salt.path().to_str().unwrap()]);

        let rendered = format!("{cli:?}");
        assert!(
            !rendered.contains("salt-from-a-file-16chars"),
            "Cli Debug leaked a secret: {rendered}"
        );
        assert!(rendered.contains("<redacted"), "{rendered}");
    }

    #[test]
    fn test_debug_of_layers_never_leaks_a_secret() {
        // The `cli` layer holds the contents of any --*-file flag, and `carried` holds the
        // running config's boot values — both include the salt and master key in the clear.
        let salt = temp_file("salt-from-a-file-16chars\n", ".secret");
        let cli = run_cli(&["--api-key-salt-file", salt.path().to_str().unwrap()]);
        let mut layers = load_layers(&cli).unwrap();
        layers.carried = Config::for_test().boot_layer();

        let rendered = format!("{layers:?}");
        assert!(
            !rendered.contains("salt-from-a-file-16chars"),
            "Layers Debug leaked a secret: {rendered}"
        );
        assert!(
            !rendered.contains("test-master-key-minimum-16chars"),
            "Layers Debug leaked the carried master key: {rendered}"
        );
        assert!(rendered.contains("<redacted"), "{rendered}");
    }

    #[test]
    fn test_missing_secret_file_is_an_error() {
        let err = parse(args(&["--api-key-salt-file", "/nonexistent/salt"])).unwrap_err();
        assert!(err.contains("cannot read secret file"), "{err}");
    }

    #[test]
    fn test_secrets_have_no_plain_value_flag() {
        // A plain --api-key-salt=<value> would leak through `ps aux`; assert it is not accepted.
        let err = parse(args(&["--api-key-salt", "hunter2hunter2hunter2"])).unwrap_err();
        assert!(err.contains("unexpected argument"), "{err}");
    }

    // ── layer loading ─────────────────────────────────────────────────────────

    #[test]
    fn test_load_layers_reads_config_and_env_files() {
        let toml = temp_file("[server]\nport = 4444\n", ".toml");
        let env = temp_file("SERVER_HOST=10.0.0.1\n", ".env");

        let cli = Cli {
            config_file: Some(toml.path().to_path_buf()),
            env_file: Some(env.path().to_path_buf()),
            ..Default::default()
        };
        let layers = load_layers(&cli).unwrap();

        assert_eq!(layers.file.get("SERVER_PORT").unwrap(), "4444");
        assert_eq!(layers.env_file.get("SERVER_HOST").unwrap(), "10.0.0.1");
    }

    #[test]
    fn test_explicit_missing_env_file_is_an_error() {
        let cli = Cli {
            env_file: Some(PathBuf::from("/nonexistent/.env")),
            ..Default::default()
        };
        let err = load_layers(&cli).unwrap_err();
        assert!(err.contains("cannot read env file"), "{err}");
    }

    #[test]
    fn test_no_env_file_skips_loading() {
        let env = temp_file("SERVER_HOST=10.0.0.1\n", ".env");
        let cli = Cli {
            env_file: Some(env.path().to_path_buf()),
            no_env_file: true,
            ..Default::default()
        };
        assert!(load_layers(&cli).unwrap().env_file.is_empty());
    }

    #[test]
    fn test_missing_config_file_is_an_error() {
        let cli = Cli {
            config_file: Some(PathBuf::from("/nonexistent/captchapi.toml")),
            ..Default::default()
        };
        let err = load_layers(&cli).unwrap_err();
        assert!(err.contains("cannot read config file"), "{err}");
    }

    #[test]
    fn test_layers_resolve_with_cli_winning_over_file() {
        let toml = temp_file(
            "[server]\nport = 4444\n[captcha]\ncompression = 55\n",
            ".toml",
        );
        let cli = Cli {
            config_file: Some(toml.path().to_path_buf()),
            values: [
                ("SERVER_PORT".to_string(), "8080".to_string()),
                (
                    "API_KEY_SALT".to_string(),
                    "salt-minimum-16-chars".to_string(),
                ),
                (
                    "MASTER_API_KEY".to_string(),
                    "master-minimum-16-chars".to_string(),
                ),
            ]
            .into_iter()
            .collect(),
            no_env_file: true,
            ..Default::default()
        };

        let layers = load_layers(&cli).unwrap();
        let empty = MockEnv(HashMap::new());
        let config = Config::from_env_provider(&layers.stack(&empty)).unwrap();

        assert_eq!(config.server_port, 8080, "CLI should beat the config file");
        assert_eq!(config.captcha_compression, 55, "file value should apply");
    }

    // ── help rendering ────────────────────────────────────────────────────────

    #[test]
    fn test_help_documents_every_parameter() {
        let text = help();
        for param in PARAMS {
            assert!(text.contains(param.flag), "help is missing {}", param.flag);
            assert!(text.contains(param.env), "help is missing {}", param.env);
            assert!(
                text.contains(param.help),
                "help is missing text for {}",
                param.env
            );
        }
    }

    #[test]
    fn test_help_lists_every_verb() {
        let text = help();
        for verb in ["run", "config show", "config check", "reload"] {
            assert!(text.contains(verb), "help is missing `{verb}`");
        }
    }

    #[test]
    fn test_help_marks_required_parameters() {
        assert!(help().contains("required"));
    }

    #[test]
    fn test_help_never_suggests_a_plain_secret_flag() {
        let text = help();
        assert!(!text.contains("--api-key-salt <"), "{text}");
        assert!(!text.contains("--master-api-key <"), "{text}");
    }

    #[test]
    fn test_version_includes_the_crate_version() {
        assert_eq!(
            version(),
            format!("captchapi {}", env!("CARGO_PKG_VERSION"))
        );
    }

    // ── config show ───────────────────────────────────────────────────────────

    #[test]
    fn test_render_config_redacts_secrets_and_shows_provenance() {
        let config = Config::for_test();
        let layers = Layers {
            cli: [("SERVER_PORT".to_string(), "3000".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let empty = MockEnv(HashMap::new());
        let rendered = render_config(&config, &layers.stack(&empty));

        assert!(
            !rendered.contains(&config.api_key_salt),
            "config show leaked the salt: {rendered}"
        );
        assert!(
            !rendered.contains(&config.master_api_key),
            "config show leaked the master key: {rendered}"
        );
        assert!(rendered.contains("<redacted"), "{rendered}");
        assert!(rendered.contains("[cli]"), "{rendered}");
        assert!(rendered.contains("[default]"), "{rendered}");
    }

    #[test]
    fn test_render_config_lists_every_parameter() {
        let layers = Layers::default();
        let empty = MockEnv(HashMap::new());
        let rendered = render_config(&Config::for_test(), &layers.stack(&empty));
        for param in PARAMS {
            assert!(rendered.contains(param.field), "missing {}", param.field);
        }
    }

    // ── pid files ─────────────────────────────────────────────────────────────

    #[test]
    fn test_pid_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("captchapi.pid");

        write_pid_file(&path).unwrap();
        assert_eq!(read_pid_file(&path).unwrap(), std::process::id() as i32);

        remove_pid_file(&path);
        assert!(!path.exists());
    }

    #[test]
    fn test_write_pid_file_creates_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/captchapi.pid");
        write_pid_file(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_remove_pid_file_is_forgiving() {
        remove_pid_file(Path::new("/nonexistent/captchapi.pid"));
    }

    #[test]
    fn test_read_pid_file_missing_suggests_the_override() {
        let err = read_pid_file(Path::new("/nonexistent/captchapi.pid")).unwrap_err();
        assert!(err.contains("--pid"), "{err}");
    }

    #[test]
    fn test_read_pid_file_rejects_garbage() {
        let f = temp_file("not-a-pid\n", ".pid");
        let err = read_pid_file(f.path()).unwrap_err();
        assert!(err.contains("does not contain a process id"), "{err}");
    }

    #[test]
    fn test_reload_pid_file_precedence() {
        let env = MockEnv(
            [("PID_FILE".to_string(), "/from/env.pid".to_string())]
                .into_iter()
                .collect(),
        );

        // Explicit flag wins.
        let target = ReloadTarget {
            pid_file: Some(PathBuf::from("/from/flag.pid")),
            no_env_file: true,
            ..Default::default()
        };
        assert_eq!(
            reload_pid_file(&target, &env),
            PathBuf::from("/from/flag.pid")
        );

        // Then the environment.
        let target = ReloadTarget {
            no_env_file: true,
            ..Default::default()
        };
        assert_eq!(
            reload_pid_file(&target, &env),
            PathBuf::from("/from/env.pid")
        );

        // Then the declared default.
        let empty = MockEnv(HashMap::new());
        assert_eq!(
            reload_pid_file(&target, &empty),
            PathBuf::from("./data/captchapi.pid")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_send_reload_signal_reports_a_dead_process() {
        // PID 0 addresses the caller's process group on Linux, so use an unlikely-but-valid
        // pid instead and assert we surface the OS error rather than panicking.
        let err = send_reload_signal(i32::MAX).unwrap_err();
        assert!(err.contains("cannot signal process"), "{err}");
    }

    // ── handle ────────────────────────────────────────────────────────────────

    #[test]
    fn test_handle_help_and_version_are_done() {
        assert!(matches!(handle(Action::Help).unwrap(), Handled::Done));
        assert!(matches!(handle(Action::Version).unwrap(), Handled::Done));
    }

    #[test]
    fn test_handle_reload_without_a_pid_file_fails_cleanly() {
        let action = Action::Reload(ReloadTarget {
            pid_file: Some(PathBuf::from("/nonexistent/captchapi.pid")),
            ..ReloadTarget::default()
        });
        let err = handle(action).unwrap_err();
        assert!(err.contains("cannot read pid file"), "{err}");
    }
}
