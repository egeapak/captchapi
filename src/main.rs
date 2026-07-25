mod app;
mod config;
mod error;
mod metrics;
mod middleware;
mod models;
mod routes;
mod services;
mod tasks;
mod telemetry;
mod validation;

use crate::app::build_app;
use crate::config::Config;
use crate::metrics::init_metrics;
use crate::tasks::start_cleanup_task;
use crate::telemetry::{init_telemetry, shutdown_telemetry};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
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
fn log_filter(directives: Option<&str>) -> Targets {
    let default = || {
        Targets::new()
            .with_target("captchapi", Level::DEBUG)
            .with_target("tower_http", Level::DEBUG)
    };

    match directives {
        Some(directives) => directives.parse().unwrap_or_else(|e| {
            // The subscriber is not up yet, so this cannot go through tracing.
            eprintln!("warning: ignoring unparseable RUST_LOG ({directives:?}): {e}");
            default()
        }),
        None => default(),
    }
}

/// Read `RUST_LOG` and turn it into a filter.
fn log_filter_from_env() -> Targets {
    log_filter(std::env::var("RUST_LOG").ok().as_deref())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load configuration from environment first (needed for telemetry config)
    dotenvy::dotenv().ok();

    // Check if OpenTelemetry is enabled
    let otel_enabled = crate::telemetry::is_telemetry_enabled();

    // Initialize tracing with conditional OpenTelemetry support
    if otel_enabled {
        // Initialize OpenTelemetry and get tracer
        let tracer = init_telemetry()?;

        // Create OpenTelemetry tracing layer
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        // Initialize tracing with OpenTelemetry
        tracing_subscriber::registry()
            .with(log_filter_from_env())
            .with(tracing_subscriber::fmt::layer())
            .with(telemetry_layer)
            .init();

        tracing::info!("OpenTelemetry enabled");
    } else {
        // Initialize tracing without OpenTelemetry
        tracing_subscriber::registry()
            .with(log_filter_from_env())
            .with(tracing_subscriber::fmt::layer())
            .init();

        tracing::info!("OpenTelemetry disabled");
    }

    let config = Arc::new(Config::from_env().map_err(|e| anyhow::anyhow!(e))?);

    tracing::info!("Starting CaptchAPI server");
    tracing::info!(
        "Server configuration: {}:{}",
        config.server_host,
        config.server_port
    );

    // Create data directory if it doesn't exist
    if config.database_url.starts_with("sqlite:") {
        let db_path = config
            .database_url
            .strip_prefix("sqlite:")
            .ok_or_else(|| anyhow::anyhow!("DATABASE_URL must start with 'sqlite:'"))?;
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Set up database connection pool
    let pool = SqlitePoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(
                    config
                        .database_url
                        .strip_prefix("sqlite:")
                        .ok_or_else(|| anyhow::anyhow!("DATABASE_URL must start with 'sqlite:'"))?,
                )
                .create_if_missing(true),
        )
        .await?;

    tracing::info!("Database connection established");

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("Database migrations completed");

    // Initialize metrics
    let metrics = init_metrics();
    tracing::info!("Metrics initialized");

    // Build application (services, middleware, router, rate limiter)
    let components = build_app(pool, config.clone(), metrics.clone());

    // Create shutdown token for graceful shutdown
    let shutdown_token = CancellationToken::new();

    // Start cleanup task
    let cleanup_handle = start_cleanup_task(
        components.storage,
        config.cleanup_interval_seconds,
        metrics.clone(),
        components.governor_limiter,
        shutdown_token.clone(),
    );
    tracing::info!(
        "Background cleanup task started (interval: {}s)",
        config.cleanup_interval_seconds
    );

    // Start server
    let listener = tokio::net::TcpListener::bind(config.server_address()).await?;

    tracing::info!("Server listening on {}", config.server_address());

    // Use into_make_service_with_connect_info to extract IP addresses for rate limiting
    let server = axum::serve(
        listener,
        components
            .router
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_token.clone()));

    tracing::info!("Press Ctrl+C to initiate graceful shutdown");

    // Run the server
    let result = server.await;

    // Wait for background tasks to complete
    tracing::info!("Waiting for background tasks to complete...");
    if let Err(e) = cleanup_handle.await {
        tracing::error!("Cleanup task panicked: {:?}", e);
    }

    // Shutdown OpenTelemetry gracefully if it was enabled
    if otel_enabled {
        shutdown_telemetry();
    }

    tracing::info!("Server shutdown complete");
    result?;

    Ok(())
}

/// Creates a future that completes when a shutdown signal is received.
/// Triggers the cancellation token to notify background tasks.
async fn shutdown_signal(shutdown_token: CancellationToken) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, initiating graceful shutdown");
        },
        _ = terminate => {
            tracing::info!("Received SIGTERM, initiating graceful shutdown");
        },
    }

    // Signal all background tasks to shutdown
    shutdown_token.cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Would a `target` event at `level` pass the filter?
    fn passes(filter: &Targets, target: &str, level: Level) -> bool {
        filter.would_enable(target, &level)
    }

    #[test]
    fn test_log_filter_default_enables_captchapi_debug() {
        let filter = log_filter(None);
        assert!(passes(&filter, "captchapi", Level::DEBUG));
        assert!(passes(&filter, "captchapi", Level::INFO));
        assert!(passes(&filter, "tower_http", Level::DEBUG));
        assert!(!passes(&filter, "captchapi", Level::TRACE));
        assert!(!passes(&filter, "some_noisy_crate", Level::INFO));
    }

    #[test]
    fn test_log_filter_accepts_target_level_pairs() {
        let filter = log_filter(Some("captchapi=warn"));
        assert!(passes(&filter, "captchapi", Level::WARN));
        assert!(!passes(&filter, "captchapi", Level::INFO));
    }

    #[test]
    fn test_log_filter_accepts_comma_separated_directives() {
        let filter = log_filter(Some("captchapi=info,tower_http=warn"));
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
            let filter = log_filter(Some(directive));
            assert!(
                passes(&filter, "any_crate_at_all", level),
                "{directive:?} should enable {level} globally"
            );
        }
        assert!(!passes(&log_filter(Some("off")), "captchapi", Level::ERROR));
    }

    #[test]
    fn test_log_filter_falls_back_when_the_level_is_invalid() {
        let filter = log_filter(Some("captchapi=notalevel"));
        // Fell back to the default, which enables captchapi at DEBUG.
        assert!(passes(&filter, "captchapi", Level::DEBUG));
        assert!(!passes(&filter, "some_noisy_crate", Level::INFO));
    }
}
