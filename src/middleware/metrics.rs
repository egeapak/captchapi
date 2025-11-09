use crate::metrics::Metrics;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct MetricsMiddleware {
    pub metrics: Arc<Metrics>,
}

impl MetricsMiddleware {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self { metrics }
    }

    /// Middleware function that tracks HTTP request duration
    ///
    /// Records the time taken to process each HTTP request in the `http.request.duration` histogram.
    /// The duration is measured in milliseconds as a floating-point value.
    pub async fn track_request_duration(
        State(middleware): State<MetricsMiddleware>,
        request: Request,
        next: Next,
    ) -> Response {
        let start = Instant::now();

        // Process the request
        let response = next.run(request).await;

        // Calculate duration in milliseconds
        let duration_ms = start.elapsed().as_millis() as f64;

        // Record the duration
        middleware
            .metrics
            .performance
            .request_duration
            .record(duration_ms, &[]);

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::init_metrics;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware as axum_middleware,
        response::IntoResponse,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    async fn dummy_handler() -> impl IntoResponse {
        StatusCode::OK
    }

    #[tokio::test]
    async fn test_metrics_middleware_tracks_request_duration() {
        let metrics = init_metrics();
        let middleware = MetricsMiddleware::new(metrics.clone());

        let app = Router::new().route("/test", get(dummy_handler)).layer(
            axum_middleware::from_fn_with_state(
                middleware.clone(),
                MetricsMiddleware::track_request_duration,
            ),
        );

        // Make a request
        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        // The histogram should have recorded at least one value
        // We can't easily verify the exact value, but we can verify it doesn't panic
    }

    #[tokio::test]
    async fn test_metrics_middleware_new() {
        let metrics = init_metrics();
        let middleware = MetricsMiddleware::new(metrics.clone());

        // Verify we can create the middleware
        assert_eq!(Arc::strong_count(&middleware.metrics), 2); // One in metrics, one in middleware
    }

    #[tokio::test]
    async fn test_metrics_middleware_clone() {
        let metrics = init_metrics();
        let middleware1 = MetricsMiddleware::new(metrics.clone());
        let _middleware2 = middleware1.clone();

        // Both should reference the same metrics
        assert_eq!(Arc::strong_count(&middleware1.metrics), 3); // metrics, middleware1, middleware2
    }

    #[tokio::test]
    async fn test_metrics_middleware_multiple_requests() {
        let metrics = init_metrics();
        let middleware = MetricsMiddleware::new(metrics.clone());

        let app = Router::new().route("/test", get(dummy_handler)).layer(
            axum_middleware::from_fn_with_state(
                middleware.clone(),
                MetricsMiddleware::track_request_duration,
            ),
        );

        // Make multiple requests
        for _ in 0..5 {
            let response = app
                .clone()
                .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }
        // All requests should have been tracked
    }
}
