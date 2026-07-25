//! Tracing setup and, behind the `otel` feature, OpenTelemetry OTLP export.
//!
//! `OTEL_ENABLED` gates export at runtime. The `otel` cargo feature gates it
//! at compile time: without it the OTLP exporter and its HTTP client are not
//! built at all. A binary built without the feature warns if `OTEL_ENABLED`
//! asks for telemetry it cannot provide, rather than ignoring it silently.

#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel")]
use opentelemetry::{global, KeyValue};

#[cfg(feature = "otel")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "otel")]
use opentelemetry_sdk::{
    trace::{
        BatchSpanProcessor, RandomIdGenerator, Sampler, SdkTracerProvider, SimpleSpanProcessor,
        SpanExporter, SpanProcessor, Tracer,
    },
    Resource,
};
#[cfg(feature = "otel")]
use std::sync::{Mutex, OnceLock};
#[cfg(feature = "otel")]
use std::time::Duration;

/// Holds the active SDK tracer provider so it can be shut down on exit.
///
/// In opentelemetry 0.30+ the global `shutdown_tracer_provider` helper was
/// removed; callers must retain the provider and call `.shutdown()` on it.
#[cfg(feature = "otel")]
static PROVIDER: OnceLock<Mutex<Option<SdkTracerProvider>>> = OnceLock::new();

#[cfg(feature = "otel")]
fn provider_slot() -> &'static Mutex<Option<SdkTracerProvider>> {
    PROVIDER.get_or_init(|| Mutex::new(None))
}

/// Configuration for OpenTelemetry telemetry
#[cfg(feature = "otel")]
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub otlp_endpoint: String,
    pub service_name: String,
}

#[cfg(feature = "otel")]
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
#[cfg(feature = "otel")]
#[allow(dead_code)]
pub fn build_tracer_provider<E>(config: &TelemetryConfig, exporter: E) -> SdkTracerProvider
where
    E: SpanExporter + 'static,
{
    build_tracer_provider_with_processor(config, SimpleSpanProcessor::new(exporter))
}

#[cfg(feature = "otel")]
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

