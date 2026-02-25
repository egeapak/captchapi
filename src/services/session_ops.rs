//! Session orchestration: shared business logic for HTTP and NAPI layers.

use crate::error::{AppError, Result};
use crate::metrics::Metrics;
use crate::models::Session;
use crate::services::{CaptchaService, StorageService};
use crate::validation::ValidatedSessionParams;
use std::sync::Arc;

/// Outcome of a validate-session operation.
#[derive(Debug)]
pub enum ValidationOutcome {
    /// Solution was correct; session has been deleted.
    Correct,
    /// Solution was wrong; attempt count was incremented.
    Wrong {
        /// Number of attempts remaining before the session is locked.
        /// Used by the NAPI bindings layer.
        #[allow(dead_code)]
        attempts_remaining: i64,
    },
    /// Max attempts were already exceeded; session has been deleted.
    MaxAttemptsExceeded,
}

/// Create a CAPTCHA session: generate image → build model → persist → record metrics.
pub async fn create_session_orchestrated(
    storage: &StorageService,
    captcha: &CaptchaService,
    metrics: &Arc<Metrics>,
    params: ValidatedSessionParams,
) -> Result<(Session, Vec<u8>)> {
    let start = std::time::Instant::now();
    let (text, image_bytes) = captcha.generate(
        params.length,
        params.difficulty,
        params.width,
        params.height,
        params.dark_mode,
        params.compression,
    )?;
    let generation_duration = start.elapsed().as_secs_f64();
    metrics
        .performance
        .captcha_generation_duration
        .record(generation_duration, &[]);

    let session = Session::new(
        text,
        image_bytes.clone(),
        params.expires_in,
        params.difficulty,
        params.width,
        params.height,
        params.dark_mode,
    );

    storage.create_session(&session).await?;
    metrics.sessions.created.add(1, &[]);

    Ok((session, image_bytes))
}

/// Validate a CAPTCHA session solution.
/// Handles expiry check, attempt counting, and deletion on success/max-attempts.
pub async fn validate_session_orchestrated(
    storage: &StorageService,
    metrics: &Arc<Metrics>,
    session_id: &str,
    solution: &str,
    max_validation_attempts: i64,
) -> Result<ValidationOutcome> {
    let session = storage
        .get_active_session(session_id)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    // Check attempt count
    if session.attempt_count >= max_validation_attempts {
        if let Err(e) = storage.delete_session(session_id).await {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "Failed to delete session after max attempts exceeded"
            );
        }
        metrics.sessions.validation_attempts.add(1, &[]);
        metrics.sessions.max_attempts_exceeded.add(1, &[]);
        return Ok(ValidationOutcome::MaxAttemptsExceeded);
    }

    // Check solution (case-sensitive)
    let is_valid = session.solution == solution;
    metrics.sessions.validation_attempts.add(1, &[]);

    if is_valid {
        storage.delete_session(session_id).await?;
        metrics.sessions.validated.add(1, &[]);
        Ok(ValidationOutcome::Correct)
    } else {
        storage.increment_attempt_count(session_id).await?;
        metrics.sessions.validation_failed.add(1, &[]);
        let attempts_remaining = (max_validation_attempts - session.attempt_count - 1).max(0);
        Ok(ValidationOutcome::Wrong { attempts_remaining })
    }
}
