//! NAPI-compatible types for Node.js bindings
//!
//! These types are designed to work well with JavaScript/TypeScript
//! and provide automatic type generation.

use napi::bindgen_prelude::*;
use napi_derive::napi;

/// Configuration for initializing the CaptchaApi
#[napi(object)]
#[derive(Debug, Clone)]
pub struct CaptchaConfig {
    /// SQLite database URL (e.g., "sqlite:./captcha.db")
    pub database_url: String,

    /// Salt used for hashing API keys
    pub api_key_salt: String,

    /// Default session TTL in seconds (default: 300)
    pub default_session_ttl_seconds: Option<u32>,

    /// Maximum allowed session TTL in seconds (default: 3600)
    pub max_session_ttl_seconds: Option<u32>,

    /// Maximum validation attempts per session (default: 3)
    pub max_validation_attempts: Option<i32>,

    /// Run database migrations on startup (default: true)
    pub run_migrations: Option<bool>,

    /// Default JPEG compression quality 1-100 (default: 40)
    pub captcha_compression: Option<i32>,
}

impl Default for CaptchaConfig {
    fn default() -> Self {
        Self {
            database_url: "sqlite:./captcha.db".to_string(),
            api_key_salt: String::new(),
            default_session_ttl_seconds: Some(300),
            max_session_ttl_seconds: Some(3600),
            max_validation_attempts: Some(3),
            run_migrations: Some(true),
            captcha_compression: Some(40),
        }
    }
}

/// Options for creating a new CAPTCHA session
#[napi(object)]
#[derive(Debug, Clone, Default)]
pub struct CreateSessionOptions {
    /// CAPTCHA text length 1-20 characters (default: 5)
    pub length: Option<i32>,

    /// Session expiration in seconds (uses default if not provided)
    pub expires_in_seconds: Option<u32>,

    /// Difficulty level from 1-10 (default: 5)
    pub difficulty: Option<i32>,

    /// Image width in pixels (default: 220)
    pub width: Option<i32>,

    /// Image height in pixels (default: 120)
    pub height: Option<i32>,

    /// Use dark mode colors (default: false)
    pub dark_mode: Option<bool>,

    /// JPEG compression quality 1-100 (default: 40)
    pub compression: Option<i32>,
}

/// Result of creating a CAPTCHA session
#[napi(object)]
#[derive(Clone)]
pub struct SessionResult {
    /// Unique session identifier (UUID)
    pub session_id: String,

    /// The generated CAPTCHA text (solution).
    /// Available in the library API for server-side use.
    /// Note: The HTTP REST API does NOT expose this field to clients.
    pub text: String,

    /// Session creation timestamp (Unix milliseconds)
    pub created_at: i64,

    /// Session expiration timestamp (Unix milliseconds)
    pub expires_at: i64,

    /// CAPTCHA image as JPEG bytes
    pub image: Buffer,
}

/// Session information (without the image)
#[napi(object)]
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Unique session identifier (UUID)
    pub session_id: String,

    /// Session creation timestamp (Unix milliseconds)
    pub created_at: i64,

    /// Session expiration timestamp (Unix milliseconds)
    pub expires_at: i64,

    /// Number of validation attempts made
    pub attempt_count: i32,

    /// Difficulty level (1-10)
    pub difficulty: i32,

    /// Image width in pixels
    pub width: i32,

    /// Image height in pixels
    pub height: i32,

    /// Whether dark mode is enabled
    pub dark_mode: bool,
}

/// Result of validating a CAPTCHA solution
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the solution was correct
    pub valid: bool,

    /// Session ID that was validated
    pub session_id: String,

    /// Number of attempts remaining (0 if exhausted or valid)
    pub attempts_remaining: i32,
}

/// Information about an API key
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ApiKeyInfo {
    /// SHA256 hash of the API key
    pub key_hash: String,

    /// Optional description of the key
    pub description: Option<String>,

    /// Creation timestamp (Unix milliseconds)
    pub created_at: i64,

    /// Last usage timestamp (Unix milliseconds), if ever used
    pub last_used_at: Option<i64>,

    /// Whether the key is active
    pub is_active: bool,
}

/// Result of creating a new API key
#[napi(object)]
#[derive(Debug, Clone)]
pub struct CreateApiKeyResult {
    /// The raw API key (only returned once, store it safely!)
    pub api_key: String,

    /// The hash of the API key (for reference)
    pub key_hash: String,
}

/// Options for generating a CAPTCHA without storing it
#[napi(object)]
#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    /// CAPTCHA text length 1-20 characters (default: 5)
    pub length: Option<i32>,

    /// Difficulty level from 1-10 (default: 5)
    pub difficulty: Option<i32>,

    /// Image width in pixels (default: 220)
    pub width: Option<i32>,

    /// Image height in pixels (default: 120)
    pub height: Option<i32>,

    /// Use dark mode colors (default: false)
    pub dark_mode: Option<bool>,

    /// JPEG compression quality 1-100 (default: 40)
    pub compression: Option<i32>,
}

/// Result of generating a CAPTCHA (stateless, not stored)
#[napi(object)]
#[derive(Clone)]
pub struct GenerateResult {
    /// The CAPTCHA solution text
    pub solution: String,

    /// CAPTCHA image as JPEG bytes
    pub image: Buffer,
}
