use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime,
    trace::{RandomIdGenerator, Sampler, Tracer, TracerProvider},
    Resource,
};
use std::time::Duration;

/// Check if OpenTelemetry is enabled via environment variable
///
/// Returns true if OTEL_ENABLED is set to "true", "1", "yes", or "on" (case-insensitive)
pub fn is_telemetry_enabled() -> bool {
    std::env::var("OTEL_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        .parse::<bool>()
        .unwrap_or_else(|_| {
            // If parse fails, check for other common truthy values
            matches!(
                std::env::var("OTEL_ENABLED")
                    .unwrap_or_default()
                    .to_lowercase()
                    .as_str(),
                "yes" | "on" | "1"
            )
        })
}

/// Initialize OpenTelemetry with OTLP exporter
///
/// This sets up both tracing and metrics exporters that send data to an OTLP-compatible backend
/// (e.g., Jaeger, Grafana Tempo, OpenTelemetry Collector)
///
/// Configuration via environment variables:
/// - OTEL_ENABLED: Enable OpenTelemetry (default: false). Set to "true", "1", "yes", or "on"
/// - OTEL_EXPORTER_OTLP_ENDPOINT: The OTLP endpoint (default: http://localhost:4318)
/// - OTEL_SERVICE_NAME: Service name for traces (default: captchapi)
/// - RUST_LOG: Log level filter
///
/// Returns a Tracer that can be used with tracing-opentelemetry
pub fn init_telemetry() -> anyhow::Result<Tracer> {
    // Get configuration from environment
    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4318".to_string());
    let service_name =
        std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "captchapi".to_string());

    tracing::info!(
        "Initializing OpenTelemetry with endpoint: {}",
        otlp_endpoint
    );

    // Create resource with service information
    let resource = Resource::new(vec![
        KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            service_name.clone(),
        ),
        KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
            env!("CARGO_PKG_VERSION"),
        ),
    ]);

    // Configure OTLP exporter
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(&otlp_endpoint)
        .with_timeout(Duration::from_secs(3))
        .build()?;

    // Create tracer provider
    let tracer_provider = TracerProvider::builder()
        .with_batch_exporter(otlp_exporter, runtime::Tokio)
        .with_resource(resource)
        .with_id_generator(RandomIdGenerator::default())
        .with_sampler(Sampler::AlwaysOn)
        .build();

    // Get a tracer before setting the global provider
    let tracer = tracer_provider.tracer("captchapi");

    // Set as global tracer provider
    global::set_tracer_provider(tracer_provider);

    tracing::info!("OpenTelemetry initialized successfully");

    Ok(tracer)
}

/// Shutdown OpenTelemetry providers
///
/// This should be called before the application exits to ensure all spans are flushed
pub fn shutdown_telemetry() {
    tracing::info!("Shutting down OpenTelemetry");
    global::shutdown_tracer_provider();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_telemetry_enabled_with_true() {
        std::env::set_var("OTEL_ENABLED", "true");
        assert!(is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_with_one() {
        std::env::set_var("OTEL_ENABLED", "1");
        assert!(is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_with_yes() {
        std::env::set_var("OTEL_ENABLED", "yes");
        assert!(is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_with_on() {
        std::env::set_var("OTEL_ENABLED", "on");
        assert!(is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_case_insensitive() {
        std::env::set_var("OTEL_ENABLED", "TRUE");
        assert!(is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");

        std::env::set_var("OTEL_ENABLED", "Yes");
        assert!(is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_with_false() {
        std::env::set_var("OTEL_ENABLED", "false");
        assert!(!is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_with_zero() {
        std::env::set_var("OTEL_ENABLED", "0");
        assert!(!is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_with_invalid_value() {
        std::env::set_var("OTEL_ENABLED", "invalid");
        assert!(!is_telemetry_enabled());
        std::env::remove_var("OTEL_ENABLED");
    }

    #[test]
    fn test_is_telemetry_enabled_default_false() {
        std::env::remove_var("OTEL_ENABLED");
        assert!(!is_telemetry_enabled());
    }

    #[test]
    fn test_shutdown_telemetry_does_not_panic() {
        // Just ensure shutdown doesn't panic when called
        shutdown_telemetry();
    }
}
