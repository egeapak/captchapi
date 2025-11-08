use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Internal struct for SQLite row mapping
/// SQLite stores booleans as integers (0/1), and all integers as i64
/// This struct matches the database schema exactly
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct SessionRow {
    pub id: String,
    pub solution: String,
    pub image_bytes: Vec<u8>,
    pub created_at: i64,
    pub expires_at: i64,
    pub attempt_count: i64, // SQLite INTEGER -> i64
    pub difficulty: i64,    // SQLite INTEGER -> i64
    pub width: i64,         // SQLite INTEGER -> i64
    pub height: i64,        // SQLite INTEGER -> i64
    pub dark_mode: i64,     // SQLite boolean (0/1)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub solution: String,
    pub image_bytes: Vec<u8>,
    pub created_at: i64,
    pub expires_at: i64,
    pub attempt_count: i64,
    pub difficulty: i64,
    pub width: i64,
    pub height: i64,
    pub dark_mode: bool,
}

impl From<SessionRow> for Session {
    fn from(row: SessionRow) -> Self {
        Self {
            id: row.id,
            solution: row.solution,
            image_bytes: row.image_bytes,
            created_at: row.created_at,
            expires_at: row.expires_at,
            attempt_count: row.attempt_count,
            difficulty: row.difficulty,
            width: row.width,
            height: row.height,
            dark_mode: row.dark_mode != 0,
        }
    }
}

impl Session {
    pub fn new(
        solution: String,
        image_bytes: Vec<u8>,
        expires_in_seconds: u64,
        difficulty: i64,
        width: i64,
        height: i64,
        dark_mode: bool,
    ) -> Self {
        let now = Utc::now().timestamp();
        Self {
            id: Uuid::new_v4().to_string(),
            solution: solution.to_lowercase(),
            image_bytes,
            created_at: now,
            expires_at: now + expires_in_seconds as i64,
            attempt_count: 0,
            difficulty,
            width,
            height,
            dark_mode,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() > self.expires_at
    }

    pub fn expires_at_datetime(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.expires_at, 0).unwrap_or_else(Utc::now)
    }

    pub fn created_at_datetime(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.created_at, 0).unwrap_or_else(Utc::now)
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub text: Option<String>,
    pub expires_in_seconds: Option<u64>,
    pub difficulty: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub dark_mode: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ValidateSessionRequest {
    pub solution: String,
}

#[derive(Debug, Serialize)]
pub struct ValidateSessionResponse {
    pub valid: bool,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct GetSessionDetailsResponse {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub attempt_count: i64,
    pub difficulty: i64,
    pub width: i64,
    pub height: i64,
    pub dark_mode: bool,
}
