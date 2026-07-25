use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram};
use std::sync::Arc;

/// Session-related metrics
#[derive(Clone)]
pub struct SessionMetrics {
    pub created: Counter<u64>,
    pub validated: Counter<u64>,
    pub validation_failed: Counter<u64>,
    pub deleted: Counter<u64>,
    pub expired_cleaned: Counter<u64>,
    pub validation_attempts: Counter<u64>,
    pub max_attempts_exceeded: Counter<u64>,
}

/// API key-related metrics
#[derive(Clone)]
pub struct ApiKeyMetrics {
    pub created: Counter<u64>,
    pub deleted: Counter<u64>,
    pub updated: Counter<u64>,
    pub listed: Counter<u64>,
    pub authentications: Counter<u64>,
    pub auth_failures: Counter<u64>,
}

/// Performance metrics (timing/duration)
#[derive(Clone)]
pub struct PerformanceMetrics {
    pub captcha_generation_duration: Histogram<f64>,
    pub request_duration: Histogram<f64>,
}

/// Error tracking metrics
#[derive(Clone)]
pub struct ErrorMetrics {
    pub http_errors_total: Counter<u64>,
    pub database_errors: Counter<u64>,
}

/// Health and system metrics
#[derive(Clone)]
pub struct SystemMetrics {
    /// Successful configuration reloads, from SIGHUP or the admin API.
    pub config_reloads: Counter<u64>,
    /// Reloads rejected because the resolved configuration was invalid.
    pub config_reload_failures: Counter<u64>,
    pub health_checks: Counter<u64>,
}

/// Top-level metrics container for the CaptchAPI application
///
/// Metrics are organized into logical groups for better manageability:
/// - sessions: All session-related operations (create, validate, delete, cleanup)
/// - api_keys: API key management operations
/// - performance: Timing and duration measurements
/// - errors: Error tracking and monitoring
/// - system: General system health metrics
#[derive(Clone)]
pub struct Metrics {
    pub sessions: SessionMetrics,
    pub api_keys: ApiKeyMetrics,
    pub performance: PerformanceMetrics,
    pub errors: ErrorMetrics,
    pub system: SystemMetrics,
}

impl SessionMetrics {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            created: meter
                .u64_counter("sessions.created")
                .with_description("Total number of CAPTCHA sessions created")
                .build(),

            validated: meter
                .u64_counter("sessions.validated")
                .with_description("Total number of CAPTCHA sessions validated successfully")
                .build(),

            validation_failed: meter
                .u64_counter("sessions.validation_failed")
                .with_description("Total number of failed CAPTCHA validation attempts")
                .build(),

            deleted: meter
                .u64_counter("sessions.deleted")
                .with_description("Total number of CAPTCHA sessions deleted")
                .build(),

            expired_cleaned: meter
                .u64_counter("sessions.expired_cleaned")
                .with_description("Total number of expired sessions cleaned up")
                .build(),

            validation_attempts: meter
                .u64_counter("sessions.validation_attempts")
                .with_description("Total number of session validation attempts")
                .build(),

            max_attempts_exceeded: meter
                .u64_counter("sessions.max_attempts_exceeded")
                .with_description("Total number of sessions deleted due to max attempts exceeded")
                .build(),
        }
    }
}

impl ApiKeyMetrics {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            created: meter
                .u64_counter("api_keys.created")
                .with_description("Total number of API keys created")
                .build(),

            deleted: meter
                .u64_counter("api_keys.deleted")
                .with_description("Total number of API keys deleted")
                .build(),

            updated: meter
                .u64_counter("api_keys.updated")
                .with_description("Total number of API keys updated")
                .build(),

            listed: meter
                .u64_counter("api_keys.listed")
                .with_description("Total number of API key list requests")
                .build(),

            authentications: meter
                .u64_counter("api_keys.authentications")
                .with_description("Total number of API key authentication attempts")
                .build(),

            auth_failures: meter
                .u64_counter("api_keys.auth_failures")
                .with_description("Total number of failed API key authentication attempts")
                .build(),
        }
    }
}

impl PerformanceMetrics {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            captcha_generation_duration: meter
                .f64_histogram("captcha.generation.duration")
                .with_description("Duration of CAPTCHA generation in seconds")
                .build(),

            request_duration: meter
                .f64_histogram("http.request.duration")
                .with_description("Duration of HTTP requests in milliseconds")
                .build(),
        }
    }
}

impl ErrorMetrics {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            http_errors_total: meter
                .u64_counter("http.errors.total")
                .with_description("Total number of HTTP errors (4xx and 5xx responses)")
                .build(),

            database_errors: meter
                .u64_counter("database.errors.total")
                .with_description("Total number of database errors")
                .build(),
        }
    }
}

impl SystemMetrics {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            health_checks: meter
                .u64_counter("system.health_checks")
                .with_description("Total number of health check requests")
                .build(),
            config_reloads: meter
                .u64_counter("system.config_reloads")
                .with_description("Total number of successful configuration reloads")
                .build(),
            config_reload_failures: meter
                .u64_counter("system.config_reload_failures")
                .with_description("Total number of configuration reloads rejected as invalid")
                .build(),
        }
    }
}

