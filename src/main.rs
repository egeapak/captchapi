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
use crate::middleware::{AuthMiddleware, MasterKeyMiddleware};
use crate::routes::api_keys::ApiKeysState;
use crate::routes::sessions::SessionsState;
use crate::routes::{api_keys_routes, health_check, sessions_routes};
use crate::services::{AuthService, CaptchaService, StorageService};
use crate::tasks::start_cleanup_task;
use crate::telemetry::{init_telemetry, shutdown_telemetry};
use axum::{routing::get, Router};
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

    // Create middleware
    let auth_middleware =
        AuthMiddleware::new(storage.clone(), auth_service.clone(), metrics.clone());
    let master_middleware = MasterKeyMiddleware::new(config.master_api_key.clone());

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

    // Build router
    let app = Router::new()
        .route("/health", get(health_check))
        .nest(
            "/api/v1/sessions",
            sessions_routes(sessions_state, auth_middleware),
        )
        .nest(
            "/api/v1/api-keys",
            api_keys_routes(api_keys_state, master_middleware),
        )
        .layer(TraceLayer::new_for_http());

    // Start server
    let listener = tokio::net::TcpListener::bind(config.server_address()).await?;

    tracing::info!("Server listening on {}", config.server_address());

    let result = axum::serve(listener, app).await;

    // Shutdown OpenTelemetry gracefully if it was enabled
    if otel_enabled {
        shutdown_telemetry();
    }

    result?;

    Ok(())
}
