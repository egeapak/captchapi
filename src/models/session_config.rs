//! Shared session configuration for both the HTTP server and the NAPI bindings.
//!
//! These four values are exactly the ones read on the request path, which is what makes them
//! the reloadable subset of [`Config`](crate::config::Config): everything else is captured at
//! startup into the listener, the connection pool, the middleware, or the rate limiter.

/// Shared session configuration for both HTTP server and NAPI bindings.
///
/// `Copy` so the request path can take a cheap snapshot instead of cloning an `Arc` or holding
/// a `watch` read guard across a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    pub default_session_ttl_seconds: u64,
    pub max_session_ttl_seconds: u64,
    pub max_validation_attempts: i64,
    pub captcha_compression: i64,
}
