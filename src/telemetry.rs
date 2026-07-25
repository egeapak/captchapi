//! Tracing setup and, behind the `otel` feature, OpenTelemetry OTLP export.
//!
//! `config.otel_enabled` gates export at runtime. The `otel` cargo feature
//! gates it at compile time: without it the OTLP exporter and its HTTP client
//! are not built at all. A binary built without the feature warns if the
//! configuration asks for telemetry it cannot provide, rather than ignoring
//! the request silently.

use crate::config::Config;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Build the log filter from `RUST_LOG`-style directives.
///
/// Uses `Targets` rather than `EnvFilter`: `EnvFilter` pulls in the `regex`
/// engine (~177 KiB of the binary) to support per-span field matching, which
/// this service never uses. `Targets` understands the same directive forms
/// that matter here — a bare level (`debug`), a `target=level` pair, and
/// comma-separated lists of them — with no regex.
///
/// The directives come from `config.log_level`, so `--log-level` and the TOML
/// file are honoured, not just `RUST_LOG`.
pub fn log_filter(directives: &str) -> Targets {
    let default = || {
        Targets::new()
            .with_target("captchapi", Level::DEBUG)
            .with_target("tower_http", Level::DEBUG)
    };

    directives.parse().unwrap_or_else(|e| {
        // The subscriber is not up yet, so this cannot go through tracing.
        eprintln!("warning: ignoring unparseable log filter ({directives:?}): {e}");
        default()
    })
}

