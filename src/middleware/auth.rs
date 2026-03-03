use crate::error::AppError;
use crate::metrics::Metrics;
use crate::services::{AuthService, StorageService};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct AuthMiddleware {
    pub storage: StorageService,
    pub auth_service: Arc<AuthService>,
    pub metrics: Arc<Metrics>,
    pub master_key: String,
}

impl AuthMiddleware {
    pub fn new(
        storage: StorageService,
        auth_service: Arc<AuthService>,
        metrics: Arc<Metrics>,
        master_key: String,
    ) -> Self {
        Self {
            storage,
            auth_service,
            metrics,
            master_key,
        }
    }

    #[tracing::instrument(skip(middleware, request, next), fields(auth_success))]
    pub async fn authenticate(
        State(middleware): State<AuthMiddleware>,
        request: Request,
        next: Next,
    ) -> Result<Response, AppError> {
        // Record authentication attempt
        middleware.metrics.api_keys.authentications.add(1, &[]);

        // Extract Authorization header
        let auth_header = request
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing authorization header".to_string()))?;

        // Check for Bearer token
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("Invalid authorization format".to_string()))?;

        // Check if token is the master key (constant-time comparison to prevent timing attacks)
        if token
            .as_bytes()
            .ct_eq(middleware.master_key.as_bytes())
            .into()
        {
            tracing::Span::current().record("auth_success", true);
            // Continue with the request (master key is always valid)
            return Ok(next.run(request).await);
        }

        // Hash the token
        let key_hash = middleware.auth_service.hash_api_key(token);

        // Validate against database
        let api_key = middleware
            .storage
            .get_api_key(&key_hash)
            .await?
            .ok_or_else(|| {
                tracing::Span::current().record("auth_success", false);
                middleware.metrics.api_keys.auth_failures.add(1, &[]);
                AppError::Unauthorized("Invalid API key".to_string())
            })?;

        if !api_key.is_active {
            tracing::Span::current().record("auth_success", false);
            middleware.metrics.api_keys.auth_failures.add(1, &[]);
            return Err(AppError::Unauthorized("API key is inactive".to_string()));
        }

        tracing::Span::current().record("auth_success", true);

        // Update last used timestamp (fire and forget)
        let storage = middleware.storage.clone();
        let key_hash = key_hash.clone();
        tokio::spawn(async move {
            let _ = storage.update_api_key_last_used(&key_hash).await;
        });

        // Continue with the request
        Ok(next.run(request).await)
    }
}

#[derive(Clone)]
pub struct MasterKeyMiddleware {
    pub master_key: String,
}

impl MasterKeyMiddleware {
    pub fn new(master_key: String) -> Self {
        Self { master_key }
    }

    pub async fn authenticate(
        State(middleware): State<MasterKeyMiddleware>,
        request: Request,
        next: Next,
    ) -> Result<Response, AppError> {
        // Extract Authorization header
        let auth_header = request
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing authorization header".to_string()))?;

        // Check for Bearer token
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("Invalid authorization format".to_string()))?;

        // Validate master key (constant-time comparison to prevent timing attacks)
        if !bool::from(token.as_bytes().ct_eq(middleware.master_key.as_bytes())) {
            return Err(AppError::Unauthorized("Invalid master key".to_string()));
        }

        // Continue with the request
        Ok(next.run(request).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::init_metrics;
    use crate::models::ApiKey;
    use crate::services::{AuthService, StorageService};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware as axum_middleware,
        response::IntoResponse,
        routing::get,
        Router,
    };
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn dummy_handler() -> impl IntoResponse {
        StatusCode::OK
    }