impl Metrics {
    pub fn new() -> Self {
        let meter = global::meter("captchapi");

        Self {
            sessions: SessionMetrics::new(&meter),
            api_keys: ApiKeyMetrics::new(&meter),
            performance: PerformanceMetrics::new(&meter),
            errors: ErrorMetrics::new(&meter),
            system: SystemMetrics::new(&meter),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_new() {
        let metrics = Metrics::new();
        // Ensure we can create metrics without panicking
        metrics.sessions.created.add(1, &[]);
    }

    #[test]
    fn test_metrics_default() {
        let metrics = Metrics::default();
        // Ensure default implementation works
        metrics.sessions.validated.add(1, &[]);
    }

    #[test]
    fn test_init_metrics() {
        let metrics = init_metrics();
        // Ensure we get an Arc<Metrics>
        assert_eq!(Arc::strong_count(&metrics), 1);
    }

    #[test]
    fn test_metrics_clone() {
        let metrics1 = init_metrics();
        let metrics2 = metrics1.clone();
        // Ensure Arc refcount increases
        assert_eq!(Arc::strong_count(&metrics1), 2);
        assert_eq!(Arc::strong_count(&metrics2), 2);
    }

    #[test]
    fn test_all_session_counters() {
        let metrics = Metrics::new();

        // Test all session-related counters can be incremented
        metrics.sessions.created.add(1, &[]);
        metrics.sessions.validated.add(1, &[]);
        metrics.sessions.validation_failed.add(1, &[]);
        metrics.sessions.deleted.add(1, &[]);
        metrics.sessions.expired_cleaned.add(5, &[]);
        metrics.sessions.validation_attempts.add(1, &[]);
        metrics.sessions.max_attempts_exceeded.add(1, &[]);

        // Test multiple increments work
        metrics.sessions.created.add(10, &[]);
        metrics.sessions.validation_attempts.add(3, &[]);
    }

    #[test]
    fn test_all_api_key_counters() {
        let metrics = Metrics::new();

        // Test all API key-related counters can be incremented
        metrics.api_keys.created.add(1, &[]);
        metrics.api_keys.deleted.add(1, &[]);
        metrics.api_keys.updated.add(1, &[]);
        metrics.api_keys.listed.add(1, &[]);
        metrics.api_keys.authentications.add(1, &[]);
        metrics.api_keys.auth_failures.add(1, &[]);

        // Test multiple increments work
        metrics.api_keys.created.add(5, &[]);
        metrics.api_keys.authentications.add(100, &[]);
    }

    #[test]
    fn test_performance_histograms() {
        let metrics = Metrics::new();

        // Test that histogram recording works
        metrics
            .performance
            .captcha_generation_duration
            .record(0.125, &[]);
        metrics.performance.request_duration.record(0.050, &[]);

        // Test multiple recordings
        metrics
            .performance
            .captcha_generation_duration
            .record(0.200, &[]);
        metrics.performance.request_duration.record(0.100, &[]);
    }

    #[test]
    fn test_error_metrics() {
        let metrics = Metrics::new();

        metrics.errors.http_errors_total.add(1, &[]);
        metrics.errors.database_errors.add(1, &[]);
    }

    #[test]
    fn test_system_metrics() {
        let metrics = Metrics::new();

        metrics.system.health_checks.add(1, &[]);
    }

    #[test]
    fn test_concurrent_counter_increments() {
        let metrics = init_metrics();

        // Simulate concurrent access from multiple "threads" (Arc safety)
        let m1 = metrics.clone();
        let m2 = metrics.clone();
        let m3 = metrics.clone();

        // All clones should be able to increment counters
        m1.sessions.created.add(1, &[]);
        m2.sessions.created.add(1, &[]);
        m3.sessions.created.add(1, &[]);

        m1.api_keys.authentications.add(5, &[]);
        m2.api_keys.authentications.add(10, &[]);
    }

    #[test]
    fn test_counter_with_zero() {
        let metrics = Metrics::new();

        // Test that zero increments don't cause issues
        metrics.sessions.created.add(0, &[]);
        metrics.sessions.expired_cleaned.add(0, &[]);
    }

    #[test]
    fn test_counter_with_large_values() {
        let metrics = Metrics::new();

        // Test that large counter values work
        metrics.sessions.validation_attempts.add(1000, &[]);
        metrics.api_keys.authentications.add(1000000, &[]);
    }

    #[test]
    fn test_all_metrics_accessible() {
        let metrics = Metrics::new();

        // Verify all metric groups are accessible
        // Session counters
        metrics.sessions.created.add(1, &[]);
        metrics.sessions.validated.add(1, &[]);
        metrics.sessions.validation_failed.add(1, &[]);
        metrics.sessions.deleted.add(1, &[]);
        metrics.sessions.expired_cleaned.add(1, &[]);
        metrics.sessions.validation_attempts.add(1, &[]);
        metrics.sessions.max_attempts_exceeded.add(1, &[]);

        // API key counters
        metrics.api_keys.created.add(1, &[]);
        metrics.api_keys.deleted.add(1, &[]);
        metrics.api_keys.updated.add(1, &[]);
        metrics.api_keys.listed.add(1, &[]);
        metrics.api_keys.authentications.add(1, &[]);
        metrics.api_keys.auth_failures.add(1, &[]);

        // Performance histograms
        metrics
            .performance
            .captcha_generation_duration
            .record(0.1, &[]);
        metrics.performance.request_duration.record(0.1, &[]);

        // Error counters
        metrics.errors.http_errors_total.add(1, &[]);
        metrics.errors.database_errors.add(1, &[]);

        // System counters
        metrics.system.health_checks.add(1, &[]);
    }
}
