use crate::error::AppError;
use crate::metrics::Metrics;
use crate::services::{AuthService, StorageService};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthMiddleware {
    pub storage: StorageService,
    pub auth_service: Arc<AuthService>,
    pub metrics: Arc<Metrics>,
}

impl AuthMiddleware {
    pub fn new(
        storage: StorageService,
        auth_service: Arc<AuthService>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            storage,
            auth_service,
            metrics,
        }
    }

    #[tracing::instrument(skip(middleware, request, next), fields(auth_success))]
    pub async fn authenticate(
        State(middleware): State<AuthMiddleware>,
        request: Request,
        next: Next,
    ) -> Result<Response, AppError> {
        // Record authentication attempt
        middleware.metrics.api_key_authentications.add(1, &[]);

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

        // Hash the token
        let key_hash = middleware.auth_service.hash_api_key(token);

        // Validate against database
        let api_key = middleware
            .storage
            .get_api_key(&key_hash)
            .await?
            .ok_or_else(|| {
                tracing::Span::current().record("auth_success", false);
                AppError::Unauthorized("Invalid API key".to_string())
            })?;

        if !api_key.is_active {
            tracing::Span::current().record("auth_success", false);
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

        // Validate master key
        if token != middleware.master_key {
            return Err(AppError::Unauthorized("Invalid master key".to_string()));
        }

        // Continue with the request
        Ok(next.run(request).await)
    }
}
