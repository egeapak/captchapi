use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime,
    trace::{RandomIdGenerator, Sampler, Tracer, TracerProvider},
    Resource,
};
use std::time::Duration;

/// Initialize OpenTelemetry with OTLP exporter
///
/// This sets up both tracing and metrics exporters that send data to an OTLP-compatible backend
/// (e.g., Jaeger, Grafana Tempo, OpenTelemetry Collector)
///
/// Configuration via environment variables:
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
