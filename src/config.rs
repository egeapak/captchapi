use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub server_host: String,
    pub server_port: u16,
    pub database_url: String,
    pub api_key_salt: String,
    pub default_session_ttl_seconds: u64,
    pub max_session_ttl_seconds: u64,
    pub max_validation_attempts: i32,
    pub cleanup_interval_seconds: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Ok(Config {
            server_host: env::var("SERVER_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            server_port: env::var("SERVER_PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .map_err(|_| "Invalid SERVER_PORT")?,
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:./data/captchapi.db".to_string()),
            api_key_salt: env::var("API_KEY_SALT")
                .map_err(|_| "API_KEY_SALT must be set")?,
            default_session_ttl_seconds: env::var("DEFAULT_SESSION_TTL_SECONDS")
                .unwrap_or_else(|_| "300".to_string())
                .parse()
                .map_err(|_| "Invalid DEFAULT_SESSION_TTL_SECONDS")?,
            max_session_ttl_seconds: env::var("MAX_SESSION_TTL_SECONDS")
                .unwrap_or_else(|_| "3600".to_string())
                .parse()
                .map_err(|_| "Invalid MAX_SESSION_TTL_SECONDS")?,
            max_validation_attempts: env::var("MAX_VALIDATION_ATTEMPTS")
                .unwrap_or_else(|_| "3".to_string())
                .parse()
                .map_err(|_| "Invalid MAX_VALIDATION_ATTEMPTS")?,
            cleanup_interval_seconds: env::var("CLEANUP_INTERVAL_SECONDS")
                .unwrap_or_else(|_| "60".to_string())
                .parse()
                .map_err(|_| "Invalid CLEANUP_INTERVAL_SECONDS")?,
        })
    }

    pub fn server_address(&self) -> String {
        format!("{}:{}", self.server_host, self.server_port)
    }
}
