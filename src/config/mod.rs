use crate::models::SessionConfig;
use std::env;
use std::fmt;

pub mod handle;
pub mod params;
pub mod sources;

pub use handle::ConfigHandle;
pub use params::{Kind, Param, Reload, PARAMS};
pub use sources::{Layer, LayeredEnv, Source};

/// Trait for providing environment variables (enables testing without modifying global state)
pub trait EnvProvider {
    fn get(&self, key: &str) -> Result<String, env::VarError>;
}

/// Production implementation using real environment variables
pub struct RealEnv;

impl EnvProvider for RealEnv {
    fn get(&self, key: &str) -> Result<String, env::VarError> {
        env::var(key)
    }
}

/// Parse a boolean leniently, accepting the spellings operators actually use.
///
/// Returns `None` for anything unrecognised so callers can decide between erroring and
/// defaulting.
pub fn parse_bool_lenient(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub server_host: String,
    pub server_port: u16,
    pub pid_file: String,
    pub database_url: String,
    pub database_max_connections: u32,
    pub api_key_salt: String,
    pub master_api_key: String,
    pub default_session_ttl_seconds: u64,
    pub max_session_ttl_seconds: u64,
    pub max_validation_attempts: i64,
    pub cleanup_interval_seconds: u64,
    // Rate limiting (tower_governor)
    pub rate_limit_requests_per_second: u64,
    pub rate_limit_burst_size: u32,
    pub rate_limit_reverse_proxy: bool,
    pub captcha_compression: u8,
    /// Whether `PATCH /api/v1/admin/config` may change settings at runtime.
    pub admin_config_write: bool,
    // Observability
    pub log_level: String,
    pub otel_enabled: bool,
    pub otel_endpoint: String,
    pub otel_service_name: String,
}

/// Redacting `Debug` so a stray `tracing::debug!("{:?}", config)` can never leak the salt or
/// the master key. Deliberately hand-written rather than derived — see the test at the bottom
/// of this module, which fails if a future field carries a secret in the clear.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("server_host", &self.server_host)
            .field("server_port", &self.server_port)
            .field("pid_file", &self.pid_file)
            .field("database_url", &self.database_url)
            .field("database_max_connections", &self.database_max_connections)
            .field("api_key_salt", &Redacted(self.api_key_salt.len()))
            .field("master_api_key", &Redacted(self.master_api_key.len()))
            .field(
                "default_session_ttl_seconds",
                &self.default_session_ttl_seconds,
            )
            .field("max_session_ttl_seconds", &self.max_session_ttl_seconds)
            .field("max_validation_attempts", &self.max_validation_attempts)
            .field("cleanup_interval_seconds", &self.cleanup_interval_seconds)
            .field(
                "rate_limit_requests_per_second",
                &self.rate_limit_requests_per_second,
            )
            .field("rate_limit_burst_size", &self.rate_limit_burst_size)
            .field("rate_limit_reverse_proxy", &self.rate_limit_reverse_proxy)
            .field("captcha_compression", &self.captcha_compression)
            .field("admin_config_write", &self.admin_config_write)
            .field("log_level", &self.log_level)
            .field("otel_enabled", &self.otel_enabled)
            .field("otel_endpoint", &self.otel_endpoint)
            .field("otel_service_name", &self.otel_service_name)
            .finish()
    }
}

/// Placeholder rendered in place of a secret.
struct Redacted(usize);

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted, {} bytes>", self.0)
    }
}

impl Config {
    /// Load configuration from environment variables (production use)
    pub fn from_env() -> Result<Self, String> {
        Self::from_env_provider(&RealEnv)
    }

