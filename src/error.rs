use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Session not found or expired")]
    SessionNotFound,

    #[error("Invalid session parameters: {0}")]
    InvalidSessionParams(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("CAPTCHA generation failed: {0}")]
    CaptchaGeneration(String),

    #[error("Internal server error")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            AppError::Database(ref e) => {
                tracing::error!("Database error: {:?}", e);
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
            AppError::InvalidSessionParams(ref msg) => {
                (StatusCode::BAD_REQUEST, "invalid_parameters", msg.clone())
            }
            AppError::Unauthorized(ref msg) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone())
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

        let body = Json(json!({
            "error": error_code,
            "message": message,
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