/// Install the fmt subscriber with no OpenTelemetry layer.
fn init_plain_tracing(config: &Config) {
    tracing_subscriber::registry()
        .with(log_filter(&config.log_level))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Install the tracing subscriber, adding the OTLP export layer when the
/// `otel` feature is compiled in and the configuration asks for it.
#[cfg(feature = "otel")]
pub fn init_tracing(config: &Config, otel_enabled: bool) -> anyhow::Result<()> {
    if otel_enabled {
        let tracer = init_telemetry(config)?;
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        tracing_subscriber::registry()
            .with(log_filter(&config.log_level))
            .with(tracing_subscriber::fmt::layer())
            .with(telemetry_layer)
            .init();

        tracing::info!("OpenTelemetry enabled");
    } else {
        init_plain_tracing(config);
        tracing::info!("OpenTelemetry disabled");
    }
    Ok(())
}

/// Without the `otel` feature there is no exporter to install.
///
/// `is_telemetry_enabled` has already warned on stderr if the configuration
/// asked for telemetry, so this only records the build configuration.
#[cfg(not(feature = "otel"))]
pub fn init_tracing(config: &Config, _otel_enabled: bool) -> anyhow::Result<()> {
    init_plain_tracing(config);
    tracing::info!("OpenTelemetry not compiled in (rebuild with --features otel)");
    Ok(())
}

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
    /// Build a `TelemetryConfig` from the resolved application configuration.
    ///
    /// This is the path used at startup, so telemetry honours `--otel-endpoint` and the TOML
    /// config file rather than only reading the process environment.
    pub fn from_config(config: &Config) -> Self {
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

/// Should telemetry actually be initialised?
///
/// `config.otel_enabled` says whether the operator asked for it; this adds the
/// compile-time question of whether the exporter was built in at all.
#[cfg(feature = "otel")]
pub fn is_telemetry_enabled(config: &Config) -> bool {
    config.otel_enabled
}

/// Always false: this binary was built without the `otel` feature.
///
/// Warns rather than ignoring the request silently, so a deployment that
/// expects traces finds out at startup instead of wondering where they went.
#[cfg(not(feature = "otel"))]
pub fn is_telemetry_enabled(config: &Config) -> bool {
    if config.otel_enabled {
        // Called before the subscriber is installed, so this cannot use tracing.
        eprintln!(
            "warning: OpenTelemetry is enabled in the configuration, but this \
             binary was built without the `otel` feature; no traces will be \
             exported. Rebuild with `cargo build --features otel` to enable it."
        );
    }
    false
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
#[cfg(feature = "otel")]
pub fn init_telemetry(config: &Config) -> anyhow::Result<Tracer> {
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
    #[allow(unused_imports)]
    use super::*;

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

    // ── TelemetryConfig::from_config ──────────────────────────────────────────

    #[cfg(feature = "otel")]
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

    #[cfg(feature = "otel")]
    #[test]
    fn test_from_config_carries_the_defaults() {
        let telemetry = TelemetryConfig::from_config(&crate::config::Config::for_test());
        assert_eq!(telemetry.otlp_endpoint, "http://localhost:4318");
        assert_eq!(telemetry.service_name, "captchapi");
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

    // ── log_filter ────────────────────────────────────────────────────────────
    //
    // Not gated on `otel`: `log_filter` is how every build decides what to log.

    /// Would a `target` event at `level` pass the filter?
    fn passes(filter: &Targets, target: &str, level: Level) -> bool {
        filter.would_enable(target, &level)
    }

    /// The compiled-in fallback and the default declared in `PARAMS` must agree.
    ///
    /// They are written out separately — one as `Targets` in `log_filter`, one as
    /// a string in the parameter table — so nothing but this test stops them from
    /// drifting apart. If they drift, the fallback silently starts logging
    /// differently from a default boot.
    #[test]
    fn test_log_filter_fallback_matches_the_declared_default() {
        let declared = crate::config::params::PARAMS
            .iter()
            .find(|p| p.field == "log_level")
            .and_then(|p| p.default)
            .expect("log_level should declare a default");

        let from_default = log_filter(declared);
        // A bad *level* is the one form that actually fails to parse, so it is
        // what reaches the fallback. A bare unknown word does not: see
        // `test_log_filter_bare_word_is_a_target_not_an_error`.
        let from_fallback = log_filter("captchapi=notalevel");

        for target in ["captchapi", "tower_http", "some_noisy_crate"] {
            for level in [Level::TRACE, Level::DEBUG, Level::INFO, Level::WARN] {
                assert_eq!(
                    passes(&from_default, target, level),
                    passes(&from_fallback, target, level),
                    "declared default {declared:?} and the code fallback disagree \
                     on {target} at {level}"
                );
            }
        }
    }

    #[test]
    fn test_log_filter_default_enables_captchapi_debug() {
        let filter = log_filter("captchapi=debug,tower_http=debug");
        assert!(passes(&filter, "captchapi", Level::DEBUG));
        assert!(passes(&filter, "captchapi", Level::INFO));
        assert!(passes(&filter, "tower_http", Level::DEBUG));
        assert!(!passes(&filter, "captchapi", Level::TRACE));
        assert!(!passes(&filter, "some_noisy_crate", Level::INFO));
    }

    #[test]
    fn test_log_filter_accepts_target_level_pairs() {
        let filter = log_filter("captchapi=warn");
        assert!(passes(&filter, "captchapi", Level::WARN));
        assert!(!passes(&filter, "captchapi", Level::INFO));
    }

    #[test]
    fn test_log_filter_accepts_comma_separated_directives() {
        let filter = log_filter("captchapi=info,tower_http=warn");
        assert!(passes(&filter, "captchapi", Level::INFO));
        assert!(!passes(&filter, "captchapi", Level::DEBUG));
        assert!(passes(&filter, "tower_http", Level::WARN));
        assert!(!passes(&filter, "tower_http", Level::INFO));
    }

    /// `RUST_LOG=debug` is the most common form; it must stay a global level
    /// rather than being read as a target named "debug".
    #[test]
    fn test_log_filter_accepts_a_bare_level_as_global() {
        for (directive, level) in [
            ("trace", Level::TRACE),
            ("debug", Level::DEBUG),
            ("info", Level::INFO),
            ("warn", Level::WARN),
            ("error", Level::ERROR),
        ] {
            let filter = log_filter(directive);
            assert!(
                passes(&filter, "any_crate_at_all", level),
                "{directive:?} should enable {level} globally"
            );
        }
        assert!(!passes(&log_filter("off"), "captchapi", Level::ERROR));
    }

    #[test]
    fn test_log_filter_falls_back_when_the_level_is_invalid() {
        let filter = log_filter("captchapi=notalevel");
        // Fell back to the default, which enables captchapi at DEBUG.
        assert!(passes(&filter, "captchapi", Level::DEBUG));
        assert!(!passes(&filter, "some_noisy_crate", Level::INFO));
    }

    /// A bare word is a *target* directive, not a malformed level.
    ///
    /// This is standard `RUST_LOG` syntax and matches what `EnvFilter` did
    /// before `Targets` replaced it, so it is pinned rather than fixed. The
    /// consequence is worth knowing: `RUST_LOG=debg` parses cleanly as "the
    /// target `debg` at TRACE" and takes the whole service silent, without
    /// reaching the fallback. A typo'd level is quiet, not loud.
    #[test]
    fn test_log_filter_bare_word_is_a_target_not_an_error() {
        // A real target name enables that target at TRACE.
        let filter = log_filter("captchapi");
        assert!(passes(&filter, "captchapi", Level::TRACE));
        assert!(passes(&filter, "captchapi", Level::ERROR));

        // A misspelt level is read the same way — as a target nothing logs to.
        let typo = log_filter("debg");
        assert!(!passes(&typo, "captchapi", Level::ERROR));
    }

    /// An empty filter still lets errors through, as `EnvFilter` did.
    #[test]
    fn test_log_filter_empty_keeps_errors_globally() {
        let filter = log_filter("");
        assert!(passes(&filter, "any_crate_at_all", Level::ERROR));
        assert!(!passes(&filter, "captchapi", Level::DEBUG));
    }
}
