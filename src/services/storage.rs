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
