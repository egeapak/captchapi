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
    /// The duration is measured in seconds as a floating-point value.
    pub async fn track_request_duration(
        State(middleware): State<MetricsMiddleware>,
        request: Request,
        next: Next,
    ) -> Response {
        let start = Instant::now();

        // Process the request
        let response = next.run(request).await;

        // Calculate duration in seconds
        let duration = start.elapsed().as_secs_f64();

        // Record the duration
        middleware
            .metrics
            .performance
            .request_duration
            .record(duration, &[]);

        response
    }
}