    /// Load configuration from a custom environment provider (testing use)
    pub fn from_env_provider<E: EnvProvider>(env: &E) -> Result<Self, String> {
        Ok(Config {
            server_host: env
                .get("SERVER_HOST")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            server_port: env
                .get("SERVER_PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .map_err(|_| "Invalid SERVER_PORT: must be a valid port number (0-65535)")?,
            pid_file: env
                .get("PID_FILE")
                .unwrap_or_else(|_| "./data/captchapi.pid".to_string()),
            database_url: env
                .get("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:./data/captchapi.db".to_string()),
            database_max_connections: env
                .get("DATABASE_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .map_err(|_| "Invalid DATABASE_MAX_CONNECTIONS: must be a positive integer")?,
            api_key_salt: {
                let salt = env
                    .get("API_KEY_SALT")
                    .map_err(|_| "API_KEY_SALT must be set")?;
                if salt.len() < 16 {
                    return Err("API_KEY_SALT must be at least 16 bytes for security".to_string());
                }
                salt
            },
            master_api_key: {
                let key = env
                    .get("MASTER_API_KEY")
                    .map_err(|_| "MASTER_API_KEY must be set")?;
                if key.len() < 16 {
                    return Err("MASTER_API_KEY must be at least 16 bytes for security".to_string());
                }
                key
            },
            default_session_ttl_seconds: env
                .get("DEFAULT_SESSION_TTL_SECONDS")
                .unwrap_or_else(|_| "300".to_string())
                .parse()
                .map_err(|_| "Invalid DEFAULT_SESSION_TTL_SECONDS: must be a positive number")?,
            max_session_ttl_seconds: env
                .get("MAX_SESSION_TTL_SECONDS")
                .unwrap_or_else(|_| "3600".to_string())
                .parse()
                .map_err(|_| "Invalid MAX_SESSION_TTL_SECONDS: must be a positive number")?,
            max_validation_attempts: env
                .get("MAX_VALIDATION_ATTEMPTS")
                .unwrap_or_else(|_| "3".to_string())
                .parse()
                .map_err(|_| "Invalid MAX_VALIDATION_ATTEMPTS: must be a positive integer")?,
            cleanup_interval_seconds: env
                .get("CLEANUP_INTERVAL_SECONDS")
                .unwrap_or_else(|_| "60".to_string())
                .parse()
                .map_err(|_| "Invalid CLEANUP_INTERVAL_SECONDS: must be a positive number")?,
            rate_limit_requests_per_second: env
                .get("RATE_LIMIT_REQUESTS_PER_SECOND")
                .unwrap_or_else(|_| "2".to_string())
                .parse()
                .map_err(|_| {
                    "Invalid RATE_LIMIT_REQUESTS_PER_SECOND: must be a positive integer"
                })?,
            rate_limit_burst_size: env
                .get("RATE_LIMIT_BURST_SIZE")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .map_err(|_| "Invalid RATE_LIMIT_BURST_SIZE: must be a positive integer")?,
            rate_limit_reverse_proxy: env
                .get("RATE_LIMIT_REVERSE_PROXY")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .map_err(|_| "Invalid RATE_LIMIT_REVERSE_PROXY: must be 'true' or 'false'")?,
            captcha_compression: env
                .get("CAPTCHA_COMPRESSION")
                .unwrap_or_else(|_| "40".to_string())
                .parse::<u8>()
                .map_err(|_| "Invalid CAPTCHA_COMPRESSION: must be a number between 1 and 100")?
                .clamp(1, 100),
            admin_config_write: {
                let raw = env
                    .get("ADMIN_CONFIG_WRITE")
                    .unwrap_or_else(|_| "true".to_string());
                parse_bool_lenient(&raw)
                    .ok_or("Invalid ADMIN_CONFIG_WRITE: must be 'true' or 'false'")?
            },
            log_level: env
                .get("RUST_LOG")
                .unwrap_or_else(|_| "captchapi=debug,tower_http=debug".to_string()),
            otel_enabled: {
                let raw = env
                    .get("OTEL_ENABLED")
                    .unwrap_or_else(|_| "false".to_string());
                parse_bool_lenient(&raw).ok_or("Invalid OTEL_ENABLED: must be 'true' or 'false'")?
            },
            otel_endpoint: env
                .get("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:4318".to_string()),
            otel_service_name: env
                .get("OTEL_SERVICE_NAME")
                .unwrap_or_else(|_| "captchapi".to_string()),
        })
    }

    pub fn server_address(&self) -> String {
        format!("{}:{}", self.server_host, self.server_port)
    }

    /// Create a `SessionConfig` from the relevant config fields.
    ///
    /// These four values are exactly the ones read per request, which is why they are also
    /// exactly the reloadable set.
    pub fn session_config(&self) -> SessionConfig {
        SessionConfig {
            default_session_ttl_seconds: self.default_session_ttl_seconds,
            max_session_ttl_seconds: self.max_session_ttl_seconds,
            max_validation_attempts: self.max_validation_attempts,
            captcha_compression: self.captcha_compression as i64,
        }
    }

    /// The current value of a field, by its `Config` field name, in canonical string form.
    ///
    /// Backs `config show`, the admin API, and the anti-drift test that keeps
    /// [`PARAMS`] honest. Returns `None` for an unknown field name.
    pub fn field_value(&self, field: &str) -> Option<String> {
        Some(match field {
            "server_host" => self.server_host.clone(),
            "server_port" => self.server_port.to_string(),
            "pid_file" => self.pid_file.clone(),
            "database_url" => self.database_url.clone(),
            "database_max_connections" => self.database_max_connections.to_string(),
            "api_key_salt" => self.api_key_salt.clone(),
            "master_api_key" => self.master_api_key.clone(),
            "default_session_ttl_seconds" => self.default_session_ttl_seconds.to_string(),
            "max_session_ttl_seconds" => self.max_session_ttl_seconds.to_string(),
            "max_validation_attempts" => self.max_validation_attempts.to_string(),
            "cleanup_interval_seconds" => self.cleanup_interval_seconds.to_string(),
            "rate_limit_requests_per_second" => self.rate_limit_requests_per_second.to_string(),
            "rate_limit_burst_size" => self.rate_limit_burst_size.to_string(),
            "rate_limit_reverse_proxy" => self.rate_limit_reverse_proxy.to_string(),
            "captcha_compression" => self.captcha_compression.to_string(),
            "admin_config_write" => self.admin_config_write.to_string(),
            "log_level" => self.log_level.clone(),
            "otel_enabled" => self.otel_enabled.to_string(),
            "otel_endpoint" => self.otel_endpoint.clone(),
            "otel_service_name" => self.otel_service_name.clone(),
            _ => return None,
        })
    }

    /// Boot-only values of this config, as a layer.
    ///
    /// Used as the lowest-precedence layer during a reload so re-resolution cannot fail just
    /// because a secret file was rotated away or unmounted after startup — those fields would
    /// have been discarded as boot-only anyway.
    pub fn boot_layer(&self) -> Layer {
        PARAMS
            .iter()
            .filter(|p| p.reload == Reload::Boot)
            .filter_map(|p| self.field_value(p.field).map(|v| (p.env.to_string(), v)))
            .collect()
    }

    /// Names of boot-only fields whose freshly resolved value differs from the running one.
    ///
    /// A pure function so it can be asserted on directly instead of by scraping log output.
    /// The caller warns about each name; the values themselves are never applied.
    pub fn boot_drift(&self, resolved: &Config) -> Vec<&'static str> {
        PARAMS
            .iter()
            .filter(|p| p.reload == Reload::Boot)
            .filter(|p| self.field_value(p.field) != resolved.field_value(p.field))
            .map(|p| p.field)
            .collect()
    }

    /// Return a copy of `resolved` with every boot-only field taken from `self`.
    ///
    /// This is what makes a reload safe: only the live fields can ever move.
    pub fn with_boot_fields_from(&self, resolved: &Config) -> Config {
        Config {
            // Live fields come from the freshly resolved config...
            default_session_ttl_seconds: resolved.default_session_ttl_seconds,
            max_session_ttl_seconds: resolved.max_session_ttl_seconds,
            max_validation_attempts: resolved.max_validation_attempts,
            captcha_compression: resolved.captcha_compression,
            cleanup_interval_seconds: resolved.cleanup_interval_seconds,
            // ...everything else is pinned to the running process.
            server_host: self.server_host.clone(),
            server_port: self.server_port,
            pid_file: self.pid_file.clone(),
            database_url: self.database_url.clone(),
            database_max_connections: self.database_max_connections,
            api_key_salt: self.api_key_salt.clone(),
            master_api_key: self.master_api_key.clone(),
            rate_limit_requests_per_second: self.rate_limit_requests_per_second,
            rate_limit_burst_size: self.rate_limit_burst_size,
            rate_limit_reverse_proxy: self.rate_limit_reverse_proxy,
            admin_config_write: self.admin_config_write,
            log_level: self.log_level.clone(),
            otel_enabled: self.otel_enabled,
            otel_endpoint: self.otel_endpoint.clone(),
            otel_service_name: self.otel_service_name.clone(),
        }
    }

    /// A `Config` with valid placeholder values, for tests and test harnesses.
    ///
    /// Deliberately a named constructor rather than a `Default` impl: `api_key_salt` and
    /// `master_api_key` are the two required parameters with no sensible default, and a
    /// `Default` yielding empty strings would silently build an `AuthService` with an empty
    /// salt. Not `#[cfg(test)]` because `tests/common/mod.rs` is a separate crate.
    #[doc(hidden)]
    pub fn for_test() -> Self {
        Config {
            server_host: "127.0.0.1".to_string(),
            server_port: 3000,
            pid_file: "./data/captchapi.pid".to_string(),
            database_url: "sqlite::memory:".to_string(),
            database_max_connections: 5,
            api_key_salt: "test-salt-minimum-16chars".to_string(),
            master_api_key: "test-master-key-minimum-16chars".to_string(),
            default_session_ttl_seconds: 300,
            max_session_ttl_seconds: 3600,
            max_validation_attempts: 3,
            cleanup_interval_seconds: 60,
            rate_limit_requests_per_second: 2,
            rate_limit_burst_size: 10,
            rate_limit_reverse_proxy: false,
            captcha_compression: 40,
            admin_config_write: true,
            log_level: "captchapi=debug,tower_http=debug".to_string(),
            otel_enabled: false,
            otel_endpoint: "http://localhost:4318".to_string(),
            otel_service_name: "captchapi".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Mock environment provider for testing
    struct MockEnv {
        vars: HashMap<String, String>,
    }

    impl MockEnv {
        fn new() -> Self {
            MockEnv {
                vars: HashMap::new(),
            }
        }

        fn set(&mut self, key: &str, value: &str) {
            self.vars.insert(key.to_string(), value.to_string());
        }

        fn set_all_required(&mut self) {
            self.set("SERVER_HOST", "0.0.0.0");
            self.set("SERVER_PORT", "3000");
            self.set("DATABASE_URL", "sqlite::memory:");
            self.set("DATABASE_MAX_CONNECTIONS", "5");
            self.set("API_KEY_SALT", "test-salt-minimum-16chars");
            self.set("MASTER_API_KEY", "test-master-minimum-16chars");
            self.set("DEFAULT_SESSION_TTL_SECONDS", "300");
            self.set("MAX_SESSION_TTL_SECONDS", "3600");
            self.set("MAX_VALIDATION_ATTEMPTS", "3");
            self.set("CLEANUP_INTERVAL_SECONDS", "60");
            self.set("RATE_LIMIT_REQUESTS_PER_SECOND", "2");
            self.set("RATE_LIMIT_BURST_SIZE", "10");
            self.set("RATE_LIMIT_REVERSE_PROXY", "false");
            self.set("CAPTCHA_COMPRESSION", "40");
        }

        /// Only the two genuinely required parameters, so defaults are exercised.
        fn set_only_secrets(&mut self) {
            self.set("API_KEY_SALT", "test-salt-minimum-16chars");
            self.set("MASTER_API_KEY", "test-master-minimum-16chars");
        }
    }

    impl EnvProvider for MockEnv {
        fn get(&self, key: &str) -> Result<String, env::VarError> {
            self.vars.get(key).cloned().ok_or(env::VarError::NotPresent)
        }
    }

    #[test]
    fn test_config_from_valid_env() {
        let mut env = MockEnv::new();
        env.set_all_required();

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.server_host, "0.0.0.0");
        assert_eq!(config.server_port, 3000);
        assert_eq!(config.database_url, "sqlite::memory:");
        assert_eq!(config.database_max_connections, 5);
        assert_eq!(config.api_key_salt, "test-salt-minimum-16chars");
        assert_eq!(config.master_api_key, "test-master-minimum-16chars");
        assert_eq!(config.default_session_ttl_seconds, 300);
        assert_eq!(config.max_session_ttl_seconds, 3600);
        assert_eq!(config.max_validation_attempts, 3);
        assert_eq!(config.cleanup_interval_seconds, 60);
        assert_eq!(config.rate_limit_requests_per_second, 2);
        assert_eq!(config.rate_limit_burst_size, 10);
        assert!(!config.rate_limit_reverse_proxy);
        assert_eq!(config.captcha_compression, 40);
    }

    #[test]
    fn test_config_missing_api_key_salt() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.vars.remove("API_KEY_SALT"); // Remove required var

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "API_KEY_SALT must be set");
    }

