use axum::Router;
use captchapi::{
    config::{Config, ConfigHandle},
    metrics::Metrics,
    middleware::{AuthMiddleware, MasterKeyMiddleware},
    models::ApiKey,
    routes::{
        admin::AdminState, admin_routes, api_keys::ApiKeysState, api_keys_routes, health_check,
        sessions::SessionsState, sessions_routes,
    },
    services::{
        create_session_orchestrated, AuthService, CaptchaService, ImageCipher, SolutionHasher,
        StorageService,
    },
    validation::ValidatedSessionParams,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

pub struct TestApp {
    pub storage: StorageService,
    pub auth_service: Arc<AuthService>,
    pub solution_hasher: Arc<SolutionHasher>,
    pub image_cipher: Arc<ImageCipher>,
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
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));
        let solution_hasher = Arc::new(SolutionHasher::new("test-solution-secret-1234"));
        let image_cipher = Arc::new(ImageCipher::new("test-image-secret-1234"));

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
            solution_hasher,
            image_cipher,
            api_key: api_key.to_string(),
            master_key: "test-master-key-minimum-16chars".to_string(),
        }
    }

    pub fn build_app(&self) -> Router {
        self.build_app_with_config(Config {
            master_api_key: self.master_key.clone(),
            ..Config::for_test()
        })
    }

    /// Build the app over a specific configuration.
    ///
    /// Added alongside `build_app` rather than replacing it, so the other test files do not
    /// churn. The caller is responsible for setting `master_api_key` if it matters.
    #[allow(dead_code)] // Used in some test files, but clippy doesn't see cross-module usage
    pub fn build_app_with_config(&self, config: Config) -> Router {
        self.build_app_with_handle(ConfigHandle::from_static(config))
    }

    /// Build the app over a configuration handle the caller assembled.
    ///
    /// The way to get a handle with real provenance — `ConfigHandle::from_static_with_cli` —
    /// which `build_app_with_config` cannot express, since it takes a bare `Config`.
    #[allow(dead_code)] // Used in some test files, but clippy doesn't see cross-module usage
    pub fn build_app_with_handle(&self, config: ConfigHandle) -> Router {
        let captcha = Arc::new(CaptchaService::new());
        let metrics = Arc::new(Metrics::new());
        // `Config::for_test()` carries the same secrets this harness passes to SolutionHasher
        // and ImageCipher above, so the config and the services agree.

        let auth_middleware = AuthMiddleware::new(
            self.storage.clone(),
            self.auth_service.clone(),
            metrics.clone(),
            self.master_key.clone(),
        );
        let master_middleware = MasterKeyMiddleware::new(self.master_key.clone());
        let master_middleware_admin = MasterKeyMiddleware::new(self.master_key.clone());

        let sessions_state = SessionsState {
            storage: self.storage.clone(),
            captcha,
            solution_hasher: self.solution_hasher.clone(),
            image_cipher: self.image_cipher.clone(),
            config: config.clone(),
            metrics: metrics.clone(),
        };

        let api_keys_state = ApiKeysState {
            storage: self.storage.clone(),
            auth_service: self.auth_service.clone(),
            metrics: metrics.clone(),
        };

        let admin_state = AdminState {
            storage: self.storage.clone(),
            metrics: metrics.clone(),
            config: config.clone(),
            store: captchapi::services::ConfigStore::new(self.storage.pool().clone()),
            restart: None,
        };

        Router::new()
            .route("/health", axum::routing::get(health_check))
            .with_state(metrics.clone())
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

    /// Create a session through the service layer and return `(session_id, solution)`.
    ///
    /// Storage only holds a keyed hash of the answer and the HTTP API never
    /// returns it, so tests that need the correct solution have to capture it at
    /// creation time.
    #[allow(dead_code)] // Used in test files, but clippy doesn't see cross-module usage
    pub async fn create_session_with_solution(&self, length: i64) -> (String, String) {
        let captcha = CaptchaService::new();
        let metrics = Arc::new(Metrics::new());
        let params = ValidatedSessionParams {
            length,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode: false,
            compression: 40,
            expires_in: 300,
        };

        let created = create_session_orchestrated(
            &self.storage,
            &captcha,
            &self.solution_hasher,
            &self.image_cipher,
            &metrics,
            params,
        )
        .await
        .expect("Failed to create test session");

        (created.session.id, created.solution)
    }
}
