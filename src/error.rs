use crate::metrics::Metrics;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::LazyLock;
use thiserror::Error;

/// Lazily-initialized metrics instance for error tracking.
/// This avoids creating a new Metrics struct on every error response.
static ERROR_METRICS: LazyLock<Metrics> = LazyLock::new(Metrics::new);

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Session not found or expired")]
    SessionNotFound,

    #[error("API key not found")]
    ApiKeyNotFound,

    #[error("Invalid session parameters: {0}")]
    InvalidSessionParams(String),

    #[error("Invalid API key parameters: {0}")]
    InvalidApiKeyParams(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("CAPTCHA generation failed: {0}")]
    #[allow(dead_code)]
    CaptchaGeneration(String),

    #[error("Internal server error")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Use lazily-initialized static metrics instance
        let metrics = &*ERROR_METRICS;

        let (status, error_code, message) = match self {
            AppError::Database(ref e) => {
                tracing::error!("Database error: {:?}", e);
                // Track database error
                metrics.errors.database_errors.add(1, &[]);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "An internal database error occurred".to_string(),
                )
            }
            AppError::SessionNotFound => (
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session does not exist or has expired".to_string(),
            ),
            AppError::ApiKeyNotFound => (
                StatusCode::NOT_FOUND,
                "api_key_not_found",
                "API key does not exist".to_string(),
            ),
            AppError::InvalidSessionParams(ref msg) => {
                (StatusCode::BAD_REQUEST, "invalid_parameters", msg.clone())
            }
            AppError::InvalidApiKeyParams(ref msg) => (
                StatusCode::BAD_REQUEST,
                "invalid_api_key_parameters",
                msg.clone(),
            ),
            AppError::Unauthorized(ref msg) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone())
            }
            AppError::RateLimitExceeded => {
                // Track rate limit exceeded
                metrics.rate_limit.requests_blocked.add(1, &[]);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limit_exceeded",
                    "Rate limit exceeded. Please try again later.".to_string(),
                )
            }
            AppError::CaptchaGeneration(ref msg) => {
                tracing::error!("CAPTCHA generation error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "captcha_generation_failed",
                    "Failed to generate CAPTCHA".to_string(),
                )
            }
            AppError::Internal(ref e) => {
                tracing::error!("Internal error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred".to_string(),
                )
            }
        };

        // Track HTTP errors (4xx and 5xx status codes)
        if status.is_client_error() || status.is_server_error() {
            metrics.errors.http_errors_total.add(1, &[]);
        }

        let body = Json(json!({
            "error": error_code,
            "message": message,
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_not_found_status_code() {
        let error = AppError::SessionNotFound;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_api_key_not_found_status_code() {
        let error = AppError::ApiKeyNotFound;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_unauthorized_status_code() {
        let error = AppError::Unauthorized("Invalid key".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_invalid_params_status_code() {
        let error = AppError::InvalidSessionParams("Bad TTL".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_invalid_api_key_params_status_code() {
        let error = AppError::InvalidApiKeyParams("Description too long".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_database_error_status_code() {
        let error = AppError::Database(sqlx::Error::RowNotFound);
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_captcha_generation_status_code() {
        let error = AppError::CaptchaGeneration("Test error".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_internal_error_status_code() {
        let error = AppError::Internal(anyhow::anyhow!("Test error"));
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_session_not_found_error_code() {
        let error = AppError::SessionNotFound;
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "session_not_found");
        assert_eq!(body["message"], "Session does not exist or has expired");
    }

    #[test]
    fn test_api_key_not_found_error_code() {
        let error = AppError::ApiKeyNotFound;
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "api_key_not_found");
        assert_eq!(body["message"], "API key does not exist");
    }

    #[test]
    fn test_unauthorized_error_code() {
        let error = AppError::Unauthorized("Invalid API key".to_string());
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "unauthorized");
        assert_eq!(body["message"], "Invalid API key");
    }

    #[test]
    fn test_rate_limit_exceeded_status_code() {
        let error = AppError::RateLimitExceeded;
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn test_rate_limit_exceeded_error_code() {
        let error = AppError::RateLimitExceeded;
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "rate_limit_exceeded");
        assert_eq!(
            body["message"],
            "Rate limit exceeded. Please try again later."
        );
    }

    #[test]
    fn test_invalid_params_error_code() {
        let error = AppError::InvalidSessionParams("TTL exceeds maximum".to_string());
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "invalid_parameters");
        assert_eq!(body["message"], "TTL exceeds maximum");
    }

    #[test]
    fn test_invalid_api_key_params_error_code() {
        let error = AppError::InvalidApiKeyParams("Description exceeds 255 characters".to_string());
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "invalid_api_key_parameters");
        assert_eq!(body["message"], "Description exceeds 255 characters");
    }

    #[test]
    fn test_database_error_returns_generic_message() {
        let error = AppError::Database(sqlx::Error::RowNotFound);
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "database_error");
        assert_eq!(body["message"], "An internal database error occurred");
        // Should not expose internal SQL details
        assert!(!body["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("sql"));
    }

    #[test]
    fn test_internal_error_returns_generic_message() {
        let error = AppError::Internal(anyhow::anyhow!("Secret internal error"));
        let response = error.into_response();
        let body = extract_body_json(response);

        assert_eq!(body["error"], "internal_error");
        assert_eq!(body["message"], "An internal error occurred");
        // Should not expose internal details
        assert!(!body["message"].as_str().unwrap().contains("Secret"));
    }

    // Helper function to extract JSON body from response
    fn extract_body_json(response: Response) -> serde_json::Value {
        use axum::body::to_bytes;

        let (_parts, body) = response.into_parts();
        let body_bytes = tokio_test::block_on(async {
            to_bytes(body, usize::MAX)
                .await
                .expect("Failed to read body")
        });

        serde_json::from_slice(&body_bytes).expect("Failed to parse JSON")
    }
}
