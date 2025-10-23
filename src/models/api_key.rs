use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key_hash: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub is_active: bool,
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
