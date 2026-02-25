pub mod api_key;
pub mod session;
pub mod session_config;

// Domain types: stable public API
pub use api_key::ApiKey;
pub use session::Session;
pub use session_config::SessionConfig;

// HTTP DTOs: crate-private (only needed by HTTP routes)
pub(crate) use api_key::{
    ApiKeyInfo, CreateApiKeyRequest, CreateApiKeyResponse, UpdateApiKeyRequest,
};
pub(crate) use session::{
    CreateSessionRequest, CreateSessionResponse, GetSessionDetailsResponse, ValidateSessionRequest,
    ValidateSessionResponse,
};

// Internal row types for database mapping (crate-private)
pub(crate) use api_key::ApiKeyRow;
pub(crate) use session::SessionRow;
