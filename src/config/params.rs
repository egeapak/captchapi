//! The single source of truth for every configuration parameter.
//!
//! [`PARAMS`] drives CLI flag parsing, `--help` rendering, TOML key mapping, provenance
//! reporting in `config show`, and the reloadable/boot-only partition. Adding a new knob is
//! one row here plus one field on [`Config`](super::Config).

/// The type of value a parameter carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain string.
    Str,
    /// An integer.
    Num,
    /// A boolean. The CLI form is a bare flag taking no value.
    Bool,
    /// A path to a file whose *contents* become the value. Used for secrets so they never
    /// appear in `ps aux`, shell history, or `docker inspect`.
    SecretFile,
}

/// Whether a parameter may be stored in the database and survive a restart.
///
/// Separate from [`Reload`], which asks whether the *running* process can adopt a new value.
/// A boot-only parameter can still be stored — the stored value is what the next start reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persist {
    /// Storable. Written to `config_settings` and read back as a configuration layer.
    Allowed,
    /// Never stored, for one of exactly three reasons: it is a secret, it is needed to open
    /// the database the store lives in, or it is consumed before the store is read and has no
    /// way to be reconfigured afterwards.
    Never,
}

/// Whether a parameter can change at runtime or is fixed for the life of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reload {
    /// Captured during startup (listener, pool, middleware, rate limiter). A reload can never
    /// change it, and attempting to will log a warning naming the field.
    Boot,
    /// Read per request or per tick. A reload takes effect immediately.
    Live,
}

/// A single configuration parameter, described once and used everywhere.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    /// The canonical key. Every layer (CLI, environment, TOML file) resolves to this, which is
    /// what lets the CLI plug in as just another [`EnvProvider`](super::EnvProvider).
    pub env: &'static str,
    /// The corresponding field name on [`Config`](super::Config), used for display.
    pub field: &'static str,
    /// Long CLI flag, including the leading `--`.
    pub flag: &'static str,
    /// Optional short CLI flag, including the leading `-`.
    pub short: Option<&'static str>,
    /// Dotted path in the TOML config file. `None` means the parameter is deliberately not
    /// representable in a config file (secrets), so a checked-in file is safe by construction.
    pub toml: Option<&'static str>,
    pub kind: Kind,
    pub reload: Reload,
    /// Whether this parameter may be persisted in the database.
    pub persist: Persist,
    /// Redact the value in `config show` and in the admin API.
    pub secret: bool,
    /// Built-in default, or `None` when the parameter is required.
    pub default: Option<&'static str>,
    /// One-line description for `--help`, where it shares a column with every other flag and
    /// so has to stay short.
    pub help: &'static str,
    /// A sentence explaining what the parameter does, and what changing it costs when that is
    /// not obvious. Reported by the admin API and shown in the console, where there is room to
    /// say the thing `help` has to leave out.
    pub about: &'static str,
}

