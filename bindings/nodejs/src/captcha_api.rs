//! Main CaptchaApi class for Node.js bindings
//!
//! This provides a high-level API for CAPTCHA operations that can be
//! used directly from JavaScript/TypeScript.

use crate::error::IntoNapiResult;
use crate::types::*;
use captchapi::metrics::Metrics;
use captchapi::models::{ApiKey, Session};
use captchapi::services::{AuthService, CaptchaService, StorageService};
use captchapi::validation;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::sync::Arc;

/// Main CAPTCHA API client for Node.js
///
/// This class provides all CAPTCHA operations including session management,
/// validation, and API key management.
///
/// @example
/// ```typescript
/// const api = await CaptchaApi.create({
///   databaseUrl: 'sqlite:./captcha.db',
///   apiKeySalt: 'your-secret-salt'
/// });
///
/// const session = await api.createSession({ difficulty: 5 });
/// const isValid = await api.validate(session.sessionId, userAnswer);
/// ```
#[napi]
pub struct CaptchaApi {
    pool: SqlitePool,
    storage: StorageService,
    captcha: CaptchaService,
    auth: AuthService,
    config: Arc<InternalConfig>,
    metrics: Arc<Metrics>,
}

struct InternalConfig {
    default_session_ttl_seconds: u64,
    max_session_ttl_seconds: u64,
    max_validation_attempts: i64,
}

#[napi]
impl CaptchaApi {
    /// Create a new CaptchaApi instance
    ///
    /// This is an async factory method that initializes the database connection
    /// and runs migrations if configured.
    ///
    /// @param config - Configuration options for the API
    /// @returns A promise that resolves to a CaptchaApi instance
    #[napi(factory)]
    pub async fn create(config: CaptchaConfig) -> Result<Self> {
        let database_url = &config.database_url;

        // Parse the database URL and create connection options
        let connect_options = SqliteConnectOptions::from_str(database_url)
            .map_err(|e| napi::Error::from_reason(format!("Invalid database URL: {}", e)))?
            .create_if_missing(true);

        // Create the connection pool
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options)
            .await
            .map_err(|e| {
                napi::Error::from_reason(format!("Failed to connect to database: {}", e))
            })?;

        // Run migrations if enabled
        if config.run_migrations.unwrap_or(true) {
            sqlx::migrate!("../../migrations")
                .run(&pool)
                .await
                .map_err(|e| {
                    napi::Error::from_reason(format!("Failed to run migrations: {}", e))
                })?;
        }

        let storage = StorageService::new(pool.clone());
        let captcha = CaptchaService::new();
        let auth = AuthService::new(config.api_key_salt.clone());
        let metrics = Arc::new(Metrics::new());

        let internal_config = Arc::new(InternalConfig {
            default_session_ttl_seconds: config.default_session_ttl_seconds.unwrap_or(300) as u64,
            max_session_ttl_seconds: config.max_session_ttl_seconds.unwrap_or(3600) as u64,
            max_validation_attempts: config.max_validation_attempts.unwrap_or(3) as i64,
        });

