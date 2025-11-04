use crate::error::{AppError, Result};
use crate::middleware::MasterKeyMiddleware;
use crate::models::api_key::validate_description;
use crate::models::{
    ApiKey, ApiKeyInfo, CreateApiKeyRequest, CreateApiKeyResponse, UpdateApiKeyRequest,
};
use crate::services::{AuthService, StorageService};
use axum::{
    extract::{Path, State},
    middleware,
    routing::{delete, get, post, put},
    Json, Router,
};
use rand::{distributions::Alphanumeric, Rng};
use std::sync::Arc;

#[derive(Clone)]
pub struct ApiKeysState {
    pub storage: StorageService,
    pub auth_service: Arc<AuthService>,
}

pub fn api_keys_routes(state: ApiKeysState, master_middleware: MasterKeyMiddleware) -> Router {
    Router::new()
        .route("/", post(create_api_key))
        .route("/", get(list_api_keys))
        .route("/{key_hash}", put(update_api_key))
        .route("/{key_hash}", delete(delete_api_key))
        .route_layer(middleware::from_fn_with_state(
            master_middleware,
            MasterKeyMiddleware::authenticate,
        ))
        .with_state(state)
}

async fn create_api_key(
    State(state): State<ApiKeysState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateApiKeyResponse>)> {
    // Validate description
    validate_description(&req.description).map_err(|e| AppError::InvalidApiKeyParams(e))?;

    // Generate a random API key
    let api_key: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    // Hash the API key
    let key_hash = state.auth_service.hash_api_key(&api_key);

    // Create the API key record
    let api_key_record = ApiKey::new(key_hash.clone(), req.description.clone());

    // Save to database
    state.storage.create_api_key(&api_key_record).await?;

    tracing::info!(
        "Created API key with hash: {} (description: {:?})",
        key_hash,
        req.description
    );

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            api_key,
            key_hash,
            description: req.description,
            created_at: api_key_record.created_at_datetime(),
        }),
    ))
}

async fn list_api_keys(State(state): State<ApiKeysState>) -> Result<Json<Vec<ApiKeyInfo>>> {
    let api_keys = state.storage.list_api_keys().await?;

    let api_key_infos: Vec<ApiKeyInfo> = api_keys.into_iter().map(ApiKeyInfo::from).collect();

    Ok(Json(api_key_infos))
}

async fn update_api_key(
    State(state): State<ApiKeysState>,
    Path(key_hash): Path<String>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiKeyInfo>> {
    // Validate description if present
    validate_description(&req.description).map_err(|e| AppError::InvalidApiKeyParams(e))?;

    // Update the API key
    let updated = state
        .storage
        .update_api_key(&key_hash, req.is_active, req.description)
        .await?;

    if !updated {
        return Err(AppError::ApiKeyNotFound);
    }

    // Fetch the updated key (use get_api_key_by_hash to get regardless of active status)
    let api_key = state
        .storage
        .get_api_key_by_hash(&key_hash)
        .await?
        .ok_or(AppError::ApiKeyNotFound)?;

    tracing::info!("Updated API key with hash: {}", key_hash);

    Ok(Json(ApiKeyInfo::from(api_key)))
}

async fn delete_api_key(
    State(state): State<ApiKeysState>,
    Path(key_hash): Path<String>,
) -> Result<axum::http::StatusCode> {
    let deleted = state.storage.delete_api_key(&key_hash).await?;

    if deleted {
        tracing::info!("Deleted API key with hash: {}", key_hash);
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::ApiKeyNotFound)
    }
}