/// Every configuration parameter the service understands.
pub const PARAMS: &[Param] = &[
    Param {
        env: "SERVER_HOST",
        field: "server_host",
        flag: "--host",
        short: Some("-H"),
        toml: Some("server.host"),
        kind: Kind::Str,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("0.0.0.0"),
        help: "Address to bind the HTTP listener to",
        about: "The address the HTTP listener binds to: 0.0.0.0 accepts connections on every interface, 127.0.0.1 only from this machine.",
    },
    Param {
        env: "SERVER_PORT",
        field: "server_port",
        flag: "--port",
        short: Some("-p"),
        toml: Some("server.port"),
        kind: Kind::Num,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("3000"),
        help: "Port to listen on",
        about: "The TCP port the HTTP listener binds to; 0 asks the operating system for an ephemeral port.",
    },
    Param {
        env: "PID_FILE",
        field: "pid_file",
        flag: "--pid-file",
        short: None,
        toml: Some("server.pid_file"),
        kind: Kind::Str,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("./data/captchapi.pid"),
        help: "Where to write the process ID, so `captchapi reload` can find the server",
        about: "Where the running server writes its process ID, so `captchapi reload` can find it without being told.",
    },
    Param {
        env: "DATABASE_URL",
        field: "database_url",
        flag: "--database-url",
        short: Some("-d"),
        toml: Some("database.url"),
        kind: Kind::Str,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: false,
        default: Some("sqlite:./data/captchapi.db"),
        help: "SQLite connection string (must start with 'sqlite:')",
        about: "The SQLite file holding sessions and API keys; it and its parent directory are created if missing.",
    },
    Param {
        env: "DATABASE_MAX_CONNECTIONS",
        field: "database_max_connections",
        flag: "--database-max-connections",
        short: None,
        toml: Some("database.max_connections"),
        kind: Kind::Num,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: false,
        default: Some("5"),
        help: "Connection pool size",
        about: "How many SQLite connections the pool keeps open; SQLite serialises writers, so raising this helps concurrent reads far more than writes.",
    },
    Param {
        env: "API_KEY_SALT",
        field: "api_key_salt",
        flag: "--api-key-salt-file",
        short: None,
        toml: None,
        kind: Kind::SecretFile,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: true,
        default: None,
        help: "File containing the API key hashing salt (min 16 bytes)",
        about: "Salt mixed into every stored API key hash, and the fallback key for solution hashing and image encryption; changing it invalidates every existing API key.",
    },
    Param {
        env: "MASTER_API_KEY",
        field: "master_api_key",
        flag: "--master-api-key-file",
        short: None,
        toml: None,
        kind: Kind::SecretFile,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: true,
        default: None,
        help: "File containing the master admin key (min 16 bytes)",
        about: "The credential granting full administrative access, including this console; protect it like a root password.",
    },
    Param {
        env: "SOLUTION_HASH_SECRET",
        field: "solution_hash_secret",
        flag: "--solution-hash-secret-file",
        short: None,
        toml: None,
        kind: Kind::SecretFile,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: true,
        // No static default: falls back to API_KEY_SALT at resolution time.
        default: None,
        help: "File containing the CAPTCHA solution hashing key (min 16 bytes; defaults to the API key salt)",
        about: "Key used to hash CAPTCHA solutions so the database never holds an answer in plaintext; rotating it invalidates sessions issued before the restart.",
    },
    Param {
        env: "IMAGE_ENCRYPTION_SECRET",
        field: "image_encryption_secret",
        flag: "--image-encryption-secret-file",
        short: None,
        toml: None,
        kind: Kind::SecretFile,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: true,
        // No static default: falls back to API_KEY_SALT at resolution time.
        default: None,
        help: "File containing the stored-image encryption key (min 16 bytes; defaults to the API key salt)",
        about: "Key used to encrypt stored CAPTCHA images at rest; rotating it leaves images from earlier sessions undecryptable.",
    },
    Param {
        env: "DEFAULT_SESSION_TTL_SECONDS",
        field: "default_session_ttl_seconds",
        flag: "--default-session-ttl",
        short: None,
        toml: Some("captcha.default_ttl_seconds"),
        kind: Kind::Num,
        reload: Reload::Live,
        persist: Persist::Allowed,
        secret: false,
        default: Some("300"),
        help: "Default session lifetime in seconds",
        about: "How long a new session stays valid when the client does not request a specific lifetime.",
    },
    Param {
        env: "MAX_SESSION_TTL_SECONDS",
        field: "max_session_ttl_seconds",
        flag: "--max-session-ttl",
        short: None,
        toml: Some("captcha.max_ttl_seconds"),
        kind: Kind::Num,
        reload: Reload::Live,
        persist: Persist::Allowed,
        secret: false,
        default: Some("3600"),
        help: "Maximum session lifetime a client may request, in seconds",
        about: "The longest lifetime a client may request; a longer request is rejected rather than shortened.",
    },
    Param {
        env: "MAX_VALIDATION_ATTEMPTS",
        field: "max_validation_attempts",
        flag: "--max-validation-attempts",
        short: None,
        toml: Some("captcha.max_validation_attempts"),
        kind: Kind::Num,
        reload: Reload::Live,
        persist: Persist::Allowed,
        secret: false,
        default: Some("3"),
        help: "Failed validation attempts before a session is destroyed",
        about: "How many failed solution attempts a session survives before it is destroyed.",
    },
    Param {
        env: "CAPTCHA_COMPRESSION",
        field: "captcha_compression",
        flag: "--captcha-compression",
        short: None,
        toml: Some("captcha.compression"),
        kind: Kind::Num,
        reload: Reload::Live,
        persist: Persist::Allowed,
        secret: false,
        default: Some("40"),
        help: "JPEG quality from 1 to 100 (values outside the range are clamped)",
        about: "JPEG quality for rendered images, from 1 to 100 and clamped into range; it trades bandwidth against fidelity and is not a security control.",
    },
    Param {
        env: "CLEANUP_INTERVAL_SECONDS",
        field: "cleanup_interval_seconds",
        flag: "--cleanup-interval",
        short: None,
        toml: Some("tasks.cleanup_interval_seconds"),
        kind: Kind::Num,
        reload: Reload::Live,
        persist: Persist::Allowed,
        secret: false,
        default: Some("60"),
        help: "How often the background task removes expired sessions, in seconds",
        about: "How often the background task deletes expired sessions from the database.",
    },
    Param {
        env: "RATE_LIMIT_REQUESTS_PER_SECOND",
        field: "rate_limit_requests_per_second",
        flag: "--rate-limit-rps",
        short: None,
        toml: Some("rate_limit.requests_per_second"),
        kind: Kind::Num,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("2"),
        help: "Sustained request rate allowed per client IP",
        about: "Sustained requests each client IP may make to the session endpoints before being throttled.",
    },
    Param {
        env: "RATE_LIMIT_BURST_SIZE",
        field: "rate_limit_burst_size",
        flag: "--rate-limit-burst",
        short: None,
        toml: Some("rate_limit.burst_size"),
        kind: Kind::Num,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("10"),
        help: "Burst capacity allowed per client IP",
        about: "How many requests a client IP may make back to back before the sustained rate starts to apply.",
    },
    Param {
        env: "RATE_LIMIT_REVERSE_PROXY",
        field: "rate_limit_reverse_proxy",
        flag: "--rate-limit-reverse-proxy",
        short: None,
        toml: Some("rate_limit.reverse_proxy"),
        kind: Kind::Bool,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("false"),
        help: "Read the client IP from proxy headers (only enable behind a trusted proxy)",
        about: "Take the client IP from proxy headers instead of the socket; enable this only behind a proxy you control, because clients can otherwise spoof them and evade rate limiting.",
    },
    Param {
        env: "ADMIN_CONFIG_WRITE",
        field: "admin_config_write",
        flag: "--admin-config-write",
        short: None,
        toml: Some("admin.config_write"),
        kind: Kind::Bool,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("true"),
        help: "Allow PATCH /api/v1/admin/config to change settings at runtime",
        about: "Whether the admin API may change settings at runtime; turning it off still leaves reads and reloads working.",
    },
    Param {
        env: "RUST_LOG",
        field: "log_level",
        flag: "--log-level",
        short: None,
        toml: Some("logging.level"),
        // Boot-only on purpose. Making this live requires `tracing_subscriber::reload::Layer`,
        // whose `register_callsite` returns `Interest::sometimes()` — that permanently disables
        // per-callsite interest caching for the whole subscriber, so every request pays for a
        // feature almost nobody uses. Not a good trade in a service built with `opt-level = "z"`.
        kind: Kind::Str,
        reload: Reload::Boot,
        persist: Persist::Allowed,
        secret: false,
        default: Some("captchapi=debug,tower_http=debug"),
        help: "Tracing filter directives",
        about: "Which modules log and at what level, in RUST_LOG syntax, for example `captchapi=debug,tower_http=info`.",
    },
    Param {
        env: "OTEL_ENABLED",
        field: "otel_enabled",
        flag: "--otel",
        short: None,
        toml: Some("telemetry.enabled"),
        kind: Kind::Bool,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: false,
        default: Some("false"),
        help: "Export traces over OTLP",
        about: "Whether traces and metrics are exported over OTLP; it has no effect in a binary built without the `otel` feature.",
    },
    Param {
        env: "OTEL_EXPORTER_OTLP_ENDPOINT",
        field: "otel_endpoint",
        flag: "--otel-endpoint",
        short: None,
        toml: Some("telemetry.endpoint"),
        kind: Kind::Str,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: false,
        default: Some("http://localhost:4318"),
        help: "OTLP collector endpoint",
        about: "The OTLP collector that receives exported traces and metrics.",
    },
    Param {
        env: "OTEL_SERVICE_NAME",
        field: "otel_service_name",
        flag: "--otel-service-name",
        short: None,
        toml: Some("telemetry.service_name"),
        kind: Kind::Str,
        reload: Reload::Boot,
        persist: Persist::Never,
        secret: false,
        default: Some("captchapi"),
        help: "Service name reported on exported traces",
        about: "The service name attached to exported telemetry, used to tell this service apart in a collector.",
    },
];

