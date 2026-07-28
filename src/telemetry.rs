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
use tracing_subscriber::{layer::SubscriberExt, reload, util::SubscriberInitExt, Registry};

/// Swaps the running log filter.
///
/// The subscriber can only be installed once, and it is installed before the database — and so
/// before any stored configuration — is available. Rather than delay it and lose the logs from
/// config resolution and migration, which are the ones most worth having when boot fails, the
/// filter goes in behind a [`reload::Layer`] and is replaced once the final configuration is
/// known. That is also what lets `log_level` be a live field: a reload or a `PATCH` reaches the
/// running filter through this handle.
///
/// Cheap to clone; the underlying handle is an `Arc`.
#[derive(Clone)]
pub struct LogFilterHandle(reload::Handle<Targets, Registry>);

impl LogFilterHandle {
    /// Replace the running filter with one parsed from `directives`.
    ///
    /// Unparseable directives leave the current filter in place. `log_filter` would otherwise
    /// substitute its built-in default, which for a *running* server means a bad edit silently
    /// changes what is logged instead of being ignored.
    pub fn apply(&self, directives: &str) -> Result<(), String> {
        let filter: Targets = directives
            .parse()
            .map_err(|e| format!("unparseable log filter {directives:?}: {e}"))?;
        self.0
            .modify(|current| *current = filter)
            .map_err(|e| format!("log filter handle is no longer live: {e}"))
    }
}

impl std::fmt::Debug for LogFilterHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogFilterHandle")
    }
}

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
fn init_plain_tracing(config: &Config) -> LogFilterHandle {
    let (filter, handle) = reload::Layer::new(log_filter(&config.log_level));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
    LogFilterHandle(handle)
}

/// Install the tracing subscriber, adding the OTLP export layer when the
/// `otel` feature is compiled in and the configuration asks for it.
#[cfg(feature = "otel")]
pub fn init_tracing(config: &Config, otel_enabled: bool) -> anyhow::Result<LogFilterHandle> {
    let handle = if otel_enabled {
        let tracer = init_telemetry(config)?;
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        let (filter, handle) = reload::Layer::new(log_filter(&config.log_level));

        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .with(telemetry_layer)
            .init();

        tracing::info!("OpenTelemetry enabled");
        LogFilterHandle(handle)
    } else {
        let handle = init_plain_tracing(config);
        tracing::info!("OpenTelemetry disabled");
        handle
    };
    Ok(handle)
}

/// Without the `otel` feature there is no exporter to install.
///
/// `is_telemetry_enabled` has already warned on stderr if the configuration
/// asked for telemetry, so this only records the build configuration.
#[cfg(not(feature = "otel"))]
pub fn init_tracing(config: &Config, _otel_enabled: bool) -> anyhow::Result<LogFilterHandle> {
    let handle = init_plain_tracing(config);
    tracing::info!("OpenTelemetry not compiled in (rebuild with --features otel)");
    Ok(handle)
}

#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel")]
use opentelemetry::{global, KeyValue};

