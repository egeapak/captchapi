use crate::config::ConfigHandle;
use crate::error::{AppError, Result};
use crate::metrics::Metrics;
use crate::middleware::AuthMiddleware;
use crate::models::{
    CreateSessionRequest, CreateSessionResponse, GetSessionDetailsResponse, ValidateSessionRequest,
    ValidateSessionResponse,
};
use crate::services::{
    create_session_orchestrated, validate_session_orchestrated, CaptchaService, StorageService,
    ValidationOutcome,
};
use crate::validation;
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
    pub config: ConfigHandle,
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
    length = req.length.unwrap_or(validation::DEFAULT_LENGTH),
    difficulty = req.difficulty.unwrap_or(validation::DEFAULT_DIFFICULTY),
    width = req.width.unwrap_or(validation::DEFAULT_WIDTH),
    height = req.height.unwrap_or(validation::DEFAULT_HEIGHT),
    dark_mode = req.dark_mode.unwrap_or(validation::DEFAULT_DARK_MODE),
    expires_in_seconds,
    session_id
))]
async fn create_session(
    State(state): State<SessionsState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateSessionResponse>)> {
    // One snapshot for the whole request. Reading the handle more than once would let a reload
    // landing mid-handler make the traced value disagree with the value used for validation.
    let settings = state.config.session_config();

    let expires_in_seconds = req
        .expires_in_seconds
        .unwrap_or(settings.default_session_ttl_seconds);
    tracing::Span::current().record("expires_in_seconds", expires_in_seconds);

    // Validate parameters — use client-supplied compression if provided, else fall back to
    // the server-configured default so existing behaviour is preserved.
    let compression = req.compression.or(Some(settings.captcha_compression));
    let params = validation::validate_session_params(
        req.length,
        req.difficulty,
        req.width,
        req.height,
        req.dark_mode,
        compression,
        req.expires_in_seconds,
        settings.default_session_ttl_seconds,
        settings.max_session_ttl_seconds,
    )
    .map_err(AppError::InvalidSessionParams)?;

    // Use orchestration function for generate + store + metrics
    let (session, _image_bytes) =
        create_session_orchestrated(&state.storage, &state.captcha, &state.metrics, params).await?;

    // Record session_id in the span
    tracing::Span::current().record("session_id", session.id.as_str());

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
    // get_active_session handles expiry check and eager deletion
    let session = state
        .storage
        .get_active_session(&session_id)
        .await?
        .ok_or(AppError::SessionNotFound)?;

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
    // get_active_session handles expiry check and eager deletion
    let session = state
        .storage
        .get_active_session(&session_id)
        .await?
        .ok_or(AppError::SessionNotFound)?;

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
    // Input length limit on solution to prevent abuse — validate before any DB read
    validation::validate_solution(&req.solution).map_err(AppError::InvalidSessionParams)?;

    // Use orchestration function for full validation flow
    let outcome = validate_session_orchestrated(
        &state.storage,
        &state.metrics,
        &session_id,
        &req.solution,
        state.config.session_config().max_validation_attempts,
    )
    .await?;

    match outcome {
        ValidationOutcome::Correct => {
            tracing::Span::current().record("is_valid", true);
            tracing::Span::current().record("is_expired", false);
            tracing::Span::current().record("max_attempts_reached", false);
            tracing::info!("Session {} validated successfully", session_id);
            Ok(Json(ValidateSessionResponse {
                valid: true,
                session_id,
            }))
        }
        ValidationOutcome::Wrong { .. } => {
            tracing::Span::current().record("is_valid", false);
            tracing::Span::current().record("is_expired", false);
            tracing::Span::current().record("max_attempts_reached", false);
            Ok(Json(ValidateSessionResponse {
                valid: false,
                session_id,
            }))
        }
        ValidationOutcome::MaxAttemptsExceeded => {
            tracing::Span::current().record("is_valid", false);
            tracing::Span::current().record("is_expired", false);
            tracing::Span::current().record("max_attempts_reached", true);
            Ok(Json(ValidateSessionResponse {
                valid: false,
                session_id,
            }))
        }
    }
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
