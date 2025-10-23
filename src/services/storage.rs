use crate::error::Result;
use crate::models::{ApiKey, Session};
use chrono::Utc;
use sqlx::{sqlite::SqlitePool, Row};

#[derive(Clone)]
pub struct StorageService {
    pool: SqlitePool,
}

impl StorageService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // Session operations
    pub async fn create_session(&self, session: &Session) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sessions (id, solution, image_base64, created_at, expires_at, attempt_count, difficulty, width, height, dark_mode)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&session.id)
        .bind(&session.solution)
        .bind(&session.image_base64)
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

    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let row = sqlx::query(
            r#"
            SELECT id, solution, image_base64, created_at, expires_at, attempt_count, difficulty, width, height, dark_mode
            FROM sessions
            WHERE id = ?
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| Session {
            id: r.get("id"),
            solution: r.get("solution"),
            image_base64: r.get("image_base64"),
            created_at: r.get("created_at"),
            expires_at: r.get("expires_at"),
            attempt_count: r.get("attempt_count"),
            difficulty: r.get("difficulty"),
            width: r.get("width"),
            height: r.get("height"),
            dark_mode: r.get::<i32, _>("dark_mode") == 1,
        }))
    }

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

        Ok(result.rows_affected() > 0)
    }

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

        Ok(result.rows_affected())
    }

    // API Key operations
    pub async fn get_api_key(&self, key_hash: &str) -> Result<Option<ApiKey>> {
        let row = sqlx::query(
            r#"
            SELECT key_hash, description, created_at, last_used_at, is_active
            FROM api_keys
            WHERE key_hash = ? AND is_active = 1
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ApiKey {
            key_hash: r.get("key_hash"),
            description: r.get("description"),
            created_at: r.get("created_at"),
            last_used_at: r.get("last_used_at"),
            is_active: r.get::<i32, _>("is_active") == 1,
        }))
    }

    pub async fn update_api_key_last_used(&self, key_hash: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        sqlx::query(
            r#"
            UPDATE api_keys
            SET last_used_at = ?
            WHERE key_hash = ?
            "#,
        )
        .bind(now)
        .bind(key_hash)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn create_api_key(&self, api_key: &ApiKey) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO api_keys (key_hash, description, created_at, is_active)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(&api_key.key_hash)
        .bind(&api_key.description)
        .bind(api_key.created_at)
        .bind(if api_key.is_active { 1 } else { 0 })
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_api_keys(&self) -> Result<Vec<ApiKey>> {
        let rows = sqlx::query(
            r#"
            SELECT key_hash, description, created_at, last_used_at, is_active
            FROM api_keys
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ApiKey {
                key_hash: r.get("key_hash"),
                description: r.get("description"),
                created_at: r.get("created_at"),
                last_used_at: r.get("last_used_at"),
                is_active: r.get::<i32, _>("is_active") == 1,
            })
            .collect())
    }

    pub async fn update_api_key(
        &self,
        key_hash: &str,
        is_active: Option<bool>,
        description: Option<String>,
    ) -> Result<bool> {
        // Build dynamic query based on what fields are being updated
        let mut query_parts = Vec::new();
        let mut had_update = false;

        if is_active.is_some() {
            query_parts.push("is_active = ?");
            had_update = true;
        }

        if description.is_some() {
            query_parts.push("description = ?");
            had_update = true;
        }

        if !had_update {
            return Ok(false);
        }

        let query_str = format!(
            "UPDATE api_keys SET {} WHERE key_hash = ?",
            query_parts.join(", ")
        );

        let mut query = sqlx::query(&query_str);

        if let Some(active) = is_active {
            query = query.bind(if active { 1 } else { 0 });
        }

        if let Some(desc) = description {
            query = query.bind(desc);
        }

        query = query.bind(key_hash);

        let result = query.execute(&self.pool).await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_api_key(&self, key_hash: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM api_keys
            WHERE key_hash = ?
            "#,
        )
        .bind(key_hash)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