#[cfg(feature = "otel")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "otel")]
use opentelemetry_sdk::{
    metrics::{exporter::PushMetricExporter, PeriodicReader, SdkMeterProvider},
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

/// Holds the active SDK meter provider, for the same reason as [`PROVIDER`].
///
/// Metrics are pushed on an interval rather than per-event, so without an
/// explicit shutdown the final period is simply lost — the process exits
/// between ticks and the last batch never leaves.
#[cfg(feature = "otel")]
static METER: OnceLock<Mutex<Option<SdkMeterProvider>>> = OnceLock::new();

#[cfg(feature = "otel")]
fn meter_slot() -> &'static Mutex<Option<SdkMeterProvider>> {
    METER.get_or_init(|| Mutex::new(None))
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

/// The resource attributes both signals are tagged with.
///
/// Shared deliberately: traces and metrics only line up in a backend if they
/// carry identical `service.name` and `service.version`, and duplicating the
/// construction is how they drift apart.
#[cfg(feature = "otel")]
fn build_resource(config: &TelemetryConfig) -> Resource {
    Resource::builder_empty()
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
        .build()
}

/// Build a MeterProvider from a config and any push exporter.
///
/// Takes the exporter rather than a reader, mirroring [`build_tracer_provider`]
/// — `MetricReader` is only public behind an experimental feature, and wrapping
/// in a `PeriodicReader` here keeps the collection strategy in one place.
///
/// Pure: it touches no global state, so a test can drive it with an in-memory
/// exporter and assert on what actually came out.
#[cfg(feature = "otel")]
pub fn build_meter_provider<E>(config: &TelemetryConfig, exporter: E) -> SdkMeterProvider
where
    E: PushMetricExporter,
{
    SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .with_resource(build_resource(config))
        .build()
}

#[cfg(feature = "otel")]
fn build_tracer_provider_with_processor<P>(
    config: &TelemetryConfig,
    processor: P,
) -> SdkTracerProvider
where
    P: SpanProcessor + 'static,
{
    let resource = build_resource(config);

    SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_resource(resource)
        .with_id_generator(RandomIdGenerator::default())
        .with_sampler(Sampler::AlwaysOn)
        .build()
}

/// Append a signal path to a base OTLP endpoint.
///
/// **`with_endpoint` takes a complete URL, not a base.** The SDK only appends
/// `/v1/metrics` or `/v1/traces` when it reads the endpoint from the
/// environment itself; an endpoint passed to the builder is used verbatim (see
/// `test_not_append_signal_path_to_signal_env` upstream). Passing
/// `http://collector:4318` therefore POSTs every payload to `/`, which a
/// collector answers with **404** — so telemetry was configured, connected,
/// and silently rejected. Nothing in the service surfaced it: the export error
/// only appears in the SDK's own `opentelemetry-otlp` debug logs, which the
/// default filter excludes.
///
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is conventionally a *base* — that is what
/// every collector's documentation shows, and what `.env.example` documents —
/// so the base form is what this accepts. An endpoint that already carries the
/// signal path is passed through unchanged, so an operator who writes the full
/// URL is not punished for it.
#[cfg(feature = "otel")]
fn signal_endpoint(base: &str, signal_path: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(signal_path) {
        return trimmed.to_string();
    }
    format!("{trimmed}{signal_path}")
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

/// Initialize OpenTelemetry with OTLP exporters for traces and metrics.
///
/// Both signals are pushed over OTLP/HTTP to an OTLP-compatible backend
/// (e.g. Jaeger, Grafana Tempo, an OpenTelemetry Collector). Nothing scrapes
/// this service — there is no metrics endpoint to poll.
///
/// **Call this before any instrument is created.** `global::meter()` binds to
/// whichever provider is installed at the moment it is called, so a `Meter`
/// obtained before this function runs stays attached to the default no-op
/// provider and silently discards everything recorded through it. `main`
/// therefore calls `init_tracing` (which lands here) well before
/// `init_metrics`. That ordering is not incidental — it is the whole reason
/// the counters in `crate::metrics` reach a collector at all.
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
        .with_endpoint(signal_endpoint(&config.otlp_endpoint, "/v1/traces"))
        .with_timeout(Duration::from_secs(3))
        .build()?;

    // Wrap the exporter in a batch processor for production performance, then build
    // the provider via the shared build_tracer_provider helper. As of
    // opentelemetry 0.28 the batch processor runs its own background thread and no
    // longer takes an async runtime argument.
    //
    // That thread has no Tokio reactor, which is why the exporter must be built on
    // a *blocking* HTTP client — see the `reqwest-blocking-client` note in
    // Cargo.toml. Switching back to the async client compiles, starts, logs
    // "OpenTelemetry initialized successfully", and then aborts the process on the
    // first export tick.
    let batch_processor = BatchSpanProcessor::builder(otlp_exporter).build();
    let tracer_provider = build_tracer_provider_with_processor(&config, batch_processor);

    // Get a tracer before moving the provider into global/slot state
    let tracer = tracer_provider.tracer("captchapi");

    // Retain the provider so it can be flushed and shut down on exit, then
    // register it as the global tracer provider.
    *provider_slot().lock().unwrap() = Some(tracer_provider.clone());
    global::set_tracer_provider(tracer_provider);

    // Metrics travel the same transport to the same endpoint. `PeriodicReader`
    // collects on its own background interval, so nothing here is on a request
    // path; the cost of an instrument at runtime stays an atomic add.
    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(signal_endpoint(&config.otlp_endpoint, "/v1/metrics"))
        .with_timeout(Duration::from_secs(3))
        .build()?;

    let meter_provider = build_meter_provider(&config, metric_exporter);

    *meter_slot().lock().unwrap() = Some(meter_provider.clone());
    global::set_meter_provider(meter_provider);

    tracing::info!("OpenTelemetry initialized successfully (traces and metrics)");

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

    // The meter provider flushes on shutdown. Skipping this loses everything
    // recorded since the last periodic tick, which for a short-lived process
    // can be every metric it ever produced.
    if let Some(provider) = meter_slot().lock().unwrap().take() {
        if let Err(e) = provider.shutdown() {
            tracing::warn!("Error shutting down meter provider: {e}");
        }
    }
}

#[cfg(test)]
mod metrics_export_tests {
    #[cfg(feature = "otel")]
    use super::*;

    /// The regression this pipeline exists to prevent.
    ///
    /// Before it was wired up, `crate::metrics` built every counter on
    /// `global::meter()` while nothing ever installed a `MeterProvider`, so the
    /// global default — a no-op — swallowed every measurement. The code looked
    /// instrumented, compiled, ran, and emitted nothing. The old
    /// `test_metrics_new` asserted only that incrementing "does not panic",
    /// which a no-op satisfies perfectly.
    ///
    /// This drives a provider built exactly the way `init_telemetry` builds it
    /// and asserts a recorded value comes out the far end.
    #[cfg(feature = "otel")]
    #[test]
    fn test_a_recorded_instrument_reaches_the_exporter() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::InMemoryMetricExporter;

        let exporter = InMemoryMetricExporter::default();
        let config = TelemetryConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "captchapi-test".to_string(),
        };
        let provider = build_meter_provider(&config, exporter.clone());

        let counter = provider
            .meter("captchapi")
            .u64_counter("sessions.created")
            .build();
        counter.add(7, &[]);

        provider.force_flush().expect("flush succeeds");

        let exported = exporter.get_finished_metrics().expect("metrics readable");
        assert!(
            !exported.is_empty(),
            "nothing was exported; the meter provider is not collecting"
        );

        let names: Vec<String> = exported
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .map(|m| m.name().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n == "sessions.created"),
            "recorded instrument missing from export; got {names:?}"
        );
    }

    /// A real OTLP exporter must survive being driven from a background thread.
    ///
    /// Every other test here uses an in-memory exporter, which never performs I/O
    /// — so none of them can see the failure this one exists for. The OTLP
    /// exporter built with the *async* reqwest client panics the moment it is
    /// used from `PeriodicReader`'s dedicated thread, which carries no Tokio
    /// reactor:
    ///
    ///     there is no reactor running, must be called from the context of a Tokio 1.x runtime
    ///
    /// That panic aborted the process seconds after startup, so `OTEL_ENABLED=true`
    /// shipped as a crash loop in v2.0.0-rc.1 while every test passed.
    ///
    /// No collector runs during tests, so the export *fails* — that is fine and is
    /// not what is under test. A connection error is a returned `Err`; the
    /// regression is a panic that unwinds the exporting thread.
    ///
    /// **What makes this observable is subtle enough to be worth spelling out.**
    /// `PeriodicReader` owns a private thread and `force_flush` merely *messages*
    /// it; the export runs over there. So a panic is contained to that thread —
    /// `join`ing a thread of one's own, or catching unwind around the flush,
    /// sees nothing. Two earlier versions of this test did exactly that and
    /// passed against the broken configuration.
    ///
    /// The signal that does survive: once the reader's thread has died, it is no
    /// longer receiving, so a subsequent `force_flush` cannot succeed. With a
    /// working exporter the flush is *attempted* — it returns `Err` here because
    /// nothing is listening on port 1 — but the thread stays alive and keeps
    /// answering. The test therefore flushes twice and asserts the reader is
    /// still there for the second one.
    ///
    /// Note also that `panic = "abort"` in the release profile means this panic
    /// is only ever *contained* in a test build. In the shipped binary it took
    /// the whole process down.
    #[cfg(feature = "otel")]
    #[test]
    fn test_otlp_exporter_survives_export_from_the_readers_own_thread() {
        let config = TelemetryConfig {
            // Port 1 is reserved and nothing listens there, so the export fails
            // fast instead of hanging on a real endpoint.
            otlp_endpoint: "http://127.0.0.1:1".to_string(),
            service_name: "captchapi-reactor-test".to_string(),
        };

        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_endpoint(&config.otlp_endpoint)
            .with_timeout(Duration::from_millis(200))
            .build()
            .expect("exporter builds");

        let provider = build_meter_provider(&config, exporter);

        use opentelemetry::metrics::MeterProvider as _;
        let counter = provider
            .meter("captchapi")
            .u64_counter("reactor.probe")
            .build();

        // First export: this is what panicked the reader thread under the async
        // client. The returned value is not the assertion — connection refused
        // is expected — the point is what state the reader is left in.
        counter.add(1, &[]);
        let _ = provider.force_flush();

        // Second export: only reachable if the reader thread is still alive.
        counter.add(1, &[]);
        let second = provider.force_flush();

        // Both outcomes are an `Err` here and the distinction is in the message,
        // which is the only thing the SDK exposes:
        //
        //   dead thread  -> "sending on a closed channel"   (the regression)
        //   live thread  -> "Failed to flush"               (connection refused,
        //                                                    which is expected —
        //                                                    nothing listens on
        //                                                    port 1)
        //
        // Matching on the string is unlovely, but the alternative is asserting
        // `is_ok()`, which would demand a live collector in unit tests.
        let detail = format!("{second:?}");
        assert!(
            !detail.contains("closed channel"),
            "the PeriodicReader thread did not survive the first export, so the \
             exporter panicked on it — the OTLP exporter must be built on a \
             blocking HTTP client (see `reqwest-blocking-client` in Cargo.toml). \
             Got: {detail}"
        );

        provider.shutdown().ok();
    }

    /// The signal path must reach the URL, or every export is a 404.
    ///
    /// `with_endpoint` is verbatim — it does not append `/v1/metrics` the way
    /// the SDK's own environment handling does. Posting a configured
    /// `http://collector:4318` straight to the builder sent everything to `/`,
    /// which collectors reject with 404, and the only trace of it was in the
    /// SDK's internal debug logs. v2.0.0-rc.1 shipped that way.
    #[cfg(feature = "otel")]
    #[test]
    fn test_signal_endpoint_appends_the_signal_path_to_a_base() {
        assert_eq!(
            signal_endpoint("http://collector:4318", "/v1/metrics"),
            "http://collector:4318/v1/metrics"
        );
        assert_eq!(
            signal_endpoint("http://collector:4318", "/v1/traces"),
            "http://collector:4318/v1/traces"
        );
    }

    /// A trailing slash is the commonest way to write a base URL and must not
    /// produce a double slash, which some collectors route differently.
    #[cfg(feature = "otel")]
    #[test]
    fn test_signal_endpoint_normalises_a_trailing_slash() {
        assert_eq!(
            signal_endpoint("http://collector:4318/", "/v1/metrics"),
            "http://collector:4318/v1/metrics"
        );
    }

    /// An operator who already wrote the full URL must not get it twice.
    #[cfg(feature = "otel")]
    #[test]
    fn test_signal_endpoint_leaves_a_complete_url_alone() {
        assert_eq!(
            signal_endpoint("http://collector:4318/v1/metrics", "/v1/metrics"),
            "http://collector:4318/v1/metrics"
        );
        assert_eq!(
            signal_endpoint("http://collector:4318/v1/metrics/", "/v1/metrics"),
            "http://collector:4318/v1/metrics"
        );
    }

    /// The two signals must not collide: a traces path is not a metrics path.
    #[cfg(feature = "otel")]
    #[test]
    fn test_signal_endpoint_keeps_the_two_signals_distinct() {
        let base = "http://collector:4318";
        assert_ne!(
            signal_endpoint(base, "/v1/metrics"),
            signal_endpoint(base, "/v1/traces")
        );
    }

    /// Traces and metrics only correlate in a backend if they agree on who
    /// emitted them, so both providers are built from one `build_resource`.
    #[cfg(feature = "otel")]
    #[test]
    fn test_metrics_carry_the_same_service_identity_as_traces() {
        use opentelemetry_sdk::metrics::InMemoryMetricExporter;

        let config = TelemetryConfig {
            otlp_endpoint: "http://localhost:4318".to_string(),
            service_name: "captchapi-identity".to_string(),
        };
        let exporter = InMemoryMetricExporter::default();
        let provider = build_meter_provider(&config, exporter.clone());
        provider.force_flush().expect("flush succeeds");

        let resource = build_resource(&config);
        let service_name = resource
            .get(&opentelemetry::Key::from_static_str(
                opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            ))
            .map(|v| v.to_string());
        assert_eq!(service_name.as_deref(), Some("captchapi-identity"));
        assert!(
            resource
                .get(&opentelemetry::Key::from_static_str(
                    opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
                ))
                .is_some(),
            "service.version must be present so builds are distinguishable"
        );
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    /// A live handle over a filter that is *not* installed globally.
    ///
    /// The subscriber can only be installed once per process, and other tests in this binary
    /// need it, so these exercise the handle against a free-standing reload layer. The layer
    /// must outlive the handle or `modify` fails, which is why it is returned alongside.
    fn detached_handle(
        initial: &str,
    ) -> (
        reload::Layer<Targets, Registry>,
        LogFilterHandle,
        reload::Handle<Targets, Registry>,
    ) {
        let (layer, handle) = reload::Layer::new(log_filter(initial));
        (layer, LogFilterHandle(handle.clone()), handle)
    }

    #[test]
    fn test_apply_replaces_the_running_filter() {
        let (_layer, filter, raw) = detached_handle("captchapi=info");

        filter.apply("captchapi=trace").expect("valid directives");

        // `Targets` has no field accessors, so compare its rendering — which round-trips.
        assert_eq!(raw.clone_current().unwrap().to_string(), "captchapi=trace");
    }

    #[test]
    fn test_apply_rejects_bad_directives_and_keeps_the_old_filter() {
        // `log_filter` substitutes its built-in default for unparseable input, which is right
        // at startup and wrong for a running server: a bad edit would silently change what is
        // logged instead of being ignored. `apply` parses first and refuses.
        let (_layer, filter, raw) = detached_handle("captchapi=info");

        let err = filter.apply("=:=nonsense=:=").unwrap_err();

        assert!(err.contains("unparseable log filter"), "{err}");
        assert_eq!(raw.clone_current().unwrap().to_string(), "captchapi=info");
    }

    #[test]
    fn test_apply_reports_a_dead_handle_rather_than_panicking() {
        let (layer, filter, _raw) = detached_handle("captchapi=info");
        drop(layer);

        assert!(filter.apply("captchapi=trace").is_err());
    }

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
