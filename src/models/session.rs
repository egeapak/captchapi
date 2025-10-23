use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub solution: String,
    pub image_base64: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub attempt_count: i32,
    pub difficulty: i32,
    pub width: i32,
    pub height: i32,
    pub dark_mode: bool,
}

impl Session {
    pub fn new(
        solution: String,
        image_base64: String,
        expires_in_seconds: u64,
        difficulty: i32,
        width: i32,
        height: i32,
        dark_mode: bool,
    ) -> Self {
        let now = Utc::now().timestamp();
        Self {
            id: Uuid::new_v4().to_string(),
            solution: solution.to_lowercase(),
            image_base64,
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
    pub difficulty: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
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
pub struct GetImageResponse {
    pub image: String,
    pub expires_at: DateTime<Utc>,
}
