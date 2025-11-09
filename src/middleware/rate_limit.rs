use crate::error::AppError;
use crate::metrics::Metrics;
use crate::services::RateLimiter;
use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Clone)]
pub struct RateLimitMiddleware {
    pub rate_limiter: RateLimiter,
    pub metrics: Arc<Metrics>,
}

impl RateLimitMiddleware {
    pub fn new(rate_limiter: RateLimiter, metrics: Arc<Metrics>) -> Self {
        Self {
            rate_limiter,
            metrics,
        }
    }

    /// Middleware function that enforces rate limiting per IP address
    ///
    /// Extracts the client IP from ConnectInfo and checks against the rate limiter.
    /// Returns 429 Too Many Requests via Unauthorized error if the rate limit is exceeded.
    /// Tracks both allowed and blocked requests in metrics.
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
            // Track blocked request
            middleware.metrics.rate_limit.requests_blocked.add(1, &[]);
            return Err(AppError::Unauthorized(
                "Rate limit exceeded. Please try again later.".to_string(),
            ));
        }

        // Track allowed request
        middleware.metrics.rate_limit.requests_allowed.add(1, &[]);

        Ok(next.run(request).await)
    }
}
