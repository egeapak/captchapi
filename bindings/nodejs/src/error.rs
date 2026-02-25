//! Error handling for NAPI bindings
//!
//! Converts Rust errors to NAPI errors that can be thrown in JavaScript.

use captchapi::error::AppError;
use napi::Status;

/// Convert AppError to napi::Error
pub fn app_error_to_napi(err: AppError) -> napi::Error {
    match err {
        AppError::Database(e) => {
            tracing::error!("Database error: {:?}", e);
            napi::Error::new(
                Status::GenericFailure,
                "An internal database error occurred",
            )
        }
        AppError::SessionNotFound => {
            napi::Error::new(Status::GenericFailure, "Session not found or expired")
        }
        AppError::ApiKeyNotFound => napi::Error::new(Status::GenericFailure, "API key not found"),
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
        AppError::Internal(e) => {
            tracing::error!("Internal error: {:?}", e);
            napi::Error::new(Status::GenericFailure, "An internal error occurred")
        }
    }
}

/// Extension trait for converting Results with AppError
pub trait IntoNapiResult<T> {
    fn into_napi(self) -> napi::Result<T>;
}

impl<T> IntoNapiResult<T> for Result<T, AppError> {
    fn into_napi(self) -> napi::Result<T> {
        self.map_err(app_error_to_napi)
    }
}
