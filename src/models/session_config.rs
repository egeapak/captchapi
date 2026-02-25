//! Shared session configuration for both HTTP server and NAPI bindings.
//!
//! This type is constructed by external crates (e.g., the NAPI bindings).
//! The `#[allow(dead_code)]` on the struct and the `session_config()` method
//! in `Config` suppress false-positive dead-code lints: the Rust compiler
//! cannot detect cross-crate usage within the same workspace binary target.
#![allow(dead_code)]

/// Shared session configuration for both HTTP server and NAPI bindings.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub default_session_ttl_seconds: u64,
    pub max_session_ttl_seconds: u64,
    pub max_validation_attempts: i64,
    pub captcha_compression: i64,
}