/// Look up a parameter by its canonical environment key.
pub fn by_env(env: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.env == env)
}

/// Look up a parameter by its `Config` field name.
pub fn by_field(field: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.field == field)
}

/// Look up a parameter by its dotted TOML path.
pub fn by_toml(path: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.toml == Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_env_keys_are_unique() {
        let mut seen = HashSet::new();
        for p in PARAMS {
            assert!(seen.insert(p.env), "duplicate env key: {}", p.env);
        }
    }

    #[test]
    fn test_flags_are_unique() {
        let mut seen = HashSet::new();
        for p in PARAMS {
            assert!(seen.insert(p.flag), "duplicate flag: {}", p.flag);
            if let Some(s) = p.short {
                assert!(seen.insert(s), "duplicate short flag: {}", s);
            }
        }
    }

    #[test]
    fn test_field_names_are_unique() {
        let mut seen = HashSet::new();
        for p in PARAMS {
            assert!(seen.insert(p.field), "duplicate field: {}", p.field);
        }
    }

    #[test]
    fn test_toml_paths_are_unique_and_sectioned() {
        let mut seen = HashSet::new();
        for p in PARAMS {
            if let Some(path) = p.toml {
                assert!(seen.insert(path), "duplicate toml path: {}", path);
                assert!(
                    path.split('.').count() == 2,
                    "toml path must be section.key: {}",
                    path
                );
            }
        }
    }

    #[test]
    fn test_flags_are_well_formed() {
        for p in PARAMS {
            assert!(
                p.flag.starts_with("--"),
                "long flag must start with --: {}",
                p.flag
            );
            if let Some(s) = p.short {
                assert!(
                    s.starts_with('-') && !s.starts_with("--"),
                    "bad short flag: {}",
                    s
                );
                assert_eq!(s.len(), 2, "short flag must be one character: {}", s);
            }
        }
    }

    #[test]
    fn test_secrets_are_not_representable_in_toml() {
        for p in PARAMS.iter().filter(|p| p.secret) {
            assert!(
                p.toml.is_none(),
                "secret {} must not have a TOML path — a config file must be safe to commit",
                p.env
            );
            assert_eq!(
                p.kind,
                Kind::SecretFile,
                "secret {} must be read from a file",
                p.env
            );
            assert!(
                p.default.is_none(),
                "secret {} must not have a default",
                p.env
            );
        }
    }

    #[test]
    fn test_secret_flags_name_a_file() {
        for p in PARAMS.iter().filter(|p| p.kind == Kind::SecretFile) {
            assert!(
                p.flag.ends_with("-file"),
                "{} takes a path, so its flag should end in -file",
                p.flag
            );
        }
    }

    #[test]
    fn test_every_param_has_help() {
        for p in PARAMS {
            assert!(!p.help.is_empty(), "{} is missing help text", p.env);
        }
    }

    #[test]
    fn test_lookup_helpers() {
        assert_eq!(by_env("SERVER_PORT").unwrap().field, "server_port");
        assert_eq!(by_field("server_port").unwrap().env, "SERVER_PORT");
        assert_eq!(by_toml("server.port").unwrap().env, "SERVER_PORT");
        assert!(by_env("NOPE").is_none());
        assert!(by_field("nope").is_none());
        assert!(by_toml("nope.nope").is_none());
        // Secrets are deliberately absent from the TOML namespace.
        assert!(by_toml("security.api_key_salt").is_none());
    }

    #[test]
    fn test_only_per_request_values_are_live() {
        // Everything else is captured at startup (listener, pool, middleware, rate limiter)
        // and cannot be changed without a restart.
        let live: HashSet<_> = PARAMS
            .iter()
            .filter(|p| p.reload == Reload::Live)
            .map(|p| p.field)
            .collect();
        let expected: HashSet<_> = [
            "default_session_ttl_seconds",
            "max_session_ttl_seconds",
            "max_validation_attempts",
            "captcha_compression",
            "cleanup_interval_seconds",
        ]
        .into_iter()
        .collect();
        assert_eq!(live, expected);
    }

    #[test]
    fn test_bool_params_have_a_boolean_default() {
        // Booleans accept `--flag` (true) and `--flag=false`, so either default is usable.
        for p in PARAMS.iter().filter(|p| p.kind == Kind::Bool) {
            assert!(
                matches!(p.default, Some("true") | Some("false")),
                "{} is a boolean but defaults to {:?}",
                p.env,
                p.default
            );
        }
    }

    #[test]
    fn test_no_secret_is_reloadable() {
        // The admin audit log and the PATCH path would otherwise be able to handle a secret
        // value at runtime. Redaction covers it, but the invariant is worth pinning.
        for p in PARAMS.iter().filter(|p| p.secret) {
            assert_eq!(
                p.reload,
                Reload::Boot,
                "{} is secret, so it must not be runtime-changeable",
                p.env
            );
        }
    }

    #[test]
    fn test_flags_do_not_collide_with_the_global_options() {
        // These are parsed outside the PARAMS loop (`parse_shared` and the `reload` verb), so
        // uniqueness within PARAMS alone is not enough — a colliding row would be silently
        // double-consumed by pico-args, with the first reader winning.
        const GLOBAL: &[&str] = &[
            "-c",
            "--config",
            "--env-file",
            "--no-env-file",
            "-h",
            "--help",
            "-V",
            "--version",
            "--pid",
        ];
        for p in PARAMS {
            assert!(
                !GLOBAL.contains(&p.flag),
                "{} collides with a global option",
                p.flag
            );
            if let Some(short) = p.short {
                assert!(
                    !GLOBAL.contains(&short),
                    "{short} collides with a global option"
                );
            }
        }
    }

    /// The unstorable set, spelled out.
    ///
    /// An allow-list rather than a rule, because the three reasons a parameter cannot be stored
    /// are not things code can derive: "the store lives in the database this opens" and "this is
    /// consumed before the store is read" are facts about `main.rs`'s ordering. Listing them
    /// means adding a parameter forces a decision here instead of defaulting into storability,
    /// which is the direction that fails safe.
    #[test]
    fn test_the_unstorable_parameters_are_exactly_these() {
        let never: HashSet<_> = PARAMS
            .iter()
            .filter(|p| p.persist == Persist::Never)
            .map(|p| p.field)
            .collect();
        let expected: HashSet<_> = [
            // Secrets: the store is a database file the backup story treats as data.
            "api_key_salt",
            "master_api_key",
            "solution_hash_secret",
            "image_encryption_secret",
            // Needed to open the database the store lives in.
            "database_url",
            "database_max_connections",
            // Consumed by init_telemetry, which installs global providers before the store is
            // read and has no reload handle to swap them afterwards.
            "otel_enabled",
            "otel_endpoint",
            "otel_service_name",
        ]
        .into_iter()
        .collect();
        assert_eq!(never, expected);
    }

    /// The invariant behind the first group above, stated independently of the list.
    ///
    /// If a secret ever became storable the allow-list test would still pass after someone
    /// updated it, so the rule is asserted separately from the enumeration.
    #[test]
    fn test_no_secret_is_ever_storable() {
        for p in PARAMS {
            if p.secret {
                assert_eq!(
                    p.persist,
                    Persist::Never,
                    "{} is a secret and must never be stored in the database",
                    p.field
                );
            }
        }
    }

    /// Every parameter explains itself, in a sentence, to whoever is looking at the console.
    ///
    /// The bar is deliberately "a sentence and not the flag help again": `help` shares a column
    /// with every other flag, so it is a phrase like "Port to listen on", which tells an
    /// operator nothing they could not read off the field name.
    #[test]
    fn test_every_param_explains_itself() {
        for p in PARAMS {
            assert!(!p.about.is_empty(), "{} has no `about`", p.field);
            assert!(
                p.about.ends_with('.'),
                "{}: `about` is a sentence and ends with a period, got {:?}",
                p.field,
                p.about
            );
            assert!(
                p.about != p.help,
                "{}: `about` just repeats `help`",
                p.field
            );
            assert!(
                p.about.len() > p.help.len(),
                "{}: `about` should say more than `help`, not less",
                p.field
            );
        }
    }
}
