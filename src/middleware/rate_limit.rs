use crate::error::AppError;
use crate::services::RateLimiter;
use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;

#[derive(Clone)]
pub struct RateLimitMiddleware {
    pub rate_limiter: RateLimiter,
}

impl RateLimitMiddleware {
    pub fn new(rate_limiter: RateLimiter) -> Self {
        Self { rate_limiter }
    }

    /// Middleware function that enforces rate limiting per IP address
    ///
    /// Extracts the client IP from ConnectInfo and checks against the rate limiter.
    /// Returns 429 Too Many Requests via Unauthorized error if the rate limit is exceeded.
    pub async fn check(
        State(middleware): State<RateLimitMiddleware>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        request: Request,
        next: Next,
    ) -> Result<Response, AppError> {
        let ip = addr.ip();

        // Check rate limit
        if !middleware.rate_limiter.check_rate_limit(ip).await {
            tracing::warn!("Rate limit exceeded for IP: {}", ip);
            return Err(AppError::Unauthorized(
                "Rate limit exceeded. Please try again later.".to_string(),
            ));
        }

        Ok(next.run(request).await)
    }
}
