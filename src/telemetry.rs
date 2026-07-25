use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{
        BatchSpanProcessor, RandomIdGenerator, Sampler, SdkTracerProvider, SimpleSpanProcessor,
        SpanExporter, SpanProcessor, Tracer,
    },
    Resource,
};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Holds the active SDK tracer provider so it can be shut down on exit.
///
/// In opentelemetry 0.30+ the global `shutdown_tracer_provider` helper was
/// removed; callers must retain the provider and call `.shutdown()` on it.
static PROVIDER: OnceLock<Mutex<Option<SdkTracerProvider>>> = OnceLock::new();

fn provider_slot() -> &'static Mutex<Option<SdkTracerProvider>> {
    PROVIDER.get_or_init(|| Mutex::new(None))
}

/// Configuration for OpenTelemetry telemetry
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub otlp_endpoint: String,
    pub service_name: String,
}

impl TelemetryConfig {
    /// Build a `TelemetryConfig` from the resolved application configuration.
    ///
    /// This is the path used at startup, so telemetry honours `--otel-endpoint` and the TOML
    /// config file rather than only reading the process environment.
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            otlp_endpoint: config.otel_endpoint.clone(),
            service_name: config.otel_service_name.clone(),
        }
    }
}

/// Build a TracerProvider from a config and any SpanExporter (uses simple/synchronous exporter).
///
/// This function is pure — it does not touch global state, making it easy to test.
/// For production use with batching, see `init_telemetry`.
#[allow(dead_code)]
pub fn build_tracer_provider<E>(config: &TelemetryConfig, exporter: E) -> SdkTracerProvider
where
    E: SpanExporter + 'static,
{
    build_tracer_provider_with_processor(config, SimpleSpanProcessor::new(exporter))
}

fn build_tracer_provider_with_processor<P>(
    config: &TelemetryConfig,
    processor: P,
) -> SdkTracerProvider
where
    P: SpanProcessor + 'static,
{
    let resource = Resource::builder_empty()
        .with_attributes([
            KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                config.service_name.clone(),
            ),
            KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
                env!("CARGO_PKG_VERSION"),
            ),
        ])
        .build();

    SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_resource(resource)
        .with_id_generator(RandomIdGenerator::default())
        .with_sampler(Sampler::AlwaysOn)
        .build()
}

/// Initialize OpenTelemetry with OTLP exporter
///
/// This sets up both tracing and metrics exporters that send data to an OTLP-compatible backend
/// (e.g., Jaeger, Grafana Tempo, OpenTelemetry Collector)
///
/// Endpoint and service name come from the resolved [`Config`](crate::config::Config), so they
/// honour the command line and the config file, not just the process environment.
///
/// Returns a Tracer that can be used with tracing-opentelemetry
pub fn init_telemetry(config: &crate::config::Config) -> anyhow::Result<Tracer> {
    let config = TelemetryConfig::from_config(config);

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
    // the provider via the shared build_tracer_provider helper. As of
    // opentelemetry 0.28 the batch processor runs its own background thread and no
    // longer takes an async runtime argument.
    let batch_processor = BatchSpanProcessor::builder(otlp_exporter).build();
    let tracer_provider = build_tracer_provider_with_processor(&config, batch_processor);

    // Get a tracer before moving the provider into global/slot state
    let tracer = tracer_provider.tracer("captchapi");

    // Retain the provider so it can be flushed and shut down on exit, then
    // register it as the global tracer provider.
    *provider_slot().lock().unwrap() = Some(tracer_provider.clone());
    global::set_tracer_provider(tracer_provider);

    tracing::info!("OpenTelemetry initialized successfully");

    Ok(tracer)
}

/// Shutdown OpenTelemetry providers
///
/// This should be called before the application exits to ensure all spans are flushed
pub fn shutdown_telemetry() {
    tracing::info!("Shutting down OpenTelemetry");
    // Take the retained provider (if any) and shut it down. `take()` makes this
    // idempotent: a second call finds an empty slot and is a no-op.
    if let Some(provider) = provider_slot().lock().unwrap().take() {
        if let Err(e) = provider.shutdown() {
            tracing::warn!("Error shutting down tracer provider: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::Tracer;
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
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
                resource: Arc::new(Mutex::new(Resource::builder_empty().build())),
            }
        }

        fn captured_resource(&self) -> Resource {
            self.resource.lock().unwrap().clone()
        }
    }

    impl SpanExporter for ResourceCapturingExporter {
        // As of opentelemetry 0.29, `export` takes `&self` and is an async fn.
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            self.inner.export(batch).await
        }

        fn set_resource(&mut self, resource: &Resource) {
            *self.resource.lock().unwrap() = resource.clone();
            self.inner.set_resource(resource);
        }
    }

    // ── TelemetryConfig::from_config ──────────────────────────────────────────

    #[test]
    fn test_from_config_uses_the_resolved_configuration() {
        // Telemetry settings come from the layered config, so `--otel-endpoint` and the TOML
        // file work — not just the process environment, which the removed `from_env` read.
        let config = crate::config::Config {
            otel_endpoint: "http://collector:4318".to_string(),
            otel_service_name: "my-service".to_string(),
            ..crate::config::Config::for_test()
        };

        let telemetry = TelemetryConfig::from_config(&config);

        assert_eq!(telemetry.otlp_endpoint, "http://collector:4318");
        assert_eq!(telemetry.service_name, "my-service");
    }

    #[test]
    fn test_from_config_carries_the_defaults() {
        let telemetry = TelemetryConfig::from_config(&crate::config::Config::for_test());
        assert_eq!(telemetry.otlp_endpoint, "http://localhost:4318");
        assert_eq!(telemetry.service_name, "captchapi");
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
        provider.force_flush().expect("flush should succeed");

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
        provider.force_flush().expect("flush should succeed");

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
        provider.force_flush().expect("flush should succeed");

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

    // ── provider shutdown / shutdown_telemetry ────────────────────────────────

    #[tokio::test]
    async fn test_shutdown_telemetry_after_init() {
        // Register a provider globally, then shut it down — must not panic.
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
    async fn test_provider_shutdown_succeeds_after_recording() {
        // As of opentelemetry 0.30 the global `shutdown_tracer_provider` helper is
        // gone; the new path is to retain the provider and call `.shutdown()` on it.
        // This test verifies recorded spans reach the exporter and that the new
        // shutdown API succeeds.
        let exporter = InMemorySpanExporter::default();
        let config = TelemetryConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "shutdown-flush".to_string(),
        };

        let provider = build_tracer_provider(&config, exporter.clone());
        let tracer = provider.tracer("test");
        tracer.in_span("flush-on-shutdown", |_cx| {});

        // SimpleSpanProcessor exports on span end, so the span is captured before
        // shutdown (the default InMemorySpanExporter clears itself on shutdown).
        let spans = exporter.get_finished_spans().expect("should get spans");
        assert!(
            spans.iter().any(|s| s.name == "flush-on-shutdown"),
            "span recorded before shutdown should have been exported"
        );

        // The new shutdown API must succeed.
        provider
            .shutdown()
            .expect("provider shutdown should succeed");
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
