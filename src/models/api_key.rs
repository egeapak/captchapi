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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_api_key_row(is_active: i64, last_used_at: Option<i64>) -> ApiKeyRow {
        ApiKeyRow {
            key_hash: "deadbeef1234".to_string(),
            description: Some("Test Key".to_string()),
            created_at: 1_700_000_000,
            last_used_at,
            is_active,
        }
    }

    #[test]
    fn test_api_key_new_sets_fields_correctly() {
        let before = Utc::now().timestamp();
        let key = ApiKey::new("hash123".to_string(), Some("My Key".to_string()));
        let after = Utc::now().timestamp();

        assert_eq!(key.key_hash, "hash123");
        assert_eq!(key.description, Some("My Key".to_string()));
        assert!(key.created_at >= before && key.created_at <= after);
        assert!(key.last_used_at.is_none());
        assert!(key.is_active);
    }

    #[test]
    fn test_api_key_new_no_description() {
        let key = ApiKey::new("hash456".to_string(), None);
        assert_eq!(key.key_hash, "hash456");
        assert!(key.description.is_none());
        assert!(key.is_active);
    }

    #[test]
    fn test_api_key_row_to_api_key_is_active_zero_is_false() {
        let row = make_api_key_row(0, None);
        let key: ApiKey = row.into();
        assert!(!key.is_active, "is_active=0 should convert to false");
    }

    #[test]
    fn test_api_key_row_to_api_key_is_active_nonzero_is_true() {
        let row = make_api_key_row(42, None);
        let key: ApiKey = row.into();
        assert!(
            key.is_active,
            "is_active=42 (any nonzero) should convert to true"
        );
    }

    #[test]
    fn test_api_key_row_to_api_key_field_mapping() {
        let row = ApiKeyRow {
            key_hash: "abc123hash".to_string(),
            description: Some("Production Key".to_string()),
            created_at: 1_700_000_000,
            last_used_at: Some(1_700_000_500),
            is_active: 1,
        };
        let key: ApiKey = row.into();
        assert_eq!(key.key_hash, "abc123hash");
        assert_eq!(key.description, Some("Production Key".to_string()));
        assert_eq!(key.created_at, 1_700_000_000);
        assert_eq!(key.last_used_at, Some(1_700_000_500));
        assert!(key.is_active);
    }

    #[test]
    fn test_api_key_row_to_api_key_no_description_no_last_used() {
        let row = ApiKeyRow {
            key_hash: "nohash".to_string(),
            description: None,
            created_at: 1_700_000_000,
            last_used_at: None,
            is_active: 0,
        };
        let key: ApiKey = row.into();
        assert!(key.description.is_none());
        assert!(key.last_used_at.is_none());
        assert!(!key.is_active);
    }

    #[test]
    fn test_last_used_at_datetime_none_when_never_used() {
        let key = ApiKey {
            key_hash: "hash".to_string(),
            description: None,
            created_at: 1_700_000_000,
            last_used_at: None,
            is_active: true,
        };
        assert!(
            key.last_used_at_datetime().is_none(),
            "last_used_at_datetime should be None when last_used_at is None"
        );
    }

    #[test]
    fn test_last_used_at_datetime_some_returns_correct_datetime() {
        let key = ApiKey {
            key_hash: "hash".to_string(),
            description: None,
            created_at: 1_700_000_000,
            last_used_at: Some(1_700_000_500),
            is_active: true,
        };
        let dt = key.last_used_at_datetime();
        assert!(
            dt.is_some(),
            "last_used_at_datetime should be Some when last_used_at is set"
        );
        assert_eq!(dt.unwrap().timestamp(), 1_700_000_500);
    }

    #[test]
    fn test_api_key_info_from_api_key() {
        // With last_used_at
        let key = ApiKey {
            key_hash: "myhash".to_string(),
            description: Some("Info Key".to_string()),
            created_at: 1_700_000_000,
            last_used_at: Some(1_700_000_999),
            is_active: true,
        };
        let info: ApiKeyInfo = key.into();
        assert_eq!(info.key_hash, "myhash");
        assert_eq!(info.description, Some("Info Key".to_string()));
        assert_eq!(info.created_at.timestamp(), 1_700_000_000);
        assert_eq!(info.last_used_at.unwrap().timestamp(), 1_700_000_999);
        assert!(info.is_active);

        // Without last_used_at
        let key2 = ApiKey {
            key_hash: "anotherhash".to_string(),
            description: None,
            created_at: 1_700_000_000,
            last_used_at: None,
            is_active: false,
        };
        let info2: ApiKeyInfo = key2.into();
        assert!(info2.description.is_none());
        assert!(info2.last_used_at.is_none());
        assert!(!info2.is_active);
    }
}
