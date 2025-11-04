use axum::Router;
use captchapi::{
    config::Config,
    middleware::{AuthMiddleware, MasterKeyMiddleware},
    models::ApiKey,
    routes::{
        admin::AdminState, admin_routes, api_keys::ApiKeysState, api_keys_routes, health_check,
        sessions::SessionsState, sessions_routes,
    },
    services::{AuthService, CaptchaService, StorageService},
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

pub struct TestApp {
    pub storage: StorageService,
    pub auth_service: Arc<AuthService>,
    #[allow(dead_code)] // Used in test files, but clippy doesn't see cross-module usage
    pub api_key: String,
    pub master_key: String,
}

impl TestApp {
    pub async fn new() -> Self {
        // Create in-memory database for testing
        // Use a unique database name per test instance to avoid conflicts
        use uuid::Uuid;
        let db_name = format!("file:test_{}?mode=memory&cache=shared", Uuid::new_v4());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_name)
                    .create_if_missing(true),
            )
            .await
            .expect("Failed to create test database");

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt".to_string()));

        // Create a test API key
        let api_key = "test-api-key-123";
        let key_hash = auth_service.hash_api_key(api_key);
        let api_key_record = ApiKey::new(key_hash, Some("Test API Key".to_string()));
        storage
            .create_api_key(&api_key_record)
            .await
            .expect("Failed to create test API key");

        Self {
            storage,
            auth_service,
            api_key: api_key.to_string(),
            master_key: "test-master-key".to_string(),
        }
    }

    pub fn build_app(&self) -> Router {
        let captcha = Arc::new(CaptchaService::new());
        let config = Arc::new(Config {
            server_host: "127.0.0.1".to_string(),
            server_port: 3000,
            database_url: "sqlite::memory:".to_string(),
            api_key_salt: "test-salt".to_string(),
            master_api_key: self.master_key.clone(),
            default_session_ttl_seconds: 300,
            max_session_ttl_seconds: 3600,
            max_validation_attempts: 3,
            cleanup_interval_seconds: 60,
            captcha_compression: 40,
        });

        let auth_middleware = AuthMiddleware::new(self.storage.clone(), self.auth_service.clone());
        let master_middleware = MasterKeyMiddleware::new(config.master_api_key.clone());
        let master_middleware_admin = MasterKeyMiddleware::new(config.master_api_key.clone());

        let sessions_state = SessionsState {
            storage: self.storage.clone(),
            captcha,
            config: config.clone(),
        };

        let api_keys_state = ApiKeysState {
            storage: self.storage.clone(),
            auth_service: self.auth_service.clone(),
        };

        let admin_state = AdminState {
            storage: self.storage.clone(),
        };

        Router::new()
            .route("/health", axum::routing::get(health_check))
            .nest(
                "/api/v1/sessions",
                sessions_routes(sessions_state, auth_middleware),
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
    }
}