        Ok(Self {
            pool,
            storage,
            captcha,
            auth,
            config: internal_config,
            metrics,
        })
    }

    /// Create a new CAPTCHA session
    ///
    /// Generates a CAPTCHA image and stores the session in the database.
    /// The session will expire after the configured TTL.
    ///
    /// @param options - Optional settings for the CAPTCHA
    /// @returns Session information including the image
    #[napi]
    pub async fn create_session(
        &self,
        options: Option<CreateSessionOptions>,
    ) -> Result<SessionResult> {
        let opts = options.unwrap_or_default();

        // Validate parameters using shared validation
        let params = validation::validate_session_params(
            opts.length.map(|v| v as i64),
            opts.difficulty.map(|v| v as i64),
            opts.width.map(|v| v as i64),
            opts.height.map(|v| v as i64),
            opts.dark_mode,
            opts.compression.map(|v| v as i64),
            opts.expires_in_seconds.map(|v| v as u64),
            self.config.default_session_ttl_seconds,
            self.config.max_session_ttl_seconds,
        )
        .map_err(napi::Error::from_reason)?;

        // Generate CAPTCHA (record duration)
        let start = std::time::Instant::now();
        let (solution, image_bytes) = self
            .captcha
            .generate(
                params.length,
                params.difficulty,
                params.width,
                params.height,
                params.dark_mode,
                params.compression,
            )
            .into_napi()?;
        let generation_duration = start.elapsed().as_secs_f64();
        self.metrics
            .performance
            .captcha_generation_duration
            .record(generation_duration, &[]);

        // Create session
        let session = Session::new(
            solution.clone(),
            image_bytes.clone(),
            params.expires_in,
            params.difficulty,
            params.width,
            params.height,
            params.dark_mode,
        );

        // Store session
        self.storage.create_session(&session).await.into_napi()?;

        self.metrics.sessions.created.add(1, &[]);

        Ok(SessionResult {
            session_id: session.id,
            text: solution,
            created_at: session.created_at * 1000, // Convert to milliseconds
            expires_at: session.expires_at * 1000,
            image: Buffer::from(image_bytes),
        })
    }

    /// Validate a CAPTCHA solution
    ///
    /// Checks if the provided solution matches the session's expected answer.
    /// The session is deleted on successful validation.
    /// After max attempts, the session becomes invalid.
    ///
    /// @param sessionId - The session ID to validate
    /// @param solution - The user's answer
    /// @returns Validation result
    #[napi]
    pub async fn validate(&self, session_id: String, solution: String) -> Result<ValidationResult> {
        // Input length limit on solution to prevent abuse
        validation::validate_solution(&solution).map_err(napi::Error::from_reason)?;

        // Get session
        let session = self
            .storage
            .get_session(&session_id)
            .await
            .into_napi()?
            .ok_or_else(|| napi::Error::from_reason("Session not found or expired"))?;

        // Check if expired
        if session.is_expired() {
            self.storage.delete_session(&session_id).await.into_napi()?;
            return Err(napi::Error::from_reason("Session has expired"));
        }

        // Check attempt count
        if session.attempt_count >= self.config.max_validation_attempts {
            self.storage.delete_session(&session_id).await.into_napi()?;

            self.metrics.sessions.validation_attempts.add(1, &[]);
            self.metrics.sessions.max_attempts_exceeded.add(1, &[]);

            return Ok(ValidationResult {
                valid: false,
                session_id,
                attempts_remaining: 0,
            });
        }

        // Check solution (case-sensitive, matching HTTP API behavior)
        let is_valid = session.solution == solution;

        self.metrics.sessions.validation_attempts.add(1, &[]);

        if is_valid {
            // Delete session on success
            self.storage.delete_session(&session_id).await.into_napi()?;

            self.metrics.sessions.validated.add(1, &[]);

            Ok(ValidationResult {
                valid: true,
                session_id,
                attempts_remaining: 0,
            })
        } else {
            // Increment attempt count
            self.storage
                .increment_attempt_count(&session_id)
                .await
                .into_napi()?;

            self.metrics.sessions.validation_failed.add(1, &[]);

            let attempts_remaining =
                (self.config.max_validation_attempts - session.attempt_count - 1).max(0) as i32;

            Ok(ValidationResult {
                valid: false,
                session_id,
                attempts_remaining,
            })
        }
    }

    /// Get the CAPTCHA image for a session
    ///
    /// @param sessionId - The session ID
    /// @returns The JPEG image as a Buffer
    #[napi]
    pub async fn get_image(&self, session_id: String) -> Result<Buffer> {
        let session = self
            .storage
            .get_session(&session_id)
            .await
            .into_napi()?
            .ok_or_else(|| napi::Error::from_reason("Session not found or expired"))?;

        if session.is_expired() {
            // Delete expired session before returning error
            if let Err(e) = self.storage.delete_session(&session_id).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Failed to delete expired session during eager cleanup"
                );
            }
            return Err(napi::Error::from_reason("Session not found or expired"));
        }

        Ok(Buffer::from(session.image_bytes))
    }

    /// Get session information (without the image)
    ///
    /// @param sessionId - The session ID
    /// @returns Session metadata
    #[napi]
    pub async fn get_session(&self, session_id: String) -> Result<SessionInfo> {
        let session = self
            .storage
            .get_session(&session_id)
            .await
            .into_napi()?
            .ok_or_else(|| napi::Error::from_reason("Session not found or expired"))?;

        // Check if expired
        if session.is_expired() {
            // Delete expired session before returning error
            if let Err(e) = self.storage.delete_session(&session_id).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "Failed to delete expired session during eager cleanup"
                );
            }
            return Err(napi::Error::from_reason("Session not found or expired"));
        }

        Ok(SessionInfo {
            session_id: session.id,
            created_at: session.created_at * 1000,
            expires_at: session.expires_at * 1000,
            attempt_count: session.attempt_count as i32,
            difficulty: session.difficulty as i32,
            width: session.width as i32,
            height: session.height as i32,
            dark_mode: session.dark_mode,
        })
    }

    /// Delete a session
    ///
    /// @param sessionId - The session ID to delete
    /// @returns True if the session was deleted, false if not found
    #[napi]
    pub async fn delete_session(&self, session_id: String) -> Result<bool> {
        let deleted = self.storage.delete_session(&session_id).await.into_napi()?;
        if deleted {
            self.metrics.sessions.deleted.add(1, &[]);
        }
        Ok(deleted)
    }

    /// Delete all expired sessions
    ///
    /// This is useful for cleanup. In the HTTP server, this runs automatically
    /// in a background task.
    ///
    /// @returns Number of sessions deleted
    #[napi]
    pub async fn cleanup_expired(&self) -> Result<u32> {
        let count = self.storage.delete_expired_sessions().await.into_napi()?;
        if count > 0 {
            self.metrics.sessions.expired_cleaned.add(count, &[]);
        }
        Ok(count as u32)
    }

    /// Generate a CAPTCHA without storing it (stateless)
    ///
    /// This is useful for custom implementations where you want to manage
    /// storage yourself.
    ///
    /// @param options - Optional settings for the CAPTCHA
    /// @returns The solution and image
    #[napi]
    pub fn generate(&self, options: Option<GenerateOptions>) -> Result<GenerateResult> {
        let opts = options.unwrap_or_default();

        let params = validation::validate_session_params(
            opts.length.map(|v| v as i64),
            opts.difficulty.map(|v| v as i64),
            opts.width.map(|v| v as i64),
            opts.height.map(|v| v as i64),
            opts.dark_mode,
            opts.compression.map(|v| v as i64),
            None,
            0,
            u64::MAX,
        )
        .map_err(napi::Error::from_reason)?;

        let (solution, image_bytes) = self
            .captcha
            .generate(
                params.length,
                params.difficulty,
                params.width,
                params.height,
                params.dark_mode,
                params.compression,
            )
            .into_napi()?;

        Ok(GenerateResult {
            solution,
            image: Buffer::from(image_bytes),
        })
    }

    // === API Key Management ===

    /// Create a new API key
    ///
    /// @param description - Optional description for the key
    /// @returns The API key (store it safely!) and its hash
    #[napi]
    pub async fn create_api_key(&self, description: Option<String>) -> Result<CreateApiKeyResult> {
        // Validate description
        validation::validate_api_key_description(&description).map_err(napi::Error::from_reason)?;

        // Generate a random API key
        let api_key = validation::generate_api_key();
        let key_hash = self.auth.hash_api_key(&api_key);

        let api_key_model = ApiKey::new(key_hash.clone(), description);
        self.storage
            .create_api_key(&api_key_model)
            .await
            .into_napi()?;

        self.metrics.api_keys.created.add(1, &[]);

        Ok(CreateApiKeyResult { api_key, key_hash })
    }

    /// Validate an API key
    ///
    /// @param apiKey - The API key to validate
    /// @returns True if the key is valid and active
    #[napi]
    pub async fn validate_api_key(&self, api_key: String) -> Result<bool> {
        let key_hash = self.auth.hash_api_key(&api_key);
        let result = self.storage.get_api_key(&key_hash).await.into_napi()?;
        Ok(result.is_some())
    }

    /// Get API key information by hash
    ///
    /// @param keyHash - The hash of the API key
    /// @returns API key information if found
    #[napi]
    pub async fn get_api_key(&self, key_hash: String) -> Result<Option<ApiKeyInfo>> {
        let result = self
            .storage
            .get_api_key_by_hash(&key_hash)
            .await
            .into_napi()?;

        Ok(result.map(|k| ApiKeyInfo {
            key_hash: k.key_hash,
            description: k.description,
            created_at: k.created_at * 1000,
            last_used_at: k.last_used_at.map(|t| t * 1000),
            is_active: k.is_active,
        }))
    }

    /// List all API keys
    ///
    /// @returns List of all API keys (hashes, not the actual keys)
    #[napi]
    pub async fn list_api_keys(&self) -> Result<Vec<ApiKeyInfo>> {
        let keys = self.storage.list_api_keys().await.into_napi()?;

        self.metrics.api_keys.listed.add(1, &[]);

        Ok(keys
            .into_iter()
            .map(|k| ApiKeyInfo {
                key_hash: k.key_hash,
                description: k.description,
                created_at: k.created_at * 1000,
                last_used_at: k.last_used_at.map(|t| t * 1000),
                is_active: k.is_active,
            })
            .collect())
    }

    /// Update an API key
    ///
    /// @param keyHash - The hash of the API key to update
    /// @param isActive - New active status (optional)
    /// @param description - New description (optional)
    /// @returns True if the key was updated
    #[napi]
    pub async fn update_api_key(
        &self,
        key_hash: String,
        is_active: Option<bool>,
        description: Option<String>,
    ) -> Result<bool> {
        // Validate description if provided
        validation::validate_api_key_description(&description).map_err(napi::Error::from_reason)?;

        let updated = self
            .storage
            .update_api_key(&key_hash, is_active, description)
            .await
            .into_napi()?;

        if updated {
            self.metrics.api_keys.updated.add(1, &[]);
        }

        Ok(updated)
    }

    /// Delete an API key
    ///
    /// @param keyHash - The hash of the API key to delete
    /// @returns True if the key was deleted
    #[napi]
    pub async fn delete_api_key(&self, key_hash: String) -> Result<bool> {
        let deleted = self.storage.delete_api_key(&key_hash).await.into_napi()?;
        if deleted {
            self.metrics.api_keys.deleted.add(1, &[]);
        }
        Ok(deleted)
    }

    /// Close the database connection
    ///
    /// Call this when you're done using the API to cleanly close connections.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        self.pool.close().await;
        Ok(())
    }
}
