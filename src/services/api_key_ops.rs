//! API key orchestration: shared business logic for HTTP and NAPI layers.

use crate::error::{AppError, Result};
use crate::metrics::Metrics;
use crate::models::ApiKey;
use crate::services::{AuthService, StorageService};
use crate::validation;
use std::sync::Arc;

/// Create an API key: validate description → generate → hash → persist → record metrics.
/// Returns (raw_key_string, api_key_model).
pub async fn create_api_key_orchestrated(
    storage: &StorageService,
    auth_service: &AuthService,
    metrics: &Arc<Metrics>,
    description: Option<String>,
) -> Result<(String, ApiKey)> {
    validation::validate_api_key_description(&description)
        .map_err(AppError::InvalidApiKeyParams)?;

    let api_key = validation::generate_api_key();
    let key_hash = auth_service.hash_api_key(&api_key);
    let api_key_record = ApiKey::new(key_hash, description);
    storage.create_api_key(&api_key_record).await?;
    metrics.api_keys.created.add(1, &[]);
    Ok((api_key, api_key_record))
}
