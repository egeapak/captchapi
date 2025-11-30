use crate::config::Config;
use crate::error::{AppError, Result};
use crate::metrics::Metrics;
use crate::middleware::AuthMiddleware;
use crate::models::{
    CreateSessionRequest, CreateSessionResponse, GetSessionDetailsResponse, Session,
    ValidateSessionRequest, ValidateSessionResponse,
};
use crate::services::{CaptchaService, StorageService};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue},
    middleware,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;

#[derive(Clone)]
pub struct SessionsState {
    pub storage: StorageService,
    pub captcha: Arc<CaptchaService>,
    pub config: Arc<Config>,
    pub metrics: Arc<Metrics>,
}

/// Creates session routes with authentication middleware.
/// Rate limiting should be applied externally via tower_governor layer.
pub fn sessions_routes(state: SessionsState, auth_middleware: AuthMiddleware) -> Router {
    Router::new()
        .route("/", post(create_session))
        .route("/{id}/validate", post(validate_session))
        .route("/{id}", delete(delete_session))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware.clone(),
            AuthMiddleware::authenticate,
        ))
        // Public endpoints (no authentication required)
        .route("/{id}", get(get_session_details))
        .route("/{id}/image.jpeg", get(get_image_binary))
        .with_state(state)
}

#[tracing::instrument(skip(state, req), fields(
    difficulty = req.difficulty.unwrap_or(5),
    width = req.width.unwrap_or(220),
    height = req.height.unwrap_or(120),
    dark_mode = req.dark_mode.unwrap_or(false),
    expires_in_seconds = req.expires_in_seconds.unwrap_or(state.config.default_session_ttl_seconds),
    session_id
))]
async fn create_session(
    State(state): State<SessionsState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateSessionResponse>)> {
    // Validate parameters
    let expires_in = req
        .expires_in_seconds
        .unwrap_or(state.config.default_session_ttl_seconds);

    if expires_in > state.config.max_session_ttl_seconds {
        return Err(AppError::InvalidSessionParams(format!(
            "expires_in_seconds cannot exceed {} seconds",
            state.config.max_session_ttl_seconds
        )));
    }

    let difficulty = req.difficulty.unwrap_or(5);
    if !(1..=10).contains(&difficulty) {
        return Err(AppError::InvalidSessionParams(
            "difficulty must be between 1 and 10".to_string(),
        ));
    }

    // Validate custom text if provided
    if let Some(ref text) = req.text {
        if text.is_empty() {
            return Err(AppError::InvalidSessionParams(
                "text cannot be empty".to_string(),
            ));
        }
        if text.len() > 20 {
            return Err(AppError::InvalidSessionParams(
                "text cannot exceed 20 characters".to_string(),
            ));
        }
        // Ensure text contains only alphanumeric characters
        if !text.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(AppError::InvalidSessionParams(
                "text must contain only alphanumeric characters".to_string(),
            ));
        }
    }

    let width = req.width.unwrap_or(220);
    if !(50..=1000).contains(&width) {
        return Err(AppError::InvalidSessionParams(
            "width must be between 50 and 1000 pixels".to_string(),
        ));
    }

    let height = req.height.unwrap_or(120);
    if !(30..=500).contains(&height) {
        return Err(AppError::InvalidSessionParams(
            "height must be between 30 and 500 pixels".to_string(),
        ));
    }

    let dark_mode = req.dark_mode.unwrap_or(false);
    let compression = state.config.captcha_compression;

    // Generate CAPTCHA (record duration)
    let start = std::time::Instant::now();
    let (text, image_bytes) = state.captcha.generate(
        req.text,
        difficulty,
        width,
        height,
        dark_mode,
        compression.into(),
    )?;
    let generation_duration = start.elapsed().as_secs_f64();
    state
        .metrics
        .performance
        .captcha_generation_duration
        .record(generation_duration, &[]);

    // Create session
    let session = Session::new(
        text,
        image_bytes,
        expires_in,
        difficulty,
        width,
        height,
        dark_mode,
    );

    // Record session_id in the span
    tracing::Span::current().record("session_id", session.id.as_str());

    // Save to database
    state.storage.create_session(&session).await?;

    // Record metrics
    state.metrics.sessions.created.add(1, &[]);

    tracing::info!("Created session: {}", session.id);

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateSessionResponse {
            session_id: session.id.clone(),
            expires_at: session.expires_at_datetime(),
            created_at: session.created_at_datetime(),
        }),
    ))
}

