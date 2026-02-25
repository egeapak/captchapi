use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Internal struct for SQLite row mapping
/// SQLite stores booleans as integers (0/1), so this struct matches the database schema exactly
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct ApiKeyRow {
    pub key_hash: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub is_active: i64, // SQLite boolean (0/1)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key_hash: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub is_active: bool,
}

impl From<ApiKeyRow> for ApiKey {
    fn from(row: ApiKeyRow) -> Self {
        Self {
            key_hash: row.key_hash,
            description: row.description,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
            is_active: row.is_active != 0,
        }
    }
}

impl ApiKey {
    pub fn new(key_hash: String, description: Option<String>) -> Self {
        Self {
            key_hash,
            description,
            created_at: Utc::now().timestamp(),
            last_used_at: None,
            is_active: true,
        }
    }

    pub fn created_at_datetime(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.created_at, 0).unwrap_or_else(Utc::now)
    }

    pub fn last_used_at_datetime(&self) -> Option<DateTime<Utc>> {
        self.last_used_at
            .and_then(|ts| DateTime::from_timestamp(ts, 0))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateApiKeyRequest {
    pub(crate) description: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateApiKeyResponse {
    pub(crate) api_key: String,
    pub(crate) key_hash: String,
    pub(crate) description: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ApiKeyInfo {
    pub(crate) key_hash: String,
    pub(crate) description: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) last_used_at: Option<DateTime<Utc>>,
    pub(crate) is_active: bool,
}

impl From<ApiKey> for ApiKeyInfo {
    fn from(key: ApiKey) -> Self {
        Self {
            key_hash: key.key_hash.clone(),
            description: key.description.clone(),
            created_at: key.created_at_datetime(),
            last_used_at: key.last_used_at_datetime(),
            is_active: key.is_active,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateApiKeyRequest {
    pub(crate) is_active: Option<bool>,
    pub(crate) description: Option<String>,
}
