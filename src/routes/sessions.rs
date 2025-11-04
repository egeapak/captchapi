use crate::config::Config;
use crate::error::{AppError, Result};
use crate::middleware::AuthMiddleware;
use crate::models::{
    CreateSessionRequest, CreateSessionResponse, GetImageResponse, Session, ValidateSessionRequest,
    ValidateSessionResponse,
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
}

pub fn sessions_routes(state: SessionsState, auth_middleware: AuthMiddleware) -> Router {
    Router::new()
        .route("/", post(create_session))
        .route("/{id}/validate", post(validate_session))
        .route("/{id}", delete(delete_session))
        .route_layer(middleware::from_fn_with_state(
            auth_middleware.clone(),
            AuthMiddleware::authenticate,
        ))
        .route("/{id}/image", get(get_image))
        .route("/{id}/image.jpeg", get(get_image_binary))
        .with_state(state)
}

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

    let width = req.width.unwrap_or(220);
    let height = req.height.unwrap_or(120);
    let dark_mode = req.dark_mode.unwrap_or(false);
    let compression = state.config.captcha_compression;

    // Generate CAPTCHA
    let (text, image_bytes) =
        state
            .captcha
            .generate(req.text, difficulty, width, height, dark_mode, compression)?;

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

    // Save to database
    state.storage.create_session(&session).await?;

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

async fn get_image(
    State(state): State<SessionsState>,
    Path(session_id): Path<String>,
) -> Result<Json<GetImageResponse>> {
    // Get session from database
    let session = state
        .storage
        .get_session(&session_id)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    // Check if expired
    if session.is_expired() {
        // Delete expired session
        let _ = state.storage.delete_session(&session_id).await;
        return Err(AppError::SessionNotFound);
    }

    // Convert raw bytes to base64 data URI for JSON response
    let base64_image = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &session.image_bytes,
    );
    let data_uri = format!("data:image/jpeg;base64,{}", base64_image);

    Ok(Json(GetImageResponse {
        image: data_uri,
        expires_at: session.expires_at_datetime(),
    }))
}

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
        // Delete expired session
        let _ = state.storage.delete_session(&session_id).await;
        return Err(AppError::SessionNotFound);
    }

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

    // Check if expired
    if session.is_expired() {
        let _ = state.storage.delete_session(&session_id).await;
        return Err(AppError::SessionNotFound);
    }

    // Check attempt count
    if session.attempt_count >= state.config.max_validation_attempts {
        // Delete session after max attempts
        let _ = state.storage.delete_session(&session_id).await;
        return Ok(Json(ValidateSessionResponse {
            valid: false,
            session_id,
        }));
    }

    // Validate solution (case-insensitive)
    let is_valid = session.solution == req.solution.to_lowercase();

    if is_valid {
        // Delete session on successful validation
        state.storage.delete_session(&session_id).await?;
        tracing::info!("Session {} validated successfully", session_id);
    } else {
        // Increment attempt count on failure
        state.storage.increment_attempt_count(&session_id).await?;
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

async fn delete_session(
    State(state): State<SessionsState>,
    Path(session_id): Path<String>,
) -> Result<axum::http::StatusCode> {
    let deleted = state.storage.delete_session(&session_id).await?;

    if deleted {
        tracing::info!("Deleted session: {}", session_id);
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::SessionNotFound)
    }
}
