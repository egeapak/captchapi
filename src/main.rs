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
    RateLimitMiddleware,
};
use crate::routes::admin::AdminState;
use crate::routes::api_keys::ApiKeysState;
use crate::routes::sessions::SessionsState;
use crate::routes::{admin_routes, api_keys_routes, health_check, sessions_routes};
use crate::services::{AuthService, CaptchaService, RateLimiter, StorageService};
use crate::tasks::{start_cleanup_task, start_rate_limiter_cleanup_task};
use crate::telemetry::{init_telemetry, shutdown_telemetry};
use axum::{middleware as axum_middleware, routing::get, Router};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
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
        .max_connections(5)
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
    let rate_limiter = RateLimiter::new(
        config.rate_limit_requests_per_minute,
        config.rate_limit_window_seconds,
    );
    tracing::info!(
        "Rate limiter initialized: {} requests per {} seconds",
        config.rate_limit_requests_per_minute,
        config.rate_limit_window_seconds
    );

    // Initialize metrics
    let metrics = init_metrics();
    tracing::info!("Metrics initialized");

    // Start cleanup task
    start_cleanup_task(
        storage.clone(),
        config.cleanup_interval_seconds,
        metrics.clone(),
    );
    tracing::info!(
        "Background cleanup task started (interval: {}s)",
        config.cleanup_interval_seconds
    );

    start_rate_limiter_cleanup_task(
        rate_limiter.clone(),
        config.rate_limit_cleanup_interval_seconds,
    );
    tracing::info!(
        "Rate limiter cleanup task started (interval: {}s)",
        config.rate_limit_cleanup_interval_seconds
    );

    // Create middleware
    let auth_middleware =
        AuthMiddleware::new(storage.clone(), auth_service.clone(), metrics.clone());
    let master_middleware = MasterKeyMiddleware::new(config.master_api_key.clone());
    let master_middleware_admin = MasterKeyMiddleware::new(config.master_api_key.clone());
    let rate_limit_middleware = RateLimitMiddleware::new(rate_limiter, metrics.clone());
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

    // Build router
    let app = Router::new()
        .route("/health", get(health_check))
        .with_state(metrics.clone())
        .nest(
            "/api/v1/sessions",
            sessions_routes(sessions_state, auth_middleware, Some(rate_limit_middleware)),
        )
        .nest(
            "/api/v1/api-keys",
            api_keys_routes(api_keys_state, master_middleware),
        )
        .nest(
            "/api/v1/admin",
            admin_routes(admin_state, master_middleware_admin),
        )
        .layer(TraceLayer::new_for_http())
        .layer(axum_middleware::from_fn(request_id_middleware))
        .layer(axum_middleware::from_fn_with_state(
            metrics_middleware.clone(),
            MetricsMiddleware::track_request_duration,
        ));

    // Start server
    let listener = tokio::net::TcpListener::bind(config.server_address()).await?;

    tracing::info!("Server listening on {}", config.server_address());

    // Use into_make_service_with_connect_info to extract IP addresses for rate limiting
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await;

    // Shutdown OpenTelemetry gracefully if it was enabled
    if otel_enabled {
        shutdown_telemetry();
    }

    result?;

    Ok(())
}
