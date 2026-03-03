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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;
    use crate::models::Session;
    use crate::services::{CaptchaService, StorageService};
    use crate::validation::ValidatedSessionParams;
    use chrono::Utc;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use uuid::Uuid;

    async fn setup_test_storage() -> StorageService {
        let db_name = format!(
            "file:test_session_ops_{}?mode=memory&cache=shared",
            Uuid::new_v4()
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_name)
                    .create_if_missing(true),
            )
            .await
            .expect("Failed to create test database");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        StorageService::new(pool)
    }

    fn make_metrics() -> Arc<Metrics> {
        Arc::new(Metrics::new())
    }

    /// Insert a session directly into storage, bypassing `Session::new` so that
    /// `attempt_count` can be set to an arbitrary value.
    async fn insert_session_with_attempt_count(
        storage: &StorageService,
        solution: &str,
        attempt_count: i64,
    ) -> Session {
        let mut session = Session::new(
            solution.to_string(),
            vec![0xFF, 0xD8, 0xFF],
            3600,
            5,
            220,
            120,
            false,
        );
        session.attempt_count = attempt_count;
        storage.create_session(&session).await.unwrap();
        session
    }

    // -----------------------------------------------------------------------
    // validate_session_orchestrated — attempts_remaining value
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_wrong_solution_returns_correct_attempts_remaining() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();
        // attempt_count starts at 0, max_validation_attempts = 3
        // After one wrong attempt: attempts_remaining = 3 - 0 - 1 = 2
        let session = insert_session_with_attempt_count(&storage, "CorrectAnswer", 0).await;

        let outcome =
            validate_session_orchestrated(&storage, &metrics, &session.id, "WrongAnswer", 3)
                .await
                .unwrap();

        match outcome {
            ValidationOutcome::Wrong { attempts_remaining } => {
                assert_eq!(
                    attempts_remaining, 2,
                    "After first wrong attempt with max=3 and attempt_count=0, \
                     attempts_remaining should be 2"
                );
            }
            other => panic!("Expected ValidationOutcome::Wrong, got {:?}", other),
        }

        // Confirm the DB attempt_count was incremented to 1
        let fetched = storage.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(fetched.attempt_count, 1);
    }

    #[tokio::test]
    async fn test_wrong_solution_attempts_remaining_decrements_with_existing_attempts() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();
        // attempt_count starts at 1, max = 3 → after this wrong attempt: 3 - 1 - 1 = 1
        let session = insert_session_with_attempt_count(&storage, "CorrectAnswer", 1).await;

        let outcome =
            validate_session_orchestrated(&storage, &metrics, &session.id, "WrongAnswer", 3)
                .await
                .unwrap();

        match outcome {
            ValidationOutcome::Wrong { attempts_remaining } => {
                assert_eq!(
                    attempts_remaining, 1,
                    "With attempt_count=1 and max=3, attempts_remaining should be 1"
                );
            }
            other => panic!("Expected ValidationOutcome::Wrong, got {:?}", other),
        }

        // Confirm the DB attempt_count was incremented from 1 to 2
        let fetched = storage.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(fetched.attempt_count, 2);
    }

    #[tokio::test]
    async fn test_wrong_solution_last_attempt_attempts_remaining_is_zero() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();
        // attempt_count starts at 2, max = 3 → this is the last allowed attempt: 3 - 2 - 1 = 0
        let session = insert_session_with_attempt_count(&storage, "CorrectAnswer", 2).await;

        let outcome =
            validate_session_orchestrated(&storage, &metrics, &session.id, "WrongAnswer", 3)
                .await
                .unwrap();

        match outcome {
            ValidationOutcome::Wrong { attempts_remaining } => {
                assert_eq!(
                    attempts_remaining, 0,
                    "On the last allowed wrong attempt (attempt_count=2, max=3), \
                     attempts_remaining should be 0"
                );
            }
            other => panic!("Expected ValidationOutcome::Wrong, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // validate_session_orchestrated — max attempts exceeded
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_max_attempts_exceeded_returns_correct_outcome_and_deletes_session() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();
        // attempt_count is already at max (3); the session must be treated as exhausted
        let session = insert_session_with_attempt_count(&storage, "CorrectAnswer", 3).await;

        let outcome =
            validate_session_orchestrated(&storage, &metrics, &session.id, "AnyAnswer", 3)
                .await
                .unwrap();

        assert!(
            matches!(outcome, ValidationOutcome::MaxAttemptsExceeded),
            "Expected MaxAttemptsExceeded when attempt_count == max_validation_attempts"
        );

        // Session must have been deleted
        let fetched = storage.get_session(&session.id).await.unwrap();
        assert!(
            fetched.is_none(),
            "Session should be deleted after max attempts exceeded"
        );
    }

    #[tokio::test]
    async fn test_max_attempts_exceeded_even_with_correct_solution() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();
        // Even providing the exact correct solution must not validate when attempts are maxed
        let session = insert_session_with_attempt_count(&storage, "ExactAnswer", 3).await;

        let outcome =
            validate_session_orchestrated(&storage, &metrics, &session.id, "ExactAnswer", 3)
                .await
                .unwrap();

        assert!(
            matches!(outcome, ValidationOutcome::MaxAttemptsExceeded),
            "Max-attempts check must run before solution check"
        );

        let fetched = storage.get_session(&session.id).await.unwrap();
        assert!(fetched.is_none(), "Session should be deleted");
    }

    // -----------------------------------------------------------------------
    // validate_session_orchestrated — correct solution
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_correct_solution_returns_correct_outcome_and_deletes_session() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();
        let session = insert_session_with_attempt_count(&storage, "RightAnswer", 0).await;

        let outcome =
            validate_session_orchestrated(&storage, &metrics, &session.id, "RightAnswer", 3)
                .await
                .unwrap();

        assert!(
            matches!(outcome, ValidationOutcome::Correct),
            "Correct solution should return ValidationOutcome::Correct"
        );

        // Session must be deleted after successful validation
        let fetched = storage.get_session(&session.id).await.unwrap();
        assert!(
            fetched.is_none(),
            "Session should be deleted after correct validation"
        );
    }

    #[tokio::test]
    async fn test_correct_solution_is_case_sensitive() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();
        let session = insert_session_with_attempt_count(&storage, "CaseSensitive", 0).await;

        // Lowercase version of the solution must not match
        let outcome =
            validate_session_orchestrated(&storage, &metrics, &session.id, "casesensitive", 3)
                .await
                .unwrap();

        assert!(
            matches!(outcome, ValidationOutcome::Wrong { .. }),
            "Solution comparison should be case-sensitive"
        );

        // Session must still exist (not deleted on wrong attempt)
        let fetched = storage.get_session(&session.id).await.unwrap();
        assert!(
            fetched.is_some(),
            "Session should still exist after a wrong (case-mismatch) attempt"
        );
    }

    // -----------------------------------------------------------------------
    // validate_session_orchestrated — session not found
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_nonexistent_session_returns_session_not_found_error() {
        let storage = setup_test_storage().await;
        let metrics = make_metrics();

        let result = validate_session_orchestrated(
            &storage,
            &metrics,
            "00000000-0000-0000-0000-000000000000",
            "AnyAnswer",
            3,
        )
        .await;

        match result {
            Err(AppError::SessionNotFound) => {}
            other => panic!(
                "Expected Err(AppError::SessionNotFound) for unknown session, got {:?}",
                other
            ),
        }
    }

    // -----------------------------------------------------------------------
    // create_session_orchestrated
    // -----------------------------------------------------------------------

    fn make_default_validated_params() -> ValidatedSessionParams {
        ValidatedSessionParams {
            length: 5,
            difficulty: 5,
            width: 220,
            height: 120,
            dark_mode: false,
            compression: 40,
            expires_in: 3600,
        }
    }

    #[tokio::test]
    async fn test_create_session_returns_session_with_expected_fields() {
        let storage = setup_test_storage().await;
        let captcha = CaptchaService::new();
        let metrics = make_metrics();
        let params = make_default_validated_params();

        let before = Utc::now().timestamp();
        let result = create_session_orchestrated(&storage, &captcha, &metrics, params).await;
        let after = Utc::now().timestamp();

        assert!(result.is_ok(), "create_session_orchestrated should succeed");
        let (session, image_bytes) = result.unwrap();

        // ID must be a non-empty UUID-like string
        assert!(!session.id.is_empty(), "Session ID should not be empty");

        // Solution must not be empty and must match the requested length
        assert_eq!(
            session.solution.chars().count(),
            5,
            "Solution length should match the requested length of 5"
        );

        // Timestamps must be within the test window
        assert!(
            session.created_at >= before && session.created_at <= after,
            "created_at should be within the test window"
        );
        assert!(
            session.expires_at >= before + 3600 && session.expires_at <= after + 3600,
            "expires_at should be approximately now + 3600"
        );

        // Persisted fields must match params
        assert_eq!(session.difficulty, 5);
        assert_eq!(session.width, 220);
        assert_eq!(session.height, 120);
        assert!(!session.dark_mode);
        assert_eq!(session.attempt_count, 0);

        // Image bytes must be a valid JPEG
        assert!(!image_bytes.is_empty(), "Image bytes should not be empty");
        let jpeg_signature: [u8; 3] = [0xFF, 0xD8, 0xFF];
        assert!(
            image_bytes.starts_with(&jpeg_signature),
            "Image bytes should start with JPEG signature"
        );
    }

    #[tokio::test]
    async fn test_create_session_persists_to_storage() {
        let storage = setup_test_storage().await;
        let captcha = CaptchaService::new();
        let metrics = make_metrics();
        let params = make_default_validated_params();

        let (session, _) = create_session_orchestrated(&storage, &captcha, &metrics, params)
            .await
            .unwrap();

        // Must be retrievable from storage immediately after creation
        let fetched = storage.get_active_session(&session.id).await.unwrap();

        assert!(
            fetched.is_some(),
            "Session should be retrievable from storage after creation"
        );
        assert_eq!(fetched.unwrap().id, session.id);
    }

    #[tokio::test]
    async fn test_create_session_with_custom_params() {
        let storage = setup_test_storage().await;
        let captcha = CaptchaService::new();
        let metrics = make_metrics();
        let params = ValidatedSessionParams {
            length: 8,
            difficulty: 3,
            width: 300,
            height: 150,
            dark_mode: true,
            compression: 60,
            expires_in: 600,
        };

        let before = Utc::now().timestamp();
        let (session, _) = create_session_orchestrated(&storage, &captcha, &metrics, params)
            .await
            .unwrap();
        let after = Utc::now().timestamp();

        assert_eq!(session.solution.chars().count(), 8);
        assert_eq!(session.difficulty, 3);
        assert_eq!(session.width, 300);
        assert_eq!(session.height, 150);
        assert!(session.dark_mode);
        assert!(
            session.expires_at >= before + 600 && session.expires_at <= after + 600,
            "expires_at should reflect the custom TTL of 600 seconds"
        );
    }
}
