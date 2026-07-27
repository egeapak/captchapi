// The binary is a thin wrapper over the library crate. Declaring `mod app; mod config; ...`
// here instead would compile the entire crate a second time and create a distinct set of
// types, so everything lives in `lib.rs` and is used from there.
use captchapi::app::build_app;
use captchapi::cli::{self, Handled, EXIT_USAGE};
use captchapi::config::ConfigHandle;
use captchapi::metrics::init_metrics;
use captchapi::services::{BootOutcome, ConfigStore};
use captchapi::tasks::{start_cleanup_task, start_log_filter_task};
#[cfg(feature = "otel")]
use captchapi::telemetry::shutdown_telemetry;
use captchapi::telemetry::{init_tracing, is_telemetry_enabled};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

/// How long a process must serve before the configuration it booted with is considered good.
///
/// Long enough to be past the failures that matter — binding the listener, opening the pool,
/// building the rate limiter — and short enough that an operator restarting to apply a change
/// does not have to wait around before it is safe to restart again.
const CONFIRM_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

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
    let (cli_args, config, sources) = match cli::handle(action) {
        Ok(Handled::Done) => return Ok(()),
        Ok(Handled::Serve(cli_args, config, sources)) => (*cli_args, *config, *sources),
        Err(e) => {
            eprintln!("captchapi: {e}");
            std::process::exit(EXIT_USAGE);
        }
    };

    let otel_enabled = is_telemetry_enabled(&config);

    // Initialize tracing, with OpenTelemetry export when it is available. The returned handle
    // is what makes `log_level` a live field: the filter goes in behind a reload layer, so a
    // later change can reach the installed subscriber.
    let log_filter = init_tracing(&config, otel_enabled)?;

    tracing::info!("Starting CaptchAPI server");

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

    // ---- Configuration, phase two -------------------------------------------------------
    //
    // The config store lives in the database, and where that database is, is itself
    // configured — so its settings could not be read until now. Everything above this point
    // ran on the first-pass configuration; everything below runs on the second.
    //
    // This is a fresh resolve rather than a reload: nothing has captured a boot value yet, so
    // a stored `server_port` must still be able to decide what the listener binds to. A reload
    // would pin the boot fields to the first pass and defeat the point.
    let store = ConfigStore::new(pool.clone());

    let booted_generation = match store.prepare_boot().await? {
        BootOutcome::Clean { generation } => generation,
        BootOutcome::Trying { generation } => {
            tracing::info!("Trying configuration generation {generation} for the first time");
            Some(generation)
        }
        BootOutcome::RolledBack {
            generation,
            restored_fields,
        } => {
            tracing::warn!(
                "Configuration generation {generation} was never confirmed by the start that \
                 tried it; rolled back to the last confirmed settings ({restored_fields} \
                 field(s))"
            );
            None
        }
    };

    let stored = store.load().await?;
    let mut stored_applied = true;
    let (config, sources) = if stored.is_empty() {
        (config, sources)
    } else {
        match cli::resolve_with_stored(&cli_args, stored.clone()) {
            Ok(resolved) => {
                tracing::info!("Applied {} stored configuration setting(s)", stored.len());
                resolved
            }
            // A stored value that cannot resolve must not stop the server: it would take the
            // service down over a row in a table that the service itself is the only way to
            // edit. Carry on with the first-pass configuration and say so loudly.
            Err(e) => {
                stored_applied = false;
                tracing::error!("Ignoring the stored configuration, which does not resolve: {e}");
                (config, sources)
            }
        }
    };

    // Serving proves nothing about settings that were never applied. Confirming the generation
    // anyway would record a configuration that does not resolve as the known-good one — and
    // since a rollback restores the newest *confirmed* snapshot, that would make the broken
    // settings the thing every later rollback restores to.
    let booted_generation = if stored_applied {
        booted_generation
    } else {
        tracing::warn!(
            "Not confirming this configuration generation: its stored settings were not applied"
        );
        None
    };

    // Now that the final configuration is known, push it into the filter installed at startup.
    if let Err(e) = log_filter.apply(&config.log_level) {
        tracing::warn!("Keeping the startup log filter: {e}");
    }

    // The handle owns the running configuration from here on, and retains the parsed arguments
    // so a reload resolves from exactly the same sources as this boot did.
    let pid_file = PathBuf::from(&config.pid_file);
    let config = ConfigHandle::new(config, cli_args, sources, stored);

    // Written after phase two so a stored `pid_file` is honoured rather than being read from a
    // configuration the store had not yet contributed to.
    //
    // Best-effort: a server that cannot write its PID file is still a working server, it just
    // cannot be reached by `captchapi reload` without an explicit --pid.
    if let Err(e) = cli::write_pid_file(&pid_file) {
        tracing::warn!("{e}; `captchapi reload` will need --pid");
    }

    // Boot-only values: the listener, the pool and the rate limiter capture these, so they are
    // read once here and a reload reports drift on them rather than pretending to apply it.
    let boot = config.get();
    tracing::info!(
        "Server configuration: {}:{}",
        boot.server_host,
        boot.server_port
    );

    // Initialize metrics
    let metrics = init_metrics();
    tracing::info!("Metrics initialized");

    // Build application (services, middleware, router, rate limiter)
    let components = build_app(pool.clone(), config.clone(), metrics.clone());

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

    // Follow `log_level`, which is live only because the filter is behind a reload handle.
    let log_filter_task = start_log_filter_task(config.clone(), log_filter, shutdown_token.clone());

    // Confirm the generation this process booted with, once it has proved it can serve.
    // Deliberately the id `prepare_boot` reported: a write made while this process runs opens a
    // generation it has proved nothing about.
    let confirm_task = booted_generation.map(|generation| {
        let store = store.clone();
        let shutdown = shutdown_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = shutdown.cancelled() => {}
                _ = tokio::time::sleep(CONFIRM_AFTER) => {
                    match store.confirm(generation).await {
                        Ok(true) => tracing::info!(
                            "Configuration generation {generation} confirmed after \
                             {}s of serving",
                            CONFIRM_AFTER.as_secs()
                        ),
                        Ok(false) => {}
                        Err(e) => tracing::error!(
                            "Could not confirm configuration generation {generation}: {e}"
                        ),
                    }
                }
            }
        })
    });

    // Reload on SIGHUP. Installed before the server starts because SIGHUP's default
    // disposition terminates the process — which is exactly what `captchapi reload` sends.
    #[cfg(unix)]
    let reload_task = tokio::spawn(reload_on_sighup(
        config.clone(),
        store.clone(),
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

    if let Err(e) = log_filter_task.await {
        tracing::error!("Log filter task panicked: {:?}", e);
    }

    if let Some(task) = confirm_task {
        if let Err(e) = task.await {
            tracing::error!("Config confirmation task panicked: {:?}", e);
        }
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
    store: ConfigStore,
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
                // The store is a source of truth like the files are, so a reload re-reads it.
                let stored = match store.load().await {
                    Ok(stored) => stored,
                    Err(e) => {
                        metrics.system.config_reload_failures.add(1, &[]);
                        tracing::error!("Reload failed, cannot read the config store: {e}");
                        continue;
                    }
                };
                // Resolution reads files, so it runs off the async worker threads.
                let handle = config.clone();
                match tokio::task::spawn_blocking(move || handle.reload(stored)).await {
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
