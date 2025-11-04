use crate::error::{AppError, Result};
use crate::middleware::MasterKeyMiddleware;
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

#[tracing::instrument(skip(state, req), fields(
    key_hash,
    has_description = req.description.is_some()
))]
async fn create_api_key(
    State(state): State<ApiKeysState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateApiKeyResponse>)> {
    // Generate a random API key
    let api_key: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    // Hash the API key
    let key_hash = state.auth_service.hash_api_key(&api_key);

    // Record key_hash in span
    tracing::Span::current().record("key_hash", &key_hash.as_str());

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

#[tracing::instrument(skip(state), fields(count))]
async fn list_api_keys(State(state): State<ApiKeysState>) -> Result<Json<Vec<ApiKeyInfo>>> {
    let api_keys = state.storage.list_api_keys().await?;

    let count = api_keys.len();
    tracing::Span::current().record("count", count);

    let api_key_infos: Vec<ApiKeyInfo> = api_keys.into_iter().map(ApiKeyInfo::from).collect();

    Ok(Json(api_key_infos))
}

#[tracing::instrument(skip(state, req), fields(
    key_hash = %key_hash,
    updated,
    update_is_active = req.is_active.is_some(),
    update_description = req.description.is_some()
))]
async fn update_api_key(
    State(state): State<ApiKeysState>,
    Path(key_hash): Path<String>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Result<Json<ApiKeyInfo>> {
    // Update the API key
    let updated = state
        .storage
        .update_api_key(&key_hash, req.is_active, req.description)
        .await?;

    tracing::Span::current().record("updated", updated);

    if !updated {
        return Err(AppError::SessionNotFound); // Reusing this error, could create ApiKeyNotFound
    }

    // Fetch the updated key (use get_api_key_by_hash to get regardless of active status)
    let api_key = state
        .storage
        .get_api_key_by_hash(&key_hash)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    tracing::info!("Updated API key with hash: {}", key_hash);

    Ok(Json(ApiKeyInfo::from(api_key)))
}

#[tracing::instrument(skip(state), fields(key_hash = %key_hash, deleted))]
async fn delete_api_key(
    State(state): State<ApiKeysState>,
    Path(key_hash): Path<String>,
) -> Result<axum::http::StatusCode> {
    let deleted = state.storage.delete_api_key(&key_hash).await?;

    tracing::Span::current().record("deleted", deleted);

    if deleted {
        tracing::info!("Deleted API key with hash: {}", key_hash);
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::SessionNotFound)
    }
}
