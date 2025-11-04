use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Maximum length for API key descriptions
pub const MAX_DESCRIPTION_LENGTH: usize = 255;

/// Validates an API key description
///
/// Returns Ok(()) if valid, or Err with a descriptive error message if invalid
pub fn validate_description(description: &Option<String>) -> Result<(), String> {
    if let Some(desc) = description {
        // Check if empty/whitespace only
        if desc.trim().is_empty() {
            return Err("Description cannot be empty or whitespace only".to_string());
        }

        // Check length
        if desc.len() > MAX_DESCRIPTION_LENGTH {
            return Err(format!(
                "Description exceeds maximum length of {} characters",
                MAX_DESCRIPTION_LENGTH
            ));
        }

        // Check for control characters (except newlines and tabs which are acceptable)
        if desc
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r')
        {
            return Err("Description contains invalid control characters".to_string());
        }
    }

    Ok(())
}

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

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub api_key: String,
    pub key_hash: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyInfo {
    pub key_hash: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub is_active: bool,
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
pub struct UpdateApiKeyRequest {
    pub is_active: Option<bool>,
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_description_none() {
        assert!(validate_description(&None).is_ok());
    }

    #[test]
    fn test_validate_description_valid() {
        assert!(validate_description(&Some("Test API Key".to_string())).is_ok());
        assert!(validate_description(&Some("Production Key - Web App".to_string())).is_ok());
        assert!(validate_description(&Some("Key with\nnewlines\nand\ttabs".to_string())).is_ok());
    }

    #[test]
    fn test_validate_description_empty_string() {
        let result = validate_description(&Some("".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty or whitespace"));
    }

    #[test]
    fn test_validate_description_whitespace_only() {
        let result = validate_description(&Some("   \t\n   ".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty or whitespace"));
    }

    #[test]
    fn test_validate_description_too_long() {
        let long_desc = "a".repeat(256);
        let result = validate_description(&Some(long_desc));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum length"));
    }

    #[test]
    fn test_validate_description_exactly_max_length() {
        let desc = "a".repeat(MAX_DESCRIPTION_LENGTH);
        assert!(validate_description(&Some(desc)).is_ok());
    }

    #[test]
    fn test_validate_description_control_characters() {
        // Null byte
        let result = validate_description(&Some("Test\0Key".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("control characters"));

        // Bell character
        let result = validate_description(&Some("Test\x07Key".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("control characters"));
    }

    #[test]
    fn test_validate_description_allows_common_whitespace() {
        // Newlines, tabs, carriage returns are allowed
        assert!(validate_description(&Some("Line 1\nLine 2".to_string())).is_ok());
        assert!(validate_description(&Some("Tab\there".to_string())).is_ok());
        assert!(validate_description(&Some("Windows\r\nNewline".to_string())).is_ok());
    }
}