/// Does `OTEL_ENABLED` ask for telemetry?
///
/// Returns true if OTEL_ENABLED is set to "true", "1", "yes", or "on"
/// (case-insensitive). This reflects the environment only; whether the
/// exporter was compiled in is a separate question — see
/// [`is_telemetry_enabled`].
pub fn otel_requested() -> bool {
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

/// Should telemetry actually be initialised?
///
/// Requires both that `OTEL_ENABLED` asks for it and that the exporter was
/// compiled in via the `otel` feature.
#[cfg(feature = "otel")]
pub fn is_telemetry_enabled() -> bool {
    otel_requested()
}

/// Always false: this binary was built without the `otel` feature.
///
/// Warns rather than ignoring the request silently, so a deployment that
/// expects traces finds out at startup instead of wondering where they went.
#[cfg(not(feature = "otel"))]
pub fn is_telemetry_enabled() -> bool {
    if otel_requested() {
        // Called before the subscriber is installed, so this cannot use tracing.
        eprintln!(
            "warning: OTEL_ENABLED is set, but this binary was built without the \
             `otel` feature; no traces will be exported. Rebuild with \
             `cargo build --features otel` to enable OpenTelemetry."
        );
    }
    false
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
#[cfg(feature = "otel")]
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
#[cfg(feature = "otel")]
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
    use std::sync::Mutex;

    #[cfg(feature = "otel")]
    use opentelemetry::trace::Tracer;
    #[cfg(feature = "otel")]
    use opentelemetry_sdk::error::OTelSdkResult;
    #[cfg(feature = "otel")]
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
    #[cfg(feature = "otel")]
    use std::sync::Arc;

    /// A capturing exporter wrapper that records the resource passed via set_resource.
    #[cfg(feature = "otel")]
    #[derive(Clone, Debug)]
    struct ResourceCapturingExporter {
        inner: InMemorySpanExporter,
        resource: Arc<Mutex<Resource>>,
    }

    #[cfg(feature = "otel")]
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

    #[cfg(feature = "otel")]
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

    // ── environment isolation ─────────────────────────────────────────────────

    /// Serialises tests that mutate environment variables, and restores what
    /// they found on the way out.
    ///
    /// Environment variables are process-global, so these tests race whenever
    /// the harness runs them as threads in one process — which is what plain
    /// `cargo test` does. `cargo nextest` gives every test its own process and
    /// hides the problem, so CI stays green while `cargo test` fails
    /// intermittently.
    ///
    /// Restoring on drop matters as much as the lock: without it, a test that
    /// panics between setting and clearing a variable leaks it into whichever
    /// test acquires the lock next.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        /// Take the lock, snapshot `keys`, and clear them for a known start state.
        fn new(keys: &[&'static str]) -> Self {
            static LOCK: Mutex<()> = Mutex::new(());

            // A panicking test poisons the mutex. Recover from it so one
            // failure stays local instead of cascading into every other test.
            let lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

            let saved: Vec<_> = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
            for key in keys {
                std::env::remove_var(key);
            }
            Self { _lock: lock, saved }
        }

        fn set(&self, key: &str, value: &str) {
            std::env::set_var(key, value);
        }

        fn remove(&self, key: &str) {
            std::env::remove_var(key);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn test_env_guard_restores_previous_value() {
        std::env::set_var("CAPTCHAPI_ENV_GUARD_PROBE", "original");
        {
            let env = EnvGuard::new(&["CAPTCHAPI_ENV_GUARD_PROBE"]);
            assert!(std::env::var("CAPTCHAPI_ENV_GUARD_PROBE").is_err());
            env.set("CAPTCHAPI_ENV_GUARD_PROBE", "changed");
        }
        assert_eq!(
            std::env::var("CAPTCHAPI_ENV_GUARD_PROBE").as_deref(),
            Ok("original")
        );
        std::env::remove_var("CAPTCHAPI_ENV_GUARD_PROBE");
    }

    #[test]
    fn test_env_guard_clears_variable_that_was_unset() {
        std::env::remove_var("CAPTCHAPI_ENV_GUARD_UNSET");
        {
            let env = EnvGuard::new(&["CAPTCHAPI_ENV_GUARD_UNSET"]);
            env.set("CAPTCHAPI_ENV_GUARD_UNSET", "temporary");
        }
        assert!(std::env::var("CAPTCHAPI_ENV_GUARD_UNSET").is_err());
    }

    // ── otel_requested ──────────────────────────────────────────────────

    #[test]
    fn test_otel_requested_with_true() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.set("OTEL_ENABLED", "true");
        assert!(otel_requested());
    }

    #[test]
    fn test_otel_requested_with_one() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.set("OTEL_ENABLED", "1");
        assert!(otel_requested());
    }

    #[test]
    fn test_otel_requested_with_yes() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.set("OTEL_ENABLED", "yes");
        assert!(otel_requested());
    }

    #[test]
    fn test_otel_requested_with_on() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.set("OTEL_ENABLED", "on");
        assert!(otel_requested());
    }

    #[test]
    fn test_otel_requested_case_insensitive() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);

        env.set("OTEL_ENABLED", "TRUE");
        assert!(otel_requested());

        env.set("OTEL_ENABLED", "Yes");
        assert!(otel_requested());
    }

    #[test]
    fn test_otel_requested_with_false() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.set("OTEL_ENABLED", "false");
        assert!(!otel_requested());
    }

    #[test]
    fn test_otel_requested_with_zero() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.set("OTEL_ENABLED", "0");
        assert!(!otel_requested());
    }

    #[test]
    fn test_otel_requested_with_invalid_value() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.set("OTEL_ENABLED", "invalid");
        assert!(!otel_requested());
    }

    #[test]
    fn test_otel_requested_default_false() {
        let env = EnvGuard::new(&["OTEL_ENABLED"]);
        env.remove("OTEL_ENABLED");
        assert!(!otel_requested());
    }

    // ── TelemetryConfig::from_env ─────────────────────────────────────────────

    #[cfg(feature = "otel")]
    #[test]
    fn test_telemetry_config_from_env_defaults() {
        let _env = EnvGuard::new(&["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_SERVICE_NAME"]);

        let config = TelemetryConfig::from_env();

        assert_eq!(config.otlp_endpoint, "http://localhost:4318");
        assert_eq!(config.service_name, "captchapi");
    }

    #[cfg(feature = "otel")]
    #[test]
    fn test_telemetry_config_from_env_custom() {
        let env = EnvGuard::new(&["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_SERVICE_NAME"]);
        env.set("OTEL_EXPORTER_OTLP_ENDPOINT", "http://otel-collector:4318");
        env.set("OTEL_SERVICE_NAME", "my-service");

        let config = TelemetryConfig::from_env();

        assert_eq!(config.otlp_endpoint, "http://otel-collector:4318");
        assert_eq!(config.service_name, "my-service");
    }

    // ── build_tracer_provider ─────────────────────────────────────────────────

    #[cfg(feature = "otel")]
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

    #[cfg(feature = "otel")]
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

    #[cfg(feature = "otel")]
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

    #[cfg(feature = "otel")]
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

    #[cfg(feature = "otel")]
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

    #[cfg(feature = "otel")]
    #[tokio::test]

    async fn test_shutdown_telemetry_idempotent() {
        // Calling shutdown twice should not panic
        shutdown_telemetry();
        shutdown_telemetry();
    }

    #[cfg(feature = "otel")]
    #[test]
    fn test_shutdown_telemetry_does_not_panic() {
        // Just ensure shutdown doesn't panic when called
        shutdown_telemetry();
    }
}
