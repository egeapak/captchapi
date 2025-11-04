use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::global;
use std::sync::Arc;

/// Metrics for the CaptchAPI application
#[derive(Clone)]
pub struct Metrics {
    // Session metrics
    pub sessions_created: Counter<u64>,
    pub sessions_validated: Counter<u64>,
    pub sessions_deleted: Counter<u64>,
    pub sessions_expired_cleaned: Counter<u64>,
    pub session_validation_attempts: Counter<u64>,

    // API Key metrics
    pub api_keys_created: Counter<u64>,
    pub api_keys_deleted: Counter<u64>,
    pub api_key_authentications: Counter<u64>,

    // Performance metrics
    // TODO: Add histogram recording for these metrics
    #[allow(dead_code)]
    pub captcha_generation_duration: Histogram<f64>,
    #[allow(dead_code)]
    pub request_duration: Histogram<f64>,
}

impl Metrics {
    pub fn new() -> Self {
        let meter = global::meter("captchapi");

        Self {
            // Session metrics
            sessions_created: meter
                .u64_counter("sessions.created")
                .with_description("Total number of CAPTCHA sessions created")
                .build(),

            sessions_validated: meter
                .u64_counter("sessions.validated")
                .with_description("Total number of CAPTCHA sessions validated")
                .build(),

            sessions_deleted: meter
                .u64_counter("sessions.deleted")
                .with_description("Total number of CAPTCHA sessions deleted")
                .build(),

            sessions_expired_cleaned: meter
                .u64_counter("sessions.expired_cleaned")
                .with_description("Total number of expired sessions cleaned up")
                .build(),

            session_validation_attempts: meter
                .u64_counter("sessions.validation_attempts")
                .with_description("Total number of session validation attempts")
                .build(),

            // API Key metrics
            api_keys_created: meter
                .u64_counter("api_keys.created")
                .with_description("Total number of API keys created")
                .build(),

            api_keys_deleted: meter
                .u64_counter("api_keys.deleted")
                .with_description("Total number of API keys deleted")
                .build(),

            api_key_authentications: meter
                .u64_counter("api_keys.authentications")
                .with_description("Total number of API key authentication attempts")
                .build(),

            // Performance metrics
            captcha_generation_duration: meter
                .f64_histogram("captcha.generation.duration")
                .with_description("Duration of CAPTCHA generation in seconds")
                .build(),

            request_duration: meter
                .f64_histogram("http.request.duration")
                .with_description("Duration of HTTP requests in seconds")
                .build(),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to create a shared Metrics instance
pub fn init_metrics() -> Arc<Metrics> {
    Arc::new(Metrics::new())
}
