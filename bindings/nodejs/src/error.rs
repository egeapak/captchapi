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
        AppError::Forbidden(msg) => {
            napi::Error::new(Status::GenericFailure, format!("Forbidden: {}", msg))
        }
        AppError::ConfigNotReloadable(msg) => napi::Error::new(
            Status::InvalidArg,
            format!("Configuration field is not reloadable: {}", msg),
        ),
        AppError::ConfigNotPersistable(msg) => napi::Error::new(
            Status::InvalidArg,
            format!("Configuration field cannot be stored: {}", msg),
        ),
        AppError::ConfigPinned(msg) => napi::Error::new(
            Status::InvalidArg,
            format!(
                "Configuration field is pinned by the environment it was started in: {}",
                msg
            ),
        ),
        AppError::RestartNotEnabled(msg) => napi::Error::new(
            Status::GenericFailure,
            format!("Restarting from the API is not enabled: {}", msg),
        ),
        AppError::AddressUnavailable(msg) => napi::Error::new(
            Status::GenericFailure,
            format!("Address unavailable: {}", msg),
        ),
        AppError::InvalidConfig(msg) => napi::Error::new(
            Status::InvalidArg,
            format!("Invalid configuration: {}", msg),
        ),
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
