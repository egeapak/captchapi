// The binary is a thin wrapper over the library crate. Declaring `mod app; mod config; ...`
// here instead would compile the entire crate a second time and create a distinct set of
// types, so everything lives in `lib.rs` and is used from there.
use captchapi::app::build_app;
use captchapi::cli::{self, Handled, EXIT_USAGE};
use captchapi::config::ConfigHandle;
use captchapi::metrics::init_metrics;
use captchapi::tasks::start_cleanup_task;
#[cfg(feature = "otel")]
use captchapi::telemetry::shutdown_telemetry;
use captchapi::telemetry::{init_tracing, is_telemetry_enabled};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Arguments are parsed before anything else: `--log-level` and `--otel` must be known
    // before the tracing subscriber is built, because it can only be initialized once.
    let action = cli::parse(std::env::args_os().skip(1).collect()).unwrap_or_else(|e| {
        eprintln!("captchapi: {e}\n\nTry 'captchapi --help' for more information.");
        std::process::exit(EXIT_USAGE);
    });

    // Configuration errors are reported here, before any subscriber exists — which is why they
    // go to stderr with a usage exit code rather than through `tracing`.
    let (cli_args, config) = match cli::handle(action) {
        Ok(Handled::Done) => return Ok(()),
        Ok(Handled::Serve(cli_args, config)) => (*cli_args, *config),
        Err(e) => {
            eprintln!("captchapi: {e}");
            std::process::exit(EXIT_USAGE);
        }
    };

    let otel_enabled = is_telemetry_enabled(&config);

    // Initialize tracing, with OpenTelemetry export when it is available
    init_tracing(&config, otel_enabled)?;

    // The handle owns the running configuration from here on, and retains the parsed arguments
    // so a reload resolves from exactly the same sources as this boot did.
    let pid_file = PathBuf::from(&config.pid_file);
    let config = ConfigHandle::new(config, cli_args);

    // Best-effort: a server that cannot write its PID file is still a working server, it just
    // cannot be reached by `captchapi reload` without an explicit --pid.
    if let Err(e) = cli::write_pid_file(&pid_file) {
        tracing::warn!("{e}; `captchapi reload` will need --pid");
    }

    // Boot-only values: the listener, the pool and the rate limiter capture these, so they are
    // read once here and a reload reports drift on them rather than pretending to apply it.
    let boot = config.get();

    tracing::info!("Starting CaptchAPI server");
    tracing::info!(
        "Server configuration: {}:{}",
        boot.server_host,
        boot.server_port
    );

    // Create data directory if it doesn't exist
    if boot.database_url.starts_with("sqlite:") {
        let db_path = boot
            .database_url
            .strip_prefix("sqlite:")
            .ok_or_else(|| anyhow::anyhow!("DATABASE_URL must start with 'sqlite:'"))?;
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Set up database connection pool
    let pool = SqlitePoolOptions::new()
        .max_connections(boot.database_max_connections)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(
                    boot.database_url
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

    // Start cleanup task. It holds the handle rather than a fixed interval, so a reload
    // changes how often it runs without a restart.
    let cleanup_handle = start_cleanup_task(
        components.storage,
        config.clone(),
        metrics.clone(),
        components.governor_limiter,
        shutdown_token.clone(),
    );
    tracing::info!(
        "Background cleanup task started (interval: {}s)",
        boot.cleanup_interval_seconds
    );

    // Reload on SIGHUP. Installed before the server starts because SIGHUP's default
    // disposition terminates the process — which is exactly what `captchapi reload` sends.
    #[cfg(unix)]
    let reload_task = tokio::spawn(reload_on_sighup(
        config.clone(),
        metrics.clone(),
        shutdown_token.clone(),
    ));

    // Start server
    let listener = tokio::net::TcpListener::bind(boot.server_address()).await?;

    // Log the address the listener actually bound, which differs from the configured one when
    // port 0 is used to get an ephemeral port.
    let bound = listener.local_addr()?;
    tracing::info!("Server listening on {}", bound);

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

    // The background tasks only exit when the token is cancelled, and `shutdown_signal` cancels
    // it only on Ctrl-C/SIGTERM. If `serve` returned for any other reason, cancelling here is
    // what stops the awaits below from blocking forever and swallowing the error.
    shutdown_token.cancel();

    // Wait for background tasks to complete
    tracing::info!("Waiting for background tasks to complete...");
    if let Err(e) = cleanup_handle.await {
        tracing::error!("Cleanup task panicked: {:?}", e);
    }

    #[cfg(unix)]
    if let Err(e) = reload_task.await {
        tracing::error!("Reload task panicked: {:?}", e);
    }

    // A stale PID file would make `captchapi reload` signal a recycled, unrelated process.
    cli::remove_pid_file(&pid_file);

    // Shutdown OpenTelemetry gracefully if it was enabled
    #[cfg(feature = "otel")]
    if otel_enabled {
        shutdown_telemetry();
    }

    tracing::info!("Server shutdown complete");
    result?;

    Ok(())
}

/// Reload the configuration whenever SIGHUP arrives, until shutdown.
///
/// A failed reload is reported and discarded: a running server must never be taken down by a
/// bad edit to a config file.
#[cfg(unix)]
async fn reload_on_sighup(
    config: ConfigHandle,
    metrics: std::sync::Arc<captchapi::metrics::Metrics>,
    shutdown_token: CancellationToken,
) {
    let mut hangup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
        Ok(stream) => stream,
        Err(e) => {
            tracing::error!("Failed to install SIGHUP handler, reload disabled: {e}");
            return;
        }
    };

    loop {
        tokio::select! {
            _ = shutdown_token.cancelled() => break,
            received = hangup.recv() => {
                if received.is_none() {
                    break;
                }
                tracing::info!("Received SIGHUP, reloading configuration");
                // Resolution reads files, so it runs off the async worker threads.
                let handle = config.clone();
                match tokio::task::spawn_blocking(move || handle.reload()).await {
                    Ok(Ok(outcome)) => {
                        for field in &outcome.drift {
                            tracing::warn!(
                                "`{field}` changed but is applied only at startup; restart to change it"
                            );
                        }
                        metrics.system.config_reloads.add(1, &[]);
                        tracing::info!("Configuration reloaded");
                    }
                    Ok(Err(e)) => {
                        metrics.system.config_reload_failures.add(1, &[]);
                        tracing::error!("Reload failed, keeping the running configuration: {e}");
                    }
                    Err(e) => {
                        metrics.system.config_reload_failures.add(1, &[]);
                        tracing::error!("Reload task panicked: {e:?}");
                    }
                }
            }
        }
    }
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
