use crate::config::ConfigHandle;
use crate::metrics::{init_metrics, Metrics};
use crate::middleware::{
    request_id_middleware, AuthMiddleware, MasterKeyMiddleware, MetricsMiddleware,
};
use crate::routes::admin::AdminState;
use crate::routes::api_keys::ApiKeysState;
use crate::routes::sessions::SessionsState;
use crate::routes::{admin_routes, api_keys_routes, health_check, sessions_routes};
use crate::services::{
    AuthService, CaptchaService, ImageCipher, RateLimiterConfig, SolutionHasher, StorageService,
};
use axum::{middleware as axum_middleware, routing::get, Router};
use governor::DefaultKeyedRateLimiter;
use sqlx::SqlitePool;
use std::net::IpAddr;
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};

/// The output of `build_app`: the assembled router, the rate limiter, and the storage service.
///
/// `storage` is returned so the caller (e.g. `main`) can hand it to the background cleanup task.
pub struct AppComponents {
    pub router: Router,
    pub governor_limiter: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    pub storage: StorageService,
}

/// Build the Axum application router with all services, middleware, and rate limiting configured.
///
/// Returns [`AppComponents`] containing the assembled `Router`, the governor rate limiter handle
/// (needed for the cleanup task), and the `StorageService` (also needed for cleanup).
pub fn build_app(pool: SqlitePool, config: ConfigHandle, metrics: Arc<Metrics>) -> AppComponents {
    // Everything read here is boot-only: the values are captured into the services, the
    // middleware and the rate limiter, and cannot change without a restart. One snapshot is
    // therefore both sufficient and honest about what a reload can reach.
    let boot = config.get();

    // Initialize services
    let storage = StorageService::new(pool);
    let captcha = Arc::new(CaptchaService::new());
    let auth_service = Arc::new(AuthService::new(boot.api_key_salt.clone()));
    // Both secrets are boot-only, like the API key salt: they are captured into these services
    // and rotating them would invalidate every stored hash and ciphertext.
    let solution_hasher = Arc::new(SolutionHasher::new(&boot.solution_hash_secret));
    let image_cipher = Arc::new(ImageCipher::new(&boot.image_encryption_secret));

    // Configure rate limiter
    let rate_limiter_config = if boot.rate_limit_reverse_proxy {
        RateLimiterConfig::for_reverse_proxy(
            boot.rate_limit_requests_per_second,
            boot.rate_limit_burst_size,
        )
    } else {
        RateLimiterConfig::direct(
            boot.rate_limit_requests_per_second,
            boot.rate_limit_burst_size,
        )
    };

    // Create middleware
    let auth_middleware = AuthMiddleware::new(
        storage.clone(),
        auth_service.clone(),
        metrics.clone(),
        boot.master_api_key.clone(),
    );
    let master_middleware = MasterKeyMiddleware::new(boot.master_api_key.clone());
    let master_middleware_admin = MasterKeyMiddleware::new(boot.master_api_key.clone());
    let metrics_middleware = MetricsMiddleware::new(metrics.clone());

    // Create application state
    let sessions_state = SessionsState {
        storage: storage.clone(),
        captcha,
        solution_hasher,
        image_cipher,
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
        config: config.clone(),
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
    let (router, governor_limiter): (Router, Arc<DefaultKeyedRateLimiter<IpAddr>>) =
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
            let r = base_router.nest(
                "/api/v1/sessions",
                sessions_routes(sessions_state, auth_middleware).layer(layer),
            );
            (r, limiter)
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
            let r = base_router.nest(
                "/api/v1/sessions",
                sessions_routes(sessions_state, auth_middleware).layer(layer),
            );
            (r, limiter)
        };

    // Add common middleware layers
    let router = router
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(axum_middleware::from_fn(request_id_middleware))
        .layer(axum_middleware::from_fn_with_state(
            metrics_middleware.clone(),
            MetricsMiddleware::track_request_duration,
        ));

    AppComponents {
        router,
        governor_limiter,
        storage,
    }
}

/// Build the application with freshly initialized metrics.
///
/// Convenience wrapper that creates an `Arc<Metrics>` internally. Useful for tests and
/// scenarios where the caller does not need direct access to metrics.
#[allow(dead_code)]
pub fn build_app_with_metrics(
    pool: SqlitePool,
    config: ConfigHandle,
) -> (AppComponents, Arc<Metrics>) {
    let metrics = init_metrics();
    let components = build_app(pool, config, metrics.clone());
    (components, metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn make_test_pool() -> SqlitePool {
        let db_name = format!("file:test_app_{}?mode=memory&cache=shared", Uuid::new_v4());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_name)
                    .create_if_missing(true),
            )
            .await
            .expect("Failed to create test database");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        pool
    }

    fn make_test_config(reverse_proxy: bool) -> ConfigHandle {
        ConfigHandle::from_static(Config {
            rate_limit_reverse_proxy: reverse_proxy,
            ..Config::for_test()
        })
    }

    #[tokio::test]
    async fn test_build_app_returns_router_and_rate_limiter() {
        let pool = make_test_pool().await;
        let config = make_test_config(false);
        let metrics = init_metrics();

        let components = build_app(pool, config, metrics);

        // Verify the rate limiter is valid
        assert!(Arc::strong_count(&components.governor_limiter) >= 1);

        // Verify the router responds to /health with 200
        let response = components
            .router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_build_app_with_reverse_proxy_config() {
        let pool = make_test_pool().await;
        let config = make_test_config(true);
        let metrics = init_metrics();

        let components = build_app(pool, config, metrics);

        // Verify the rate limiter exists for reverse proxy mode
        assert!(Arc::strong_count(&components.governor_limiter) >= 1);

        // Router still serves /health correctly
        let response = components
            .router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_build_app_sessions_require_auth() {
        use axum::extract::ConnectInfo;
        use std::net::SocketAddr;

        let pool = make_test_pool().await;
        let config = make_test_config(false);
        let metrics = init_metrics();

        let components = build_app(pool, config, metrics);

        // The sessions routes have a GovernorLayer (rate limiter) that requires a ConnectInfo
        // extension to extract the client IP address. We set it manually here since there is no
        // real TCP listener in unit tests.
        let peer_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/sessions")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo::<SocketAddr>(peer_addr));

        let response = components.router.oneshot(request).await.unwrap();

        // Auth middleware runs before the handler — missing auth header returns 401.
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_build_app_with_metrics_convenience() {
        let pool = make_test_pool().await;
        let config = make_test_config(false);

        let (components, metrics) = build_app_with_metrics(pool, config);

        // Metrics arc should have at least 1 strong ref
        assert!(Arc::strong_count(&metrics) >= 1);

        // Router still serves /health correctly
        let response = components
            .router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