#[tracing::instrument(skip(state), fields(session_id = %session_id, is_expired))]
async fn get_session_details(
    State(state): State<SessionsState>,
    Path(session_id): Path<String>,
) -> Result<Json<GetSessionDetailsResponse>> {
    // Get session from database
    let session = state
        .storage
        .get_session(&session_id)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    // Check if expired
    if session.is_expired() {
        tracing::Span::current().record("is_expired", true);
        // Delete expired session
        let _ = state.storage.delete_session(&session_id).await;
        return Err(AppError::SessionNotFound);
    }

    tracing::Span::current().record("is_expired", false);

    // Return session details without image data
    Ok(Json(GetSessionDetailsResponse {
        session_id: session.id.clone(),
        created_at: session.created_at_datetime(),
        expires_at: session.expires_at_datetime(),
        attempt_count: session.attempt_count,
        difficulty: session.difficulty,
        width: session.width,
        height: session.height,
        dark_mode: session.dark_mode,
    }))
}

#[tracing::instrument(skip(state), fields(session_id = %session_id, is_expired, image_size_bytes))]
async fn get_image_binary(
    State(state): State<SessionsState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse> {
    // Get session from database
    let session = state
        .storage
        .get_session(&session_id)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    // Check if expired
    if session.is_expired() {
        tracing::Span::current().record("is_expired", true);
        // Delete expired session
        let _ = state.storage.delete_session(&session_id).await;
        return Err(AppError::SessionNotFound);
    }

    tracing::Span::current().record("is_expired", false);
    tracing::Span::current().record("image_size_bytes", session.image_bytes.len());

    // Calculate cache duration (time until expiration)
    let now = Utc::now().timestamp();
    let max_age = (session.expires_at - now).max(0);

    // Build response with proper headers
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", session_id))
            .unwrap_or_else(|_| HeaderValue::from_static("\"unknown\"")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={}", max_age))
            .unwrap_or_else(|_| HeaderValue::from_static("public, max-age=0")),
    );
    headers.insert(
        header::EXPIRES,
        HeaderValue::from_str(&session.expires_at_datetime().to_rfc2822())
            .unwrap_or_else(|_| HeaderValue::from_static("")),
    );

    // Return raw JPEG bytes directly (no base64 encoding/decoding needed!)
    Ok((headers, session.image_bytes))
}

#[tracing::instrument(skip(state, req), fields(
    session_id = %session_id,
    is_valid,
    is_expired,
    attempt_count,
    max_attempts_reached
))]
async fn validate_session(
    State(state): State<SessionsState>,
    Path(session_id): Path<String>,
    Json(req): Json<ValidateSessionRequest>,
) -> Result<Json<ValidateSessionResponse>> {
    // Get session from database
    let session = state
        .storage
        .get_session(&session_id)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    tracing::Span::current().record("attempt_count", session.attempt_count);

    // Check if expired
    if session.is_expired() {
        tracing::Span::current().record("is_expired", true);
        let _ = state.storage.delete_session(&session_id).await;
        return Err(AppError::SessionNotFound);
    }

    tracing::Span::current().record("is_expired", false);

    // Check attempt count
    if session.attempt_count >= state.config.max_validation_attempts {
        tracing::Span::current().record("max_attempts_reached", true);
        // Delete session after max attempts
        let _ = state.storage.delete_session(&session_id).await;

        // Record max attempts exceeded
        state.metrics.sessions.max_attempts_exceeded.add(1, &[]);

        return Ok(Json(ValidateSessionResponse {
            valid: false,
            session_id,
        }));
    }

    tracing::Span::current().record("max_attempts_reached", false);

    // Validate solution (case-insensitive)
    let is_valid = session.solution == req.solution.to_lowercase();
    tracing::Span::current().record("is_valid", is_valid);

    // Record validation attempt
    state.metrics.sessions.validation_attempts.add(1, &[]);

    if is_valid {
        // Delete session on successful validation
        state.storage.delete_session(&session_id).await?;

        // Record successful validation
        state.metrics.sessions.validated.add(1, &[]);

        tracing::info!("Session {} validated successfully", session_id);
    } else {
        // Increment attempt count on failure
        state.storage.increment_attempt_count(&session_id).await?;

        // Record failed validation
        state.metrics.sessions.validation_failed.add(1, &[]);

        tracing::debug!(
            "Session {} validation failed, attempt {}/{}",
            session_id,
            session.attempt_count + 1,
            state.config.max_validation_attempts
        );
    }

    Ok(Json(ValidateSessionResponse {
        valid: is_valid,
        session_id,
    }))
}

#[tracing::instrument(skip(state), fields(session_id = %session_id, deleted))]
async fn delete_session(
    State(state): State<SessionsState>,
    Path(session_id): Path<String>,
) -> Result<axum::http::StatusCode> {
    let deleted = state.storage.delete_session(&session_id).await?;

    tracing::Span::current().record("deleted", deleted);

    if deleted {
        // Record deletion metric
        state.metrics.sessions.deleted.add(1, &[]);

        tracing::info!("Deleted session: {}", session_id);
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::SessionNotFound)
    }
}
