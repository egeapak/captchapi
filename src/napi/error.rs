//! Error handling for NAPI bindings
//!
//! Converts Rust errors to NAPI errors that can be thrown in JavaScript.

use crate::error::AppError;
use napi::Status;

/// Convert AppError to napi::Error
impl From<AppError> for napi::Error {
    fn from(err: AppError) -> Self {
        match err {
            AppError::Database(e) => {
                napi::Error::new(Status::GenericFailure, format!("Database error: {}", e))
            }
            AppError::SessionNotFound => {
                napi::Error::new(Status::GenericFailure, "Session not found or expired")
            }
            AppError::ApiKeyNotFound => {
                napi::Error::new(Status::GenericFailure, "API key not found")
            }
            AppError::InvalidSessionParams(msg) => {
                napi::Error::new(Status::InvalidArg, format!("Invalid parameters: {}", msg))
            }
            AppError::InvalidApiKeyParams(msg) => napi::Error::new(
                Status::InvalidArg,
                format!("Invalid API key parameters: {}", msg),
            ),
            AppError::Unauthorized(msg) => {
                napi::Error::new(Status::GenericFailure, format!("Unauthorized: {}", msg))
            }
            AppError::CaptchaGeneration(msg) => napi::Error::new(
                Status::GenericFailure,
                format!("CAPTCHA generation failed: {}", msg),
            ),
            AppError::Internal(e) => {
                napi::Error::new(Status::GenericFailure, format!("Internal error: {}", e))
            }
        }
    }
}

/// Extension trait for converting Results with AppError
pub trait IntoNapiResult<T> {
    fn into_napi(self) -> napi::Result<T>;
}

impl<T> IntoNapiResult<T> for Result<T, AppError> {
    fn into_napi(self) -> napi::Result<T> {
        self.map_err(|e| e.into())
    }
}