    /// Build an in-memory SQLite pool with migrations applied.
    async fn setup_pool() -> sqlx::SqlitePool {
        let db_name = format!("file:auth_test_{}?mode=memory&cache=shared", Uuid::new_v4());
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

    /// Build a test app with `AuthMiddleware` applied to GET /test.
    async fn build_auth_middleware_app(
        storage: StorageService,
        auth_service: Arc<AuthService>,
        master_key: &str,
    ) -> Router {
        let metrics = init_metrics();
        let middleware =
            AuthMiddleware::new(storage, auth_service, metrics, master_key.to_string());

        Router::new()
            .route("/test", get(dummy_handler))
            .layer(axum_middleware::from_fn_with_state(
                middleware,
                AuthMiddleware::authenticate,
            ))
    }

    /// Build a test app with `MasterKeyMiddleware` applied to GET /test.
    fn build_master_key_app(master_key: &str) -> Router {
        let middleware = MasterKeyMiddleware::new(master_key.to_string());

        Router::new()
            .route("/test", get(dummy_handler))
            .layer(axum_middleware::from_fn_with_state(
                middleware,
                MasterKeyMiddleware::authenticate,
            ))
    }

    // ─── AuthMiddleware tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_auth_missing_authorization_header() {
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));

        let app = build_auth_middleware_app(storage, auth_service, "master-key-minimum-16c").await;

        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Missing authorization header");
    }

    #[tokio::test]
    async fn test_auth_non_bearer_scheme_returns_invalid_format() {
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));

        let app = build_auth_middleware_app(storage, auth_service, "master-key-minimum-16c").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", "Token some-api-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Invalid authorization format");
    }

    #[tokio::test]
    async fn test_auth_empty_bearer_token_returns_invalid_api_key() {
        // "Bearer " with nothing after the space — hashes the empty string, fails DB lookup
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));

        let app = build_auth_middleware_app(storage, auth_service, "master-key-minimum-16c").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", "Bearer ")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Invalid API key");
    }

    #[tokio::test]
    async fn test_auth_unknown_api_key_returns_invalid_api_key() {
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));

        let app = build_auth_middleware_app(storage, auth_service, "master-key-minimum-16c").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", "Bearer unknown-api-key-xyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Invalid API key");
    }

    #[tokio::test]
    async fn test_auth_inactive_api_key_returns_invalid_api_key() {
        // get_api_key filters WHERE is_active = 1, so inactive keys are treated as
        // not found and return "Invalid API key".
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));

        let raw_key = "inactive-api-key-001";
        let key_hash = auth_service.hash_api_key(raw_key);
        let mut api_key_record = ApiKey::new(key_hash, Some("Inactive Key".to_string()));
        api_key_record.is_active = false;
        storage
            .create_api_key(&api_key_record)
            .await
            .expect("Failed to create inactive API key");

        let app = build_auth_middleware_app(storage, auth_service, "master-key-minimum-16c").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", format!("Bearer {}", raw_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // get_api_key filters out inactive keys (WHERE is_active = 1), so the lookup
        // returns None and the middleware returns "Invalid API key".
        assert_eq!(json["message"], "Invalid API key");
    }

    #[tokio::test]
    async fn test_auth_valid_api_key_succeeds() {
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));

        let raw_key = "valid-api-key-for-test-00";
        let key_hash = auth_service.hash_api_key(raw_key);
        let api_key_record = ApiKey::new(key_hash, Some("Valid Key".to_string()));
        storage
            .create_api_key(&api_key_record)
            .await
            .expect("Failed to create API key");

        let app = build_auth_middleware_app(storage, auth_service, "master-key-minimum-16c").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", format!("Bearer {}", raw_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_master_key_bypasses_db_lookup() {
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));
        let master_key = "master-key-minimum-16c";

        // No API keys in DB — master key should still pass
        let app = build_auth_middleware_app(storage, auth_service, master_key).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", format!("Bearer {}", master_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_last_used_at_populated_after_successful_auth() {
        let pool = setup_pool().await;
        let storage = StorageService::new(pool);
        let auth_service = Arc::new(AuthService::new("test-salt-minimum-16chars".to_string()));

        let raw_key = "trackable-api-key-0001";
        let key_hash = auth_service.hash_api_key(raw_key);
        let api_key_record = ApiKey::new(key_hash.clone(), Some("Trackable Key".to_string()));
        storage
            .create_api_key(&api_key_record)
            .await
            .expect("Failed to create API key");

        // Confirm last_used_at is None before authentication
        let before = storage
            .get_api_key_by_hash(&key_hash)
            .await
            .unwrap()
            .expect("Key should exist");
        assert!(
            before.last_used_at.is_none(),
            "last_used_at should be None before first use"
        );

        let app =
            build_auth_middleware_app(storage.clone(), auth_service, "master-key-minimum-16c")
                .await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", format!("Bearer {}", raw_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // The update is spawned via tokio::spawn (fire-and-forget). Give it a moment to complete.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let after = storage
            .get_api_key_by_hash(&key_hash)
            .await
            .unwrap()
            .expect("Key should still exist");
        assert!(
            after.last_used_at.is_some(),
            "last_used_at should be populated after successful authentication"
        );
    }

    // ─── MasterKeyMiddleware tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_master_key_missing_authorization_header() {
        let app = build_master_key_app("master-key-minimum-16c");

        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Missing authorization header");
    }

    #[tokio::test]
    async fn test_master_key_non_bearer_scheme_returns_invalid_format() {
        let app = build_master_key_app("master-key-minimum-16c");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", "Token master-key-minimum-16c")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Invalid authorization format");
    }

    #[tokio::test]
    async fn test_master_key_wrong_key_returns_invalid_master_key() {
        let app = build_master_key_app("master-key-minimum-16c");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", "Bearer wrong-master-key-000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Invalid master key");
    }

    #[tokio::test]
    async fn test_master_key_correct_key_succeeds() {
        let master_key = "master-key-minimum-16c";
        let app = build_master_key_app(master_key);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", format!("Bearer {}", master_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_master_key_lowercase_bearer_returns_invalid_format() {
        let master_key = "master-key-minimum-16c";
        let app = build_master_key_app(master_key);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("authorization", format!("bearer {}", master_key))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Invalid authorization format");
    }
}
