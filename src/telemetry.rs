use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    export::trace::SpanExporter,
    runtime,
    trace::{
        BatchSpanProcessor, RandomIdGenerator, Sampler, SpanProcessor, Tracer, TracerProvider,
    },
    Resource,
};
use std::time::Duration;

/// Configuration for OpenTelemetry telemetry
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub otlp_endpoint: String,
    pub service_name: String,
}

impl TelemetryConfig {
    /// Build a TelemetryConfig from environment variables, using defaults where absent
    pub fn from_env() -> Self {
        let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:4318".to_string());
        let service_name =
            std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "captchapi".to_string());
        Self {
            otlp_endpoint,
            service_name,
        }
    }
}

/// Build a TracerProvider from a config and any SpanExporter (uses simple/synchronous exporter).
///
/// This function is pure — it does not touch global state, making it easy to test.
/// For production use with batching, see `init_telemetry`.
#[allow(dead_code)]
pub fn build_tracer_provider<E>(config: &TelemetryConfig, exporter: E) -> TracerProvider
where
    E: SpanExporter + 'static,
{
    build_tracer_provider_with_processor(
        config,
        opentelemetry_sdk::trace::SimpleSpanProcessor::new(Box::new(exporter)),
    )
}

fn build_tracer_provider_with_processor<P>(config: &TelemetryConfig, processor: P) -> TracerProvider
where
    P: SpanProcessor + 'static,
{
    let resource = Resource::new(vec![
        KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            config.service_name.clone(),
        ),
        KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
            env!("CARGO_PKG_VERSION"),
        ),
    ]);

    TracerProvider::builder()
        .with_span_processor(processor)
        .with_resource(resource)
        .with_id_generator(RandomIdGenerator::default())
        .with_sampler(Sampler::AlwaysOn)
        .build()
}

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
    let config = TelemetryConfig::from_env();

    tracing::info!(
        "Initializing OpenTelemetry with endpoint: {}",
        config.otlp_endpoint
    );

    // Configure OTLP exporter
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(&config.otlp_endpoint)
        .with_timeout(Duration::from_secs(3))
        .build()?;

    // Wrap the exporter in a batch processor for production performance, then build
    // the provider via the shared build_tracer_provider helper.
    let batch_processor = BatchSpanProcessor::builder(otlp_exporter, runtime::Tokio).build();
    let tracer_provider = build_tracer_provider_with_processor(&config, batch_processor);

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
    use opentelemetry::trace::Tracer;
    use opentelemetry_sdk::export::trace::{ExportResult, SpanData};
    use opentelemetry_sdk::testing::trace::InMemorySpanExporter;
    use opentelemetry_sdk::Resource;
    use std::sync::{Arc, Mutex};

    /// A capturing exporter wrapper that records the resource passed via set_resource.
    #[derive(Clone, Debug)]
    struct ResourceCapturingExporter {
        inner: InMemorySpanExporter,
        resource: Arc<Mutex<Resource>>,
    }

    impl ResourceCapturingExporter {
        fn new() -> Self {
            Self {
                inner: InMemorySpanExporter::default(),
                resource: Arc::new(Mutex::new(Resource::empty())),
            }
        }

        fn captured_resource(&self) -> Resource {
            self.resource.lock().unwrap().clone()
        }
    }

    impl opentelemetry_sdk::export::trace::SpanExporter for ResourceCapturingExporter {
        fn export(
            &mut self,
            batch: Vec<SpanData>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ExportResult> + Send + 'static>>
        {
            self.inner.export(batch)
        }

        fn set_resource(&mut self, resource: &Resource) {
            *self.resource.lock().unwrap() = resource.clone();
            self.inner.set_resource(resource);
        }
    }

    // ── is_telemetry_enabled ──────────────────────────────────────────────────

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

    // ── TelemetryConfig::from_env ─────────────────────────────────────────────

    #[test]

    fn test_telemetry_config_from_env_defaults() {
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("OTEL_SERVICE_NAME");

        let config = TelemetryConfig::from_env();

        assert_eq!(config.otlp_endpoint, "http://localhost:4318");
        assert_eq!(config.service_name, "captchapi");
    }

    #[test]

    fn test_telemetry_config_from_env_custom() {
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://otel-collector:4318");
        std::env::set_var("OTEL_SERVICE_NAME", "my-service");

        let config = TelemetryConfig::from_env();

        assert_eq!(config.otlp_endpoint, "http://otel-collector:4318");
        assert_eq!(config.service_name, "my-service");

        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("OTEL_SERVICE_NAME");
    }

    // ── build_tracer_provider ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_build_tracer_provider_produces_spans() {
        let exporter = InMemorySpanExporter::default();
        let config = TelemetryConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "test-service".to_string(),
        };

        let provider = build_tracer_provider(&config, exporter.clone());
        let tracer = provider.tracer("test");

        // Create a span and finish it
        tracer.in_span("test-span", |_cx| {});

        // Force flush so the in-memory exporter captures the span
        for result in provider.force_flush() {
            result.expect("flush should succeed");
        }

        let spans = exporter.get_finished_spans().expect("should get spans");
        assert!(
            !spans.is_empty(),
            "at least one span should have been recorded"
        );
        assert!(
            spans.iter().any(|s| s.name == "test-span"),
            "the test span should be present"
        );
    }

    #[tokio::test]
    async fn test_build_tracer_provider_sets_service_name() {
        let exporter = ResourceCapturingExporter::new();
        let config = TelemetryConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "my-captcha-service".to_string(),
        };

        let provider = build_tracer_provider(&config, exporter.clone());
        // force_flush triggers set_resource to be propagated to the exporter
        for result in provider.force_flush() {
            result.expect("flush should succeed");
        }

        let resource = exporter.captured_resource();
        let service_name = resource
            .iter()
            .find(|(k, _)| k.as_str() == opentelemetry_semantic_conventions::resource::SERVICE_NAME)
            .map(|(_, v)| v.to_string());

        assert_eq!(
            service_name.as_deref(),
            Some("my-captcha-service"),
            "SERVICE_NAME resource attribute should match config"
        );
    }

    #[tokio::test]
    async fn test_build_tracer_provider_sets_service_version() {
        let exporter = ResourceCapturingExporter::new();
        let config = TelemetryConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "captchapi".to_string(),
        };

        let provider = build_tracer_provider(&config, exporter.clone());
        for result in provider.force_flush() {
            result.expect("flush should succeed");
        }

        let resource = exporter.captured_resource();
        let service_version = resource
            .iter()
            .find(|(k, _)| {
                k.as_str() == opentelemetry_semantic_conventions::resource::SERVICE_VERSION
            })
            .map(|(_, v)| v.to_string());

        assert_eq!(
            service_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
            "SERVICE_VERSION resource attribute should match Cargo package version"
        );
    }

    // ── shutdown_telemetry (global state — must be serial) ────────────────────

    #[tokio::test]

    async fn test_shutdown_telemetry_after_init() {
        // Register a provider globally, then shut it down — must not panic
        let exporter = InMemorySpanExporter::default();
        let config = TelemetryConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "shutdown-test".to_string(),
        };
        let provider = build_tracer_provider(&config, exporter);
        global::set_tracer_provider(provider);

        shutdown_telemetry();
    }

    #[tokio::test]

    async fn test_shutdown_telemetry_idempotent() {
        // Calling shutdown twice should not panic
        shutdown_telemetry();
        shutdown_telemetry();
    }

    #[test]
    fn test_shutdown_telemetry_does_not_panic() {
        // Just ensure shutdown doesn't panic when called
        shutdown_telemetry();
    }
}