    #[test]
    fn test_config_missing_master_api_key() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.vars.remove("MASTER_API_KEY");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "MASTER_API_KEY must be set");
    }

    #[test]
    fn test_config_api_key_salt_too_short() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("API_KEY_SALT", "short");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least 16 bytes"));
    }

    #[test]
    fn test_config_master_api_key_too_short() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("MASTER_API_KEY", "short");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least 16 bytes"));
    }

    #[test]
    fn test_config_invalid_port_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("SERVER_PORT", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid SERVER_PORT"));
    }

    #[test]
    fn test_config_port_out_of_range() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("SERVER_PORT", "99999"); // > 65535

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid SERVER_PORT"));
    }

    #[test]
    fn test_config_invalid_ttl_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("DEFAULT_SESSION_TTL_SECONDS", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Invalid DEFAULT_SESSION_TTL_SECONDS"));
    }

    #[test]
    fn test_config_invalid_attempts_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("MAX_VALIDATION_ATTEMPTS", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Invalid MAX_VALIDATION_ATTEMPTS"));
    }

    #[test]
    fn test_config_uses_default_values() {
        let mut env = MockEnv::new();
        // Only set required vars, not optional ones
        env.set_only_secrets();

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.server_host, "0.0.0.0"); // Default (changed for Docker compatibility)
        assert_eq!(config.server_port, 3000); // Default
        assert_eq!(config.database_url, "sqlite:./data/captchapi.db"); // Default
        assert_eq!(config.database_max_connections, 5); // Default
        assert_eq!(config.default_session_ttl_seconds, 300); // Default
        assert_eq!(config.max_session_ttl_seconds, 3600); // Default
        assert_eq!(config.max_validation_attempts, 3); // Default
        assert_eq!(config.cleanup_interval_seconds, 60); // Default
        assert_eq!(config.captcha_compression, 40); // Default
    }

    #[test]
    fn test_server_address_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("SERVER_HOST", "127.0.0.1");
        env.set("SERVER_PORT", "8080");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.server_address(), "127.0.0.1:8080");
    }

    #[test]
    fn test_config_custom_port() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("SERVER_PORT", "5000");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.server_port, 5000);
    }

    #[test]
    fn test_config_custom_database_url() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("DATABASE_URL", "sqlite:/custom/path/db.sqlite");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.database_url, "sqlite:/custom/path/db.sqlite");
    }

    #[test]
    fn test_config_custom_ttl_values() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("DEFAULT_SESSION_TTL_SECONDS", "600");
        env.set("MAX_SESSION_TTL_SECONDS", "7200");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.default_session_ttl_seconds, 600);
        assert_eq!(config.max_session_ttl_seconds, 7200);
    }

    #[test]
    fn test_config_custom_compression() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("CAPTCHA_COMPRESSION", "75");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.captcha_compression, 75);
    }

    #[test]
    fn test_config_compression_clamped_to_max() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("CAPTCHA_COMPRESSION", "150"); // Above max

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.captcha_compression, 100); // Clamped to 100
    }

    #[test]
    fn test_config_compression_clamped_to_min() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("CAPTCHA_COMPRESSION", "0"); // Below min

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.captcha_compression, 1); // Clamped to 1
    }

    #[test]
    fn test_config_invalid_compression_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("CAPTCHA_COMPRESSION", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid CAPTCHA_COMPRESSION"));
    }

    #[test]
    fn test_config_custom_rate_limit_requests_per_second() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("RATE_LIMIT_REQUESTS_PER_SECOND", "5");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.rate_limit_requests_per_second, 5);
    }

    #[test]
    fn test_config_custom_rate_limit_burst_size() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("RATE_LIMIT_BURST_SIZE", "20");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.rate_limit_burst_size, 20);
    }

    #[test]
    fn test_config_invalid_rate_limit_requests_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("RATE_LIMIT_REQUESTS_PER_SECOND", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Invalid RATE_LIMIT_REQUESTS_PER_SECOND"));
    }

    #[test]
    fn test_config_invalid_rate_limit_burst_size_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("RATE_LIMIT_BURST_SIZE", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Invalid RATE_LIMIT_BURST_SIZE"));
    }

    #[test]
    fn test_config_rate_limit_default_values() {
        let mut env = MockEnv::new();
        // Only set required vars
        env.set_only_secrets();

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.rate_limit_requests_per_second, 2); // Default
        assert_eq!(config.rate_limit_burst_size, 10); // Default
        assert!(!config.rate_limit_reverse_proxy); // Default is false
    }

    #[test]
    fn test_config_rate_limit_reverse_proxy_enabled() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("RATE_LIMIT_REVERSE_PROXY", "true");

        let config = Config::from_env_provider(&env).unwrap();
        assert!(config.rate_limit_reverse_proxy);
    }

    #[test]
    fn test_config_rate_limit_reverse_proxy_disabled() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("RATE_LIMIT_REVERSE_PROXY", "false");

        let config = Config::from_env_provider(&env).unwrap();
        assert!(!config.rate_limit_reverse_proxy);
    }

    #[test]
    fn test_config_invalid_rate_limit_reverse_proxy_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("RATE_LIMIT_REVERSE_PROXY", "not-a-bool");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Invalid RATE_LIMIT_REVERSE_PROXY"));
    }

    #[test]
    fn test_config_custom_database_max_connections() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("DATABASE_MAX_CONNECTIONS", "10");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.database_max_connections, 10);
    }

    #[test]
    fn test_config_invalid_database_max_connections_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("DATABASE_MAX_CONNECTIONS", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Invalid DATABASE_MAX_CONNECTIONS"));
    }

    #[test]
    fn test_config_invalid_max_session_ttl_format() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("MAX_SESSION_TTL_SECONDS", "not-a-number");

        let result = Config::from_env_provider(&env);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Invalid MAX_SESSION_TTL_SECONDS"));
    }

    #[test]
    fn test_session_config_matches_config_fields() {
        let mut env = MockEnv::new();
        env.set_all_required();
        env.set("DEFAULT_SESSION_TTL_SECONDS", "600");
        env.set("MAX_SESSION_TTL_SECONDS", "7200");
        env.set("MAX_VALIDATION_ATTEMPTS", "5");
        env.set("CAPTCHA_COMPRESSION", "75");

        let config = Config::from_env_provider(&env).unwrap();
        let session_config = config.session_config();

        assert_eq!(session_config.default_session_ttl_seconds, 600);
        assert_eq!(session_config.max_session_ttl_seconds, 7200);
        assert_eq!(session_config.max_validation_attempts, 5);
        assert_eq!(session_config.captcha_compression, 75);
    }

    // ── observability fields ──────────────────────────────────────────────────

    #[test]
    fn test_observability_defaults() {
        let mut env = MockEnv::new();
        env.set_only_secrets();

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.log_level, "captchapi=debug,tower_http=debug");
        assert!(!config.otel_enabled);
        assert_eq!(config.otel_endpoint, "http://localhost:4318");
        assert_eq!(config.otel_service_name, "captchapi");
        assert_eq!(config.pid_file, "./data/captchapi.pid");
    }

    #[test]
    fn test_observability_overrides() {
        let mut env = MockEnv::new();
        env.set_only_secrets();
        env.set("RUST_LOG", "captchapi=trace");
        env.set("OTEL_ENABLED", "true");
        env.set("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318");
        env.set("OTEL_SERVICE_NAME", "my-service");
        env.set("PID_FILE", "/run/captchapi.pid");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.log_level, "captchapi=trace");
        assert!(config.otel_enabled);
        assert_eq!(config.otel_endpoint, "http://collector:4318");
        assert_eq!(config.otel_service_name, "my-service");
        assert_eq!(config.pid_file, "/run/captchapi.pid");
    }

    #[test]
    fn test_otel_enabled_accepts_operator_spellings() {
        for value in ["true", "1", "yes", "on", "TRUE", "Yes", "ON"] {
            let mut env = MockEnv::new();
            env.set_only_secrets();
            env.set("OTEL_ENABLED", value);
            assert!(
                Config::from_env_provider(&env).unwrap().otel_enabled,
                "{value} should enable telemetry"
            );
        }
        for value in ["false", "0", "no", "off", "FALSE"] {
            let mut env = MockEnv::new();
            env.set_only_secrets();
            env.set("OTEL_ENABLED", value);
            assert!(
                !Config::from_env_provider(&env).unwrap().otel_enabled,
                "{value} should disable telemetry"
            );
        }
    }

    #[test]
    fn test_otel_enabled_rejects_nonsense() {
        let mut env = MockEnv::new();
        env.set_only_secrets();
        env.set("OTEL_ENABLED", "maybe");

        let err = Config::from_env_provider(&env).unwrap_err();
        assert!(err.contains("Invalid OTEL_ENABLED"), "{err}");
    }

    #[test]
    fn test_parse_bool_lenient_rejects_unknown() {
        assert_eq!(parse_bool_lenient("maybe"), None);
        assert_eq!(parse_bool_lenient(""), None);
        assert_eq!(parse_bool_lenient(" true "), Some(true));
    }

    // ── secret hygiene ────────────────────────────────────────────────────────

    #[test]
    fn test_debug_never_leaks_secrets() {
        let config = Config::for_test();
        let rendered = format!("{:?}", config);

        assert!(
            !rendered.contains(&config.api_key_salt),
            "Debug leaked the salt: {rendered}"
        );
        assert!(
            !rendered.contains(&config.master_api_key),
            "Debug leaked the master key: {rendered}"
        );
        assert!(rendered.contains("<redacted"), "{rendered}");
        // Non-secret fields must still be visible, or the impl is useless for debugging.
        assert!(rendered.contains("server_port"), "{rendered}");
    }

    #[test]
    fn test_debug_covers_every_param() {
        // A field added to Config but forgotten in the hand-written Debug impl would be
        // invisible in logs; catch that here rather than in production.
        let rendered = format!("{:?}", Config::for_test());
        for p in PARAMS {
            assert!(
                rendered.contains(p.field),
                "Debug impl is missing field `{}`",
                p.field
            );
        }
    }

    // ── PARAMS <-> Config consistency ─────────────────────────────────────────

    #[test]
    fn test_every_param_maps_to_a_real_config_field() {
        let config = Config::for_test();
        for p in PARAMS {
            assert!(
                config.field_value(p.field).is_some(),
                "PARAMS lists `{}` but Config::field_value does not know it",
                p.field
            );
        }
    }

    #[test]
    fn test_field_value_rejects_unknown_field() {
        assert_eq!(Config::for_test().field_value("nope"), None);
    }

    #[test]
    fn test_param_defaults_match_the_code_defaults() {
        // The anti-drift test that makes PARAMS trustworthy: resolve with only the two required
        // secrets set, then assert every declared default is what the code actually produces.
        let mut env = MockEnv::new();
        env.set_only_secrets();
        let config = Config::from_env_provider(&env).unwrap();

        for p in PARAMS {
            let Some(declared) = p.default else { continue };
            let actual = config.field_value(p.field).unwrap();
            assert_eq!(
                actual, declared,
                "PARAMS says {} defaults to `{}`, but the code produces `{}`",
                p.env, declared, actual
            );
        }
    }

    #[test]
    fn test_params_without_defaults_are_exactly_the_secrets() {
        let required: Vec<_> = PARAMS
            .iter()
            .filter(|p| p.default.is_none())
            .map(|p| p.env)
            .collect();
        assert_eq!(required, vec!["API_KEY_SALT", "MASTER_API_KEY"]);
    }

    // ── boot/live partition ───────────────────────────────────────────────────

    #[test]
    fn test_boot_layer_contains_boot_fields_only() {
        let config = Config::for_test();
        let layer = config.boot_layer();

        assert_eq!(layer.get("SERVER_PORT").unwrap(), "3000");
        assert_eq!(layer.get("API_KEY_SALT").unwrap(), &config.api_key_salt);
        // Live fields must be absent, or a reload could never change them.
        assert!(!layer.contains_key("CAPTCHA_COMPRESSION"));
        assert!(!layer.contains_key("MAX_VALIDATION_ATTEMPTS"));
    }

    #[test]
    fn test_boot_drift_reports_nothing_when_unchanged() {
        let a = Config::for_test();
        let b = Config::for_test();
        assert!(a.boot_drift(&b).is_empty());
    }

    #[test]
    fn test_boot_drift_names_changed_boot_fields() {
        let running = Config::for_test();
        let mut resolved = Config::for_test();
        resolved.server_port = 9999;
        resolved.database_url = "sqlite:/elsewhere.db".to_string();

        let drift = running.boot_drift(&resolved);
        assert!(drift.contains(&"server_port"), "{drift:?}");
        assert!(drift.contains(&"database_url"), "{drift:?}");
    }

    #[test]
    fn test_boot_drift_ignores_live_fields() {
        let running = Config::for_test();
        let mut resolved = Config::for_test();
        resolved.captcha_compression = 90;
        resolved.max_validation_attempts = 7;

        assert!(running.boot_drift(&resolved).is_empty());
    }

    #[test]
    fn test_with_boot_fields_from_takes_live_and_pins_boot() {
        let running = Config::for_test();
        let mut resolved = Config::for_test();
        // Live changes should be adopted...
        resolved.captcha_compression = 90;
        resolved.max_validation_attempts = 7;
        resolved.default_session_ttl_seconds = 900;
        resolved.max_session_ttl_seconds = 7200;
        resolved.cleanup_interval_seconds = 15;
        // ...boot changes must be discarded.
        resolved.server_port = 9999;
        resolved.api_key_salt = "a-completely-different-salt".to_string();

        let merged = running.with_boot_fields_from(&resolved);

        assert_eq!(merged.captcha_compression, 90);
        assert_eq!(merged.max_validation_attempts, 7);
        assert_eq!(merged.default_session_ttl_seconds, 900);
        assert_eq!(merged.max_session_ttl_seconds, 7200);
        assert_eq!(merged.cleanup_interval_seconds, 15);
        assert_eq!(merged.server_port, running.server_port);
        assert_eq!(merged.api_key_salt, running.api_key_salt);
    }

    #[test]
    fn test_with_boot_fields_from_is_identity_when_nothing_changed() {
        let running = Config::for_test();
        let resolved = Config::for_test();
        assert_eq!(running.with_boot_fields_from(&resolved), running);
    }

    #[test]
    fn test_boot_layer_lets_resolution_survive_a_missing_secret_file() {
        // The scenario this exists for: a secret is rotated away after startup, then SIGHUP
        // arrives. Re-resolving from an environment with no secrets at all must still succeed
        // once the running config's boot layer is in the stack.
        let running = Config::for_test();
        let boot = running.boot_layer();
        let (cli, env_file, file) = (Layer::new(), Layer::new(), Layer::new());
        let empty = MockEnv::new();
        let layered = LayeredEnv::new(&cli, &empty, &env_file, &file, &boot);

        let resolved = Config::from_env_provider(&layered).unwrap();
        assert_eq!(resolved.api_key_salt, running.api_key_salt);
        assert_eq!(layered.source_of("API_KEY_SALT"), Source::Carried);
    }

    #[test]
    fn test_for_test_config_is_valid_by_the_real_rules() {
        // Guards against a placeholder that the real validator would reject.
        let config = Config::for_test();
        assert!(config.api_key_salt.len() >= 16);
        assert!(config.master_api_key.len() >= 16);
        assert!((1..=100).contains(&config.captcha_compression));
    }
}
