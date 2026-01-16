mod config;
mod error;
mod metrics;
mod middleware;
mod models;
mod routes;
mod services;
mod tasks;
mod telemetry;

use crate::config::Config;
use crate::metrics::init_metrics;
use crate::middleware::{
    request_id_middleware, AuthMiddleware, MasterKeyMiddleware, MetricsMiddleware,
};
use crate::routes::admin::AdminState;
use crate::routes::api_keys::ApiKeysState;
use crate::routes::sessions::SessionsState;
use crate::routes::{admin_routes, api_keys_routes, health_check, sessions_routes};
use crate::services::{AuthService, CaptchaService, RateLimiterConfig, StorageService};
use crate::tasks::start_cleanup_task;
use crate::telemetry::{init_telemetry, shutdown_telemetry};
use axum::{middleware as axum_middleware, routing::get, Router};
use governor::DefaultKeyedRateLimiter;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::net::IpAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower_governor::{governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "captchapi=debug,tower_http=debug".into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .with(telemetry_layer)
            .init();

        tracing::info!("OpenTelemetry enabled");
    } else {
        // Initialize tracing without OpenTelemetry
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "captchapi=debug,tower_http=debug".into()),
            )
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
        let db_path = config.database_url.strip_prefix("sqlite:").unwrap();
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Set up database connection pool
    let pool = SqlitePoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(config.database_url.strip_prefix("sqlite:").unwrap())
                .create_if_missing(true),
        )
        .await?;

    tracing::info!("Database connection established");

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("Database migrations completed");

    // Initialize services
    let storage = StorageService::new(pool);
    let captcha = Arc::new(CaptchaService::new());
    let auth_service = Arc::new(AuthService::new(config.api_key_salt.clone()));

    // Configure rate limiter using tower_governor
    // Uses GCRA (Generic Cell Rate Algorithm) for sophisticated rate limiting
    // When behind a reverse proxy, use SmartIpKeyExtractor to read X-Forwarded-For/X-Real-IP headers
    let rate_limiter_config = RateLimiterConfig::new(
        config.rate_limit_requests_per_second,
        config.rate_limit_burst_size,
        config.rate_limit_reverse_proxy,
    );
    tracing::info!(
        "Rate limiter initialized: {} requests/second, burst size {}, reverse proxy mode: {}",
        rate_limiter_config.requests_per_second,
        rate_limiter_config.burst_size,
        rate_limiter_config.reverse_proxy
    );

    // Initialize metrics
    let metrics = init_metrics();
    tracing::info!("Metrics initialized");

    // Create middleware
    let auth_middleware = AuthMiddleware::new(
        storage.clone(),
        auth_service.clone(),
        metrics.clone(),
        config.master_api_key.clone(),
    );
    let master_middleware = MasterKeyMiddleware::new(config.master_api_key.clone());
    let master_middleware_admin = MasterKeyMiddleware::new(config.master_api_key.clone());
    let metrics_middleware = MetricsMiddleware::new(metrics.clone());

    // Create application state
    let sessions_state = SessionsState {
        storage: storage.clone(),
        captcha,
        config: config.clone(),
        metrics: metrics.clone(),
    };

    let api_keys_state = ApiKeysState {
        storage: storage.clone(),
        auth_service: auth_service.clone(),
        metrics: metrics.clone(),
    };

    let admin_state = AdminState {
        storage: storage.clone(),
        metrics: metrics.clone(),
    };

    // Build base router (without rate-limited sessions routes)
    let base_router = Router::new()
        .route("/health", get(health_check))
        .with_state(metrics.clone())
        .nest(
            "/api/v1/api-keys",
            api_keys_routes(api_keys_state, master_middleware),
        )
        .nest(
            "/api/v1/admin",
            admin_routes(admin_state, master_middleware_admin),
        );

    // Build router with rate limiting, using appropriate key extractor based on config
    // When behind a reverse proxy, SmartIpKeyExtractor reads X-Forwarded-For, X-Real-IP, Forwarded headers
    // Otherwise, PeerIpKeyExtractor uses the direct socket connection IP
    let (app, governor_limiter): (Router, Arc<DefaultKeyedRateLimiter<IpAddr>>) =
        if rate_limiter_config.reverse_proxy {
            let governor_conf = Arc::new(
                GovernorConfigBuilder::default()
                    .per_second(rate_limiter_config.requests_per_second)
                    .burst_size(rate_limiter_config.burst_size)
                    .key_extractor(SmartIpKeyExtractor)
                    .finish()
                    .expect("Failed to build governor config"),
            );
            let limiter = governor_conf.limiter().clone();
            let layer = tower_governor::GovernorLayer::new(governor_conf);
            let router = base_router.nest(
                "/api/v1/sessions",
                sessions_routes(sessions_state, auth_middleware).layer(layer),
            );
            (router, limiter)
        } else {
            let governor_conf = Arc::new(
                GovernorConfigBuilder::default()
                    .per_second(rate_limiter_config.requests_per_second)
                    .burst_size(rate_limiter_config.burst_size)
                    .finish()
                    .expect("Failed to build governor config"),
            );
            let limiter = governor_conf.limiter().clone();
            let layer = tower_governor::GovernorLayer::new(governor_conf);
            let router = base_router.nest(
                "/api/v1/sessions",
                sessions_routes(sessions_state, auth_middleware).layer(layer),
            );
            (router, limiter)
        };

    // Add common layers
    let app = app
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(axum_middleware::from_fn(request_id_middleware))
        .layer(axum_middleware::from_fn_with_state(
            metrics_middleware.clone(),
            MetricsMiddleware::track_request_duration,
        ));

    // Create shutdown token for graceful shutdown
    let shutdown_token = CancellationToken::new();

    // Start cleanup task
    let cleanup_handle = start_cleanup_task(
        storage.clone(),
        config.cleanup_interval_seconds,
        metrics.clone(),
        governor_limiter,
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
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
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
