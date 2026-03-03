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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;
    use crate::services::{AuthService, StorageService};
    use crate::validation::API_KEY_LENGTH;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use uuid::Uuid;

    async fn setup_test_storage() -> StorageService {
        let db_name = format!(
            "file:test_api_key_ops_{}?mode=memory&cache=shared",
            Uuid::new_v4()
        );

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

        StorageService::new(pool)
    }

    fn make_auth_service() -> AuthService {
        AuthService::new("test-salt-for-api-key-ops".to_string())
    }

    fn make_metrics() -> Arc<Metrics> {
        Arc::new(Metrics::new())
    }

    // -----------------------------------------------------------------------
    // create_api_key_orchestrated — success
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_api_key_with_valid_description_succeeds() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();

        let result = create_api_key_orchestrated(
            &storage,
            &auth,
            &metrics,
            Some("My integration key".to_string()),
        )
        .await;

        assert!(
            result.is_ok(),
            "create_api_key_orchestrated should succeed with a valid description"
        );
        let (raw_key, api_key_record) = result.unwrap();

        // Raw key must be a non-empty alphanumeric string of the configured length
        assert_eq!(
            raw_key.len(),
            API_KEY_LENGTH,
            "Raw API key should be {} characters long",
            API_KEY_LENGTH
        );
        assert!(
            raw_key.chars().all(|c| c.is_ascii_alphanumeric()),
            "Raw API key should be alphanumeric"
        );

        // Returned record must carry the description and be active
        assert_eq!(
            api_key_record.description,
            Some("My integration key".to_string())
        );
        assert!(api_key_record.is_active);

        // key_hash must be the SHA-256 hash of the raw key + salt (non-empty, 64 hex chars)
        assert_eq!(
            api_key_record.key_hash.len(),
            64,
            "key_hash should be a 64-character hex-encoded SHA-256 digest"
        );

        // The hash stored in the record must match what AuthService produces for the same raw key
        let expected_hash = auth.hash_api_key(&raw_key);
        assert_eq!(
            api_key_record.key_hash, expected_hash,
            "key_hash in the returned record should equal hash_api_key(raw_key)"
        );
    }

    #[tokio::test]
    async fn test_create_api_key_is_retrievable_from_storage() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();

        let (raw_key, api_key_record) = create_api_key_orchestrated(
            &storage,
            &auth,
            &metrics,
            Some("Retrievable key".to_string()),
        )
        .await
        .unwrap();

        // Recompute the hash so we can look it up in storage
        let expected_hash = auth.hash_api_key(&raw_key);

        let fetched = storage.get_api_key(&expected_hash).await.unwrap();

        assert!(
            fetched.is_some(),
            "Created API key should be retrievable from storage by its hash"
        );
        let fetched_key = fetched.unwrap();
        assert_eq!(fetched_key.key_hash, api_key_record.key_hash);
        assert_eq!(fetched_key.description, Some("Retrievable key".to_string()));
        assert!(fetched_key.is_active);
        assert!(fetched_key.last_used_at.is_none());
    }

    #[tokio::test]
    async fn test_create_api_key_with_none_description_succeeds() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();

        let result = create_api_key_orchestrated(&storage, &auth, &metrics, None).await;

        assert!(
            result.is_ok(),
            "create_api_key_orchestrated should succeed when description is None"
        );
        let (_, api_key_record) = result.unwrap();
        assert!(
            api_key_record.description.is_none(),
            "API key record should have no description when None was passed"
        );
    }

    #[tokio::test]
    async fn test_create_api_key_generates_unique_keys_on_successive_calls() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();

        let (raw_key_1, _) =
            create_api_key_orchestrated(&storage, &auth, &metrics, Some("Key 1".to_string()))
                .await
                .unwrap();

        let (raw_key_2, _) =
            create_api_key_orchestrated(&storage, &auth, &metrics, Some("Key 2".to_string()))
                .await
                .unwrap();

        assert_ne!(
            raw_key_1, raw_key_2,
            "Successive calls should generate distinct raw API keys"
        );
    }

    // -----------------------------------------------------------------------
    // create_api_key_orchestrated — invalid description
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_api_key_with_empty_description_returns_invalid_api_key_params() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();

        let result =
            create_api_key_orchestrated(&storage, &auth, &metrics, Some("".to_string())).await;

        match result {
            Err(AppError::InvalidApiKeyParams(_)) => {}
            other => panic!(
                "Expected Err(AppError::InvalidApiKeyParams) for empty description, got {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn test_create_api_key_with_whitespace_only_description_returns_invalid_api_key_params() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();

        let result =
            create_api_key_orchestrated(&storage, &auth, &metrics, Some("   ".to_string())).await;

        match result {
            Err(AppError::InvalidApiKeyParams(_)) => {}
            other => panic!(
                "Expected Err(AppError::InvalidApiKeyParams) for whitespace-only description, got {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn test_create_api_key_with_oversized_description_returns_invalid_api_key_params() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();
        let oversized_description = "x".repeat(crate::validation::MAX_DESCRIPTION_LENGTH + 1);

        let result =
            create_api_key_orchestrated(&storage, &auth, &metrics, Some(oversized_description))
                .await;

        match result {
            Err(AppError::InvalidApiKeyParams(_)) => {}
            other => panic!(
                "Expected Err(AppError::InvalidApiKeyParams) for description exceeding max length, got {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn test_create_api_key_invalid_description_does_not_persist_anything() {
        let storage = setup_test_storage().await;
        let auth = make_auth_service();
        let metrics = make_metrics();

        // Validation must fail before any DB write occurs
        let _ = create_api_key_orchestrated(&storage, &auth, &metrics, Some("".to_string())).await;

        let all_keys = storage.list_api_keys().await.unwrap();
        assert!(
            all_keys.is_empty(),
            "No API key should be persisted when validation fails"
        );
    }
}
