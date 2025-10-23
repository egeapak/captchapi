use crate::error::AppError;
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
}

impl AuthMiddleware {
    pub fn new(storage: StorageService, auth_service: Arc<AuthService>) -> Self {
        Self {
            storage,
            auth_service,
        }
    }

    pub async fn authenticate(
        State(middleware): State<AuthMiddleware>,
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

        // Hash the token
        let key_hash = middleware.auth_service.hash_api_key(token);

        // Validate against database
        let api_key = middleware
            .storage
            .get_api_key(&key_hash)
            .await?
            .ok_or_else(|| AppError::Unauthorized("Invalid API key".to_string()))?;

        if !api_key.is_active {
            return Err(AppError::Unauthorized("API key is inactive".to_string()));
        }

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
