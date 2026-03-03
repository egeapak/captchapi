use crate::error::Result;
use crate::models::{ApiKey, ApiKeyRow, Session, SessionRow};
use chrono::Utc;
use sqlx::sqlite::SqlitePool;

#[derive(Clone)]
pub struct StorageService {
    pool: SqlitePool,
}

impl StorageService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // Session operations
    #[tracing::instrument(skip(self, session), fields(session_id = %session.id))]
    pub async fn create_session(&self, session: &Session) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions (id, solution, image_bytes, created_at, expires_at, attempt_count, difficulty, width, height, dark_mode)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&session.id)
        .bind(&session.solution)
        .bind(&session.image_bytes)
        .bind(session.created_at)
        .bind(session.expires_at)
        .bind(session.attempt_count)
        .bind(session.difficulty)
        .bind(session.width)
        .bind(session.height)
        .bind(if session.dark_mode { 1 } else { 0 })
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Fetch a session by ID, eagerly deleting if expired.
    /// Returns Ok(None) for both missing and expired sessions.
    #[tracing::instrument(skip(self), fields(session_id = %session_id))]
    pub async fn get_active_session(&self, session_id: &str) -> Result<Option<Session>> {
        let Some(session) = self.get_session(session_id).await? else {
            return Ok(None);
        };
        if session.is_expired() {
            if let Err(e) = self.delete_session(session_id).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Failed to delete expired session during eager cleanup"
                );
            }
            return Ok(None);
        }
        Ok(Some(session))
    }

    #[tracing::instrument(skip(self), fields(session_id, found))]
    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let row = sqlx::query_as!(
            SessionRow,
            r#"
            SELECT
                id as "id!",
                solution as "solution!",
                image_bytes as "image_bytes!",
                created_at as "created_at!",
                expires_at as "expires_at!",
                attempt_count as "attempt_count!",
                difficulty as "difficulty!",
                width as "width!",
                height as "height!",
                dark_mode as "dark_mode!"
            FROM sessions
            WHERE id = ?
            "#,
            session_id
        )
        .fetch_optional(&self.pool)
        .await?;

        let found = row.is_some();
        tracing::Span::current().record("found", found);

        Ok(row.map(Session::from))
    }

    #[tracing::instrument(skip(self), fields(session_id))]
    pub async fn increment_attempt_count(&self, session_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions
            SET attempt_count = attempt_count + 1
            WHERE id = ?
            "#,
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[tracing::instrument(skip(self), fields(session_id, deleted))]
    pub async fn delete_session(&self, session_id: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM sessions
            WHERE id = ?
            "#,
        )
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        let deleted = result.rows_affected() > 0;
        tracing::Span::current().record("deleted", deleted);

        Ok(deleted)
    }

    #[tracing::instrument(skip(self), fields(deleted_count))]
    pub async fn delete_expired_sessions(&self) -> Result<u64> {
        let now = Utc::now().timestamp();
        let result = sqlx::query(
            r#"
            DELETE FROM sessions
            WHERE expires_at < ?
            "#,
        )
        .bind(now)
        .execute(&self.pool)
        .await?;

        let deleted_count = result.rows_affected();
        tracing::Span::current().record("deleted_count", deleted_count);

        Ok(deleted_count)
    }

    // API Key operations
    #[tracing::instrument(skip(self), fields(key_hash, found))]
    pub async fn get_api_key(&self, key_hash: &str) -> Result<Option<ApiKey>> {
        let row = sqlx::query_as!(
            ApiKeyRow,
            r#"
            SELECT
                key_hash as "key_hash!",
                description,
                created_at as "created_at!",
                last_used_at,
                is_active as "is_active!"
            FROM api_keys
            WHERE key_hash = ? AND is_active = 1
            "#,
            key_hash
        )
        .fetch_optional(&self.pool)
        .await?;

        let found = row.is_some();
        tracing::Span::current().record("found", found);

        Ok(row.map(ApiKey::from))
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>> {
        let row = sqlx::query_as!(
            ApiKeyRow,
            r#"
            SELECT
                key_hash as "key_hash!",
                description,
                created_at as "created_at!",
                last_used_at,
                is_active as "is_active!"
            FROM api_keys
            WHERE key_hash = ?
            "#,
            key_hash
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(ApiKey::from))
    }

    #[tracing::instrument(skip(self), fields(key_hash))]
    pub async fn update_api_key_last_used(&self, key_hash: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        sqlx::query!(
            r#"
            UPDATE api_keys
            SET last_used_at = ?
            WHERE key_hash = ?
            "#,
            now,
            key_hash
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[tracing::instrument(skip(self, api_key), fields(key_hash = %api_key.key_hash))]
    pub async fn create_api_key(&self, api_key: &ApiKey) -> Result<()> {
        let is_active = if api_key.is_active { 1 } else { 0 };
        sqlx::query!(
            r#"
            INSERT INTO api_keys (key_hash, description, created_at, is_active)
            VALUES (?, ?, ?, ?)
            "#,
            api_key.key_hash,
            api_key.description,
            api_key.created_at,
            is_active
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[tracing::instrument(skip(self), fields(count))]
    pub async fn list_api_keys(&self) -> Result<Vec<ApiKey>> {
        let rows = sqlx::query_as!(
            ApiKeyRow,
            r#"
            SELECT
                key_hash as "key_hash!",
                description,
                created_at as "created_at!",
                last_used_at,
                is_active as "is_active!"
            FROM api_keys
            ORDER BY created_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        let count = rows.len();
        tracing::Span::current().record("count", count);

        Ok(rows.into_iter().map(ApiKey::from).collect())
    }

    pub async fn update_api_key(
        &self,
        key_hash: &str,
        is_active: Option<bool>,
        description: Option<String>,
    ) -> Result<bool> {
        // Use compile-time checked queries via sqlx::query! macro
        // This provides type safety and verifies queries against the database schema
        let result = match (is_active, &description) {
            // Update both fields
            (Some(active), Some(desc)) => {
                let is_active_value = if active { 1 } else { 0 };
                sqlx::query!(
                    r#"
                    UPDATE api_keys
                    SET is_active = ?, description = ?
                    WHERE key_hash = ?
                    "#,
                    is_active_value,
                    desc,
                    key_hash
                )
                .execute(&self.pool)
                .await?
            }
            // Update only is_active
            (Some(active), None) => {
                let is_active_value = if active { 1 } else { 0 };
                sqlx::query!(
                    r#"
                    UPDATE api_keys
                    SET is_active = ?
                    WHERE key_hash = ?
                    "#,
                    is_active_value,
                    key_hash
                )
                .execute(&self.pool)
                .await?
            }
            // Update only description
            (None, Some(desc)) => {
                sqlx::query!(
                    r#"
                    UPDATE api_keys
                    SET description = ?
                    WHERE key_hash = ?
                    "#,
                    desc,
                    key_hash
                )
                .execute(&self.pool)
                .await?
            }
            // No fields to update
            (None, None) => return Ok(false),
        };

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_api_key(&self, key_hash: &str) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            DELETE FROM api_keys
            WHERE key_hash = ?
            "#,
            key_hash
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ApiKey, Session};
    use chrono::Utc;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use uuid::Uuid;

    async fn setup_test_storage() -> StorageService {
        let db_name = format!(
            "file:test_storage_{}?mode=memory&cache=shared",
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

    fn make_valid_session() -> Session {
        Session::new(
            "ANSWER".to_string(),
            vec![1, 2, 3],
            3600,
            5,
            220,
            120,
            false,
        )
    }

    fn make_expired_session() -> Session {
        let mut session = Session::new("EXPIRED".to_string(), vec![4, 5, 6], 0, 5, 220, 120, false);
        // Place expires_at 10 seconds in the past
        session.expires_at = Utc::now().timestamp() - 10;
        session
    }

    fn make_api_key(description: Option<&str>) -> ApiKey {
        let hash = format!("hash-{}", Uuid::new_v4());
        ApiKey::new(hash, description.map(str::to_string))
    }

    // -----------------------------------------------------------------------
    // Session CRUD
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_and_get_session() {
        let storage = setup_test_storage().await;
        let session = make_valid_session();

        storage.create_session(&session).await.unwrap();

        let fetched = storage
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session should exist");

        assert_eq!(fetched.id, session.id);
        assert_eq!(fetched.solution, session.solution);
        assert_eq!(fetched.image_bytes, session.image_bytes);
        assert_eq!(fetched.difficulty, session.difficulty);
        assert_eq!(fetched.width, session.width);
        assert_eq!(fetched.height, session.height);
        assert_eq!(fetched.dark_mode, session.dark_mode);
        assert_eq!(fetched.attempt_count, 0);
    }

    #[tokio::test]
    async fn test_get_session_missing_returns_none() {
        let storage = setup_test_storage().await;

        let result = storage.get_session("nonexistent-id").await.unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_session_returns_true_when_found() {
        let storage = setup_test_storage().await;
        let session = make_valid_session();
        storage.create_session(&session).await.unwrap();

        let deleted = storage.delete_session(&session.id).await.unwrap();

        assert!(deleted);
        assert!(storage.get_session(&session.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_delete_session_returns_false_when_missing() {
        let storage = setup_test_storage().await;

        let deleted = storage.delete_session("nonexistent-id").await.unwrap();

        assert!(!deleted);
    }

    // -----------------------------------------------------------------------
    // get_active_session — eager expiry deletion
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_active_session_returns_valid_session() {
        let storage = setup_test_storage().await;
        let session = make_valid_session();
        storage.create_session(&session).await.unwrap();

        let result = storage.get_active_session(&session.id).await.unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().id, session.id);
    }

    #[tokio::test]
    async fn test_get_active_session_returns_none_for_missing() {
        let storage = setup_test_storage().await;

        let result = storage.get_active_session("nonexistent-id").await.unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_active_session_deletes_expired_session_eagerly() {
        let storage = setup_test_storage().await;
        let expired = make_expired_session();
        storage.create_session(&expired).await.unwrap();

        // Confirm the row actually exists before calling get_active_session
        assert!(
            storage.get_session(&expired.id).await.unwrap().is_some(),
            "expired session should be stored"
        );

        // get_active_session must return None and delete the row
        let result = storage.get_active_session(&expired.id).await.unwrap();

        assert!(result.is_none(), "expired session should not be returned");

        // Row must have been eagerly deleted
        assert!(
            storage.get_session(&expired.id).await.unwrap().is_none(),
            "expired session should be deleted from DB"
        );
    }

    // -----------------------------------------------------------------------
    // increment_attempt_count
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_increment_attempt_count() {
        let storage = setup_test_storage().await;
        let session = make_valid_session();
        storage.create_session(&session).await.unwrap();

        storage.increment_attempt_count(&session.id).await.unwrap();
        storage.increment_attempt_count(&session.id).await.unwrap();

        let fetched = storage
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session should exist");

        assert_eq!(fetched.attempt_count, 2);
    }

    #[tokio::test]
    async fn test_increment_attempt_count_starts_at_zero() {
        let storage = setup_test_storage().await;
        let session = make_valid_session();
        storage.create_session(&session).await.unwrap();

        let fetched = storage
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session should exist");

        assert_eq!(fetched.attempt_count, 0);

        storage.increment_attempt_count(&session.id).await.unwrap();

        let fetched_after = storage
            .get_session(&session.id)
            .await
            .unwrap()
            .expect("session should still exist");

        assert_eq!(fetched_after.attempt_count, 1);
    }

    // -----------------------------------------------------------------------
    // delete_expired_sessions
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_expired_sessions_removes_only_expired() {
        let storage = setup_test_storage().await;
        let expired = make_expired_session();
        let valid = make_valid_session();

        storage.create_session(&expired).await.unwrap();
        storage.create_session(&valid).await.unwrap();

        let count = storage.delete_expired_sessions().await.unwrap();

        assert_eq!(count, 1);
        assert!(storage.get_session(&expired.id).await.unwrap().is_none());
        assert!(storage.get_session(&valid.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_delete_expired_sessions_empty_db_returns_zero() {
        let storage = setup_test_storage().await;

        let count = storage.delete_expired_sessions().await.unwrap();

        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_delete_expired_sessions_all_valid_returns_zero() {
        let storage = setup_test_storage().await;
        let v1 = make_valid_session();
        let v2 = make_valid_session();

        storage.create_session(&v1).await.unwrap();
        storage.create_session(&v2).await.unwrap();

        let count = storage.delete_expired_sessions().await.unwrap();

        assert_eq!(count, 0);
        assert!(storage.get_session(&v1.id).await.unwrap().is_some());
        assert!(storage.get_session(&v2.id).await.unwrap().is_some());
    }

    // -----------------------------------------------------------------------
    // API key CRUD
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_and_get_api_key() {
        let storage = setup_test_storage().await;
        let key = make_api_key(Some("test key"));

        storage.create_api_key(&key).await.unwrap();

        let fetched = storage
            .get_api_key(&key.key_hash)
            .await
            .unwrap()
            .expect("active key should be fetched");

        assert_eq!(fetched.key_hash, key.key_hash);
        assert_eq!(fetched.description, Some("test key".to_string()));
        assert!(fetched.is_active);
        assert!(fetched.last_used_at.is_none());
    }

    #[tokio::test]
    async fn test_get_api_key_returns_none_for_missing() {
        let storage = setup_test_storage().await;

        let result = storage.get_api_key("nonexistent-hash").await.unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_api_key_returns_none_for_inactive_key() {
        let storage = setup_test_storage().await;
        let mut key = make_api_key(Some("inactive"));
        key.is_active = false;
        storage.create_api_key(&key).await.unwrap();

        // get_api_key filters on is_active = 1
        let result = storage.get_api_key(&key.key_hash).await.unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_api_key_by_hash_fetches_inactive_key() {
        let storage = setup_test_storage().await;
        let mut key = make_api_key(Some("inactive"));
        key.is_active = false;
        storage.create_api_key(&key).await.unwrap();

        // get_api_key_by_hash ignores is_active
        let fetched = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .expect("get_api_key_by_hash should return inactive key");

        assert_eq!(fetched.key_hash, key.key_hash);
        assert!(!fetched.is_active);
    }

    #[tokio::test]
    async fn test_get_api_key_by_hash_fetches_active_key() {
        let storage = setup_test_storage().await;
        let key = make_api_key(None);
        storage.create_api_key(&key).await.unwrap();

        let fetched = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .expect("get_api_key_by_hash should return active key");

        assert_eq!(fetched.key_hash, key.key_hash);
        assert!(fetched.is_active);
    }

    #[tokio::test]
    async fn test_delete_api_key_returns_true_when_found() {
        let storage = setup_test_storage().await;
        let key = make_api_key(None);
        storage.create_api_key(&key).await.unwrap();

        let deleted = storage.delete_api_key(&key.key_hash).await.unwrap();

        assert!(deleted);
        assert!(storage.get_api_key(&key.key_hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_delete_api_key_returns_false_when_missing() {
        let storage = setup_test_storage().await;

        let deleted = storage.delete_api_key("nonexistent-hash").await.unwrap();

        assert!(!deleted);
    }

    // -----------------------------------------------------------------------
    // list_api_keys — ordering
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_api_keys_order_by_created_at_desc() {
        let storage = setup_test_storage().await;

        // Insert keys with distinct created_at timestamps
        let now = Utc::now().timestamp();

        let mut key_old = make_api_key(Some("old"));
        key_old.created_at = now - 100;

        let mut key_mid = make_api_key(Some("mid"));
        key_mid.created_at = now - 50;

        let mut key_new = make_api_key(Some("new"));
        key_new.created_at = now;

        // Insert in non-sorted order
        storage.create_api_key(&key_mid).await.unwrap();
        storage.create_api_key(&key_old).await.unwrap();
        storage.create_api_key(&key_new).await.unwrap();

        let keys = storage.list_api_keys().await.unwrap();

        assert_eq!(keys.len(), 3);
        // Newest first
        assert_eq!(keys[0].key_hash, key_new.key_hash);
        assert_eq!(keys[1].key_hash, key_mid.key_hash);
        assert_eq!(keys[2].key_hash, key_old.key_hash);
    }

    #[tokio::test]
    async fn test_list_api_keys_empty_returns_empty_vec() {
        let storage = setup_test_storage().await;

        let keys = storage.list_api_keys().await.unwrap();

        assert!(keys.is_empty());
    }

    // -----------------------------------------------------------------------
    // update_api_key_last_used
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_api_key_last_used_sets_timestamp() {
        let storage = setup_test_storage().await;
        let key = make_api_key(None);
        storage.create_api_key(&key).await.unwrap();

        // last_used_at should be None initially
        let before = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .unwrap();
        assert!(before.last_used_at.is_none());

        let before_call = Utc::now().timestamp();
        storage
            .update_api_key_last_used(&key.key_hash)
            .await
            .unwrap();
        let after_call = Utc::now().timestamp();

        let after = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .unwrap();

        let last_used = after.last_used_at.expect("last_used_at should be set");
        assert!(
            last_used >= before_call && last_used <= after_call,
            "last_used_at ({last_used}) should be between {before_call} and {after_call}"
        );
    }

    // -----------------------------------------------------------------------
    // update_api_key — all four branches
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_api_key_none_none_returns_false_without_touching_db() {
        let storage = setup_test_storage().await;
        let key = make_api_key(Some("original"));
        storage.create_api_key(&key).await.unwrap();

        // (None, None) must hit the early-return path and return Ok(false)
        let result = storage
            .update_api_key(&key.key_hash, None, None)
            .await
            .unwrap();

        assert!(!result);

        // Row must be unchanged
        let fetched = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.description, Some("original".to_string()));
        assert!(fetched.is_active);
    }

    #[tokio::test]
    async fn test_update_api_key_is_active_only() {
        let storage = setup_test_storage().await;
        let key = make_api_key(Some("desc"));
        storage.create_api_key(&key).await.unwrap();

        let updated = storage
            .update_api_key(&key.key_hash, Some(false), None)
            .await
            .unwrap();

        assert!(updated);

        let fetched = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .unwrap();
        assert!(!fetched.is_active);
        // description must be unchanged
        assert_eq!(fetched.description, Some("desc".to_string()));
    }

    #[tokio::test]
    async fn test_update_api_key_description_only() {
        let storage = setup_test_storage().await;
        let key = make_api_key(Some("old desc"));
        storage.create_api_key(&key).await.unwrap();

        let updated = storage
            .update_api_key(&key.key_hash, None, Some("new desc".to_string()))
            .await
            .unwrap();

        assert!(updated);

        let fetched = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.description, Some("new desc".to_string()));
        // is_active must be unchanged
        assert!(fetched.is_active);
    }

    #[tokio::test]
    async fn test_update_api_key_both_fields() {
        let storage = setup_test_storage().await;
        let key = make_api_key(Some("old desc"));
        storage.create_api_key(&key).await.unwrap();

        let updated = storage
            .update_api_key(&key.key_hash, Some(false), Some("new desc".to_string()))
            .await
            .unwrap();

        assert!(updated);

        let fetched = storage
            .get_api_key_by_hash(&key.key_hash)
            .await
            .unwrap()
            .unwrap();
        assert!(!fetched.is_active);
        assert_eq!(fetched.description, Some("new desc".to_string()));
    }

    #[tokio::test]
    async fn test_update_api_key_nonexistent_key_returns_false() {
        let storage = setup_test_storage().await;

        let result = storage
            .update_api_key("nonexistent-hash", Some(true), None)
            .await
            .unwrap();

        assert!(!result);
    }
}
