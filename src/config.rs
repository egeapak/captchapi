use std::env;

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

#[derive(Debug, Clone)]
pub struct Config {
    pub server_host: String,
    pub server_port: u16,
    pub database_url: String,
    pub api_key_salt: String,
    pub master_api_key: String,
    pub default_session_ttl_seconds: u64,
    pub max_session_ttl_seconds: u64,
    pub max_validation_attempts: i64,
    pub cleanup_interval_seconds: u64,
    // Rate limiting (tower_governor)
    pub rate_limit_requests_per_second: u64,
    pub rate_limit_burst_size: u32,
    pub captcha_compression: u8,
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
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            server_port: env
                .get("SERVER_PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .map_err(|_| "Invalid SERVER_PORT: must be a valid port number (0-65535)")?,
            database_url: env
                .get("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:./data/captchapi.db".to_string()),
            api_key_salt: env
                .get("API_KEY_SALT")
                .map_err(|_| "API_KEY_SALT must be set")?,
            master_api_key: env
                .get("MASTER_API_KEY")
                .map_err(|_| "MASTER_API_KEY must be set")?,
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
            captcha_compression: env
                .get("CAPTCHA_COMPRESSION")
                .unwrap_or_else(|_| "40".to_string())
                .parse::<u8>()
                .map_err(|_| "Invalid CAPTCHA_COMPRESSION: must be a number between 1 and 100")?
                .clamp(1, 100),
        })
    }

    pub fn server_address(&self) -> String {
        format!("{}:{}", self.server_host, self.server_port)
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
            self.set("API_KEY_SALT", "test-salt");
            self.set("MASTER_API_KEY", "test-master");
            self.set("DEFAULT_SESSION_TTL_SECONDS", "300");
            self.set("MAX_SESSION_TTL_SECONDS", "3600");
            self.set("MAX_VALIDATION_ATTEMPTS", "3");
            self.set("CLEANUP_INTERVAL_SECONDS", "60");
            self.set("RATE_LIMIT_REQUESTS_PER_SECOND", "2");
            self.set("RATE_LIMIT_BURST_SIZE", "10");
            self.set("CAPTCHA_COMPRESSION", "40");
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
        assert_eq!(config.api_key_salt, "test-salt");
        assert_eq!(config.master_api_key, "test-master");
        assert_eq!(config.default_session_ttl_seconds, 300);
        assert_eq!(config.max_session_ttl_seconds, 3600);
        assert_eq!(config.max_validation_attempts, 3);
        assert_eq!(config.cleanup_interval_seconds, 60);
        assert_eq!(config.rate_limit_requests_per_second, 2);
        assert_eq!(config.rate_limit_burst_size, 10);
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
        env.set("API_KEY_SALT", "test-salt");
        env.set("MASTER_API_KEY", "test-master");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.server_host, "127.0.0.1"); // Default
        assert_eq!(config.server_port, 3000); // Default
        assert_eq!(config.database_url, "sqlite:./data/captchapi.db"); // Default
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
        env.set("API_KEY_SALT", "test-salt");
        env.set("MASTER_API_KEY", "test-master");

        let config = Config::from_env_provider(&env).unwrap();
        assert_eq!(config.rate_limit_requests_per_second, 2); // Default
        assert_eq!(config.rate_limit_burst_size, 10); // Default
    }
}
