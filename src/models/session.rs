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
            solution,
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
pub(crate) struct CreateSessionRequest {
    pub(crate) length: Option<i64>,
    pub(crate) expires_in_seconds: Option<u64>,
    pub(crate) difficulty: Option<i64>,
    pub(crate) width: Option<i64>,
    pub(crate) height: Option<i64>,
    pub(crate) dark_mode: Option<bool>,
    pub(crate) compression: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateSessionResponse {
    pub(crate) session_id: String,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateSessionRequest {
    pub(crate) solution: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ValidateSessionResponse {
    pub(crate) valid: bool,
    pub(crate) session_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct GetSessionDetailsResponse {
    pub(crate) session_id: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) attempt_count: i64,
    pub(crate) difficulty: i64,
    pub(crate) width: i64,
    pub(crate) height: i64,
    pub(crate) dark_mode: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session_row(dark_mode: i64, created_at: i64, expires_at: i64) -> SessionRow {
        SessionRow {
            id: "test-id".to_string(),
            solution: "abcd".to_string(),
            image_bytes: vec![0xFF, 0xD8],
            created_at,
            expires_at,
            attempt_count: 2,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode,
        }
    }

    #[test]
    fn test_session_new_sets_fields_correctly() {
        let before = Utc::now().timestamp();
        let session = Session::new(
            "solution123".to_string(),
            vec![1, 2, 3],
            300,
            7,
            320,
            150,
            true,
        );
        let after = Utc::now().timestamp();

        assert!(!session.id.is_empty(), "id should be non-empty UUID");
        assert_eq!(session.solution, "solution123");
        assert_eq!(session.image_bytes, vec![1, 2, 3]);
        assert_eq!(session.attempt_count, 0);
        assert_eq!(session.difficulty, 7);
        assert_eq!(session.width, 320);
        assert_eq!(session.height, 150);
        assert!(session.dark_mode);
        assert!(session.created_at >= before && session.created_at <= after);
        assert!(session.expires_at >= before + 300 && session.expires_at <= after + 300);
    }

    #[test]
    fn test_session_new_unique_ids() {
        let session1 = Session::new("sol".to_string(), vec![], 60, 5, 220, 120, false);
        let session2 = Session::new("sol".to_string(), vec![], 60, 5, 220, 120, false);
        assert_ne!(
            session1.id, session2.id,
            "Each session should have a unique ID"
        );
    }

    #[test]
    fn test_is_expired_not_expired() {
        let now = Utc::now().timestamp();
        let session = Session {
            id: "id".to_string(),
            solution: "sol".to_string(),
            image_bytes: vec![],
            created_at: now - 10,
            expires_at: now + 1000,
            attempt_count: 0,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode: false,
        };
        assert!(
            !session.is_expired(),
            "Session with future expires_at should not be expired"
        );
    }

    #[test]
    fn test_is_expired_already_expired() {
        let now = Utc::now().timestamp();
        let session = Session {
            id: "id".to_string(),
            solution: "sol".to_string(),
            image_bytes: vec![],
            created_at: now - 200,
            expires_at: now - 100,
            attempt_count: 0,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode: false,
        };
        assert!(
            session.is_expired(),
            "Session with past expires_at should be expired"
        );
    }

    #[test]
    fn test_is_expired_boundary_just_expired() {
        // expires_at in the past by 1 second — clearly expired
        let now = Utc::now().timestamp();
        let session = Session {
            id: "id".to_string(),
            solution: "sol".to_string(),
            image_bytes: vec![],
            created_at: now - 10,
            expires_at: now - 1,
            attempt_count: 0,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode: false,
        };
        assert!(
            session.is_expired(),
            "Session expiring 1 second ago should be expired"
        );
    }

    #[test]
    fn test_expires_at_datetime_known_value() {
        // 1_700_000_000 = 2023-11-14T22:13:20Z
        let session = Session {
            id: "id".to_string(),
            solution: "sol".to_string(),
            image_bytes: vec![],
            created_at: 0,
            expires_at: 1_700_000_000,
            attempt_count: 0,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode: false,
        };
        let dt = session.expires_at_datetime();
        assert_eq!(dt.timestamp(), 1_700_000_000);
    }

    #[test]
    fn test_created_at_datetime_valid_timestamp() {
        let session = Session {
            id: "id".to_string(),
            solution: "sol".to_string(),
            image_bytes: vec![],
            created_at: 1_700_000_000,
            expires_at: 1_700_001_000,
            attempt_count: 0,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode: false,
        };
        let dt = session.created_at_datetime();
        assert_eq!(dt.timestamp(), 1_700_000_000);
    }

    #[test]
    fn test_session_row_to_session_dark_mode_zero_is_false() {
        let row = make_session_row(0, 1_700_000_000, 1_700_001_000);
        let session: Session = row.into();
        assert!(!session.dark_mode, "dark_mode=0 should convert to false");
    }

    #[test]
    fn test_session_row_to_session_dark_mode_nonzero_is_true() {
        let row = make_session_row(2, 1_700_000_000, 1_700_001_000);
        let session: Session = row.into();
        assert!(
            session.dark_mode,
            "dark_mode=2 (any nonzero) should convert to true"
        );
    }

    #[test]
    fn test_session_row_to_session_field_mapping() {
        let row = SessionRow {
            id: "abc-123".to_string(),
            solution: "XyZw".to_string(),
            image_bytes: vec![0xFF, 0xD8, 0xFF],
            created_at: 1_700_000_000,
            expires_at: 1_700_001_000,
            attempt_count: 3,
            difficulty: 8,
            width: 400,
            height: 200,
            dark_mode: 1,
        };
        let session: Session = row.into();
        assert_eq!(session.id, "abc-123");
        assert_eq!(session.solution, "XyZw");
        assert_eq!(session.image_bytes, vec![0xFF, 0xD8, 0xFF]);
        assert_eq!(session.created_at, 1_700_000_000);
        assert_eq!(session.expires_at, 1_700_001_000);
        assert_eq!(session.attempt_count, 3);
        assert_eq!(session.difficulty, 8);
        assert_eq!(session.width, 400);
        assert_eq!(session.height, 200);
        assert!(session.dark_mode);
    }
}
