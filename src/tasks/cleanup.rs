use crate::error::Result;
use crate::metrics::Metrics;
use crate::services::StorageService;
use governor::DefaultKeyedRateLimiter;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// Perform a single cleanup operation, removing expired sessions
#[tracing::instrument(skip(storage, metrics), fields(cleaned_count))]
pub async fn cleanup_expired_sessions(storage: &StorageService, metrics: &Metrics) -> Result<u64> {
    let count = storage.delete_expired_sessions().await?;

    tracing::Span::current().record("cleaned_count", count);

    if count > 0 {
        // Record metrics
        metrics.sessions.expired_cleaned.add(count, &[]);
        tracing::info!("Cleaned up {} expired sessions", count);
    }

    Ok(count)
}

/// Start a background task that periodically cleans up expired sessions and rate limiter entries.
/// Returns a JoinHandle that can be awaited for graceful shutdown.
pub fn start_cleanup_task(
    storage: StorageService,
    interval_seconds: u64,
    metrics: Arc<Metrics>,
    rate_limiter: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    shutdown_token: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(interval_seconds));

        loop {
            tokio::select! {
                _ = shutdown_token.cancelled() => {
                    tracing::info!("Cleanup task received shutdown signal, performing final cleanup");
                    // Perform one final cleanup before shutting down
                    if let Err(e) = cleanup_expired_sessions(&storage, &metrics).await {
                        tracing::error!("Failed to cleanup expired sessions during shutdown: {:?}", e);
                    }
                    rate_limiter.retain_recent();
                    tracing::info!("Cleanup task shutdown complete");
                    break;
                }
                _ = interval.tick() => {
                    // Cleanup expired sessions
                    if let Err(e) = cleanup_expired_sessions(&storage, &metrics).await {
                        tracing::error!("Failed to cleanup expired sessions: {:?}", e);
                    }

                    // Cleanup old rate limiter entries
                    rate_limiter.retain_recent();
                    tracing::debug!(
                        "Rate limiter cleanup completed. Tracked IPs: {}",
                        rate_limiter.len()
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Session;
    use chrono::Utc;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tokio_util::sync::CancellationToken;
    use tower_governor::governor::GovernorConfigBuilder;
    use uuid::Uuid;

    async fn setup_test_storage() -> StorageService {
        let db_name = format!(
            "file:test_cleanup_{}?mode=memory&cache=shared",
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

    fn create_expired_session() -> Session {
        // Create session that expired 10 seconds ago
        let mut session = Session::new("EXPIRED".to_string(), vec![1, 2, 3], 0, 5, 220, 120, false);
        // Manually set expires_at to be in the past
        let now = Utc::now().timestamp();
        session.expires_at = now - 10;
        session
    }

    fn create_valid_session() -> Session {
        Session::new(
            "VALID".to_string(),
            vec![4, 5, 6],
            3600, // Valid for 1 hour
            5,
            220,
            120,
            false,
        )
    }

    #[tokio::test]
    async fn test_cleanup_removes_expired_sessions() {
        let storage = setup_test_storage().await;
        let metrics = crate::metrics::Metrics::new();

        // Create expired session (expires_at is in the past)
        let expired = create_expired_session();
        storage.create_session(&expired).await.unwrap();

        // Create valid session
        let valid = create_valid_session();
        storage.create_session(&valid).await.unwrap();

        // Run cleanup
        let count = cleanup_expired_sessions(&storage, &metrics).await.unwrap();

        // Should have cleaned up 1 session
        assert_eq!(count, 1);

        // Expired should be gone
        assert!(storage.get_session(&expired.id).await.unwrap().is_none());

        // Valid should still exist
        assert!(storage.get_session(&valid.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_cleanup_with_empty_database() {
        let storage = setup_test_storage().await;
        let metrics = crate::metrics::Metrics::new();

        let count = cleanup_expired_sessions(&storage, &metrics).await.unwrap();
        assert_eq!(count, 0, "Cleanup on an empty database should return 0");
    }

    #[tokio::test]
    async fn test_cleanup_mixed_sessions() {
        let storage = setup_test_storage().await;
        let metrics = crate::metrics::Metrics::new();

        // Create mix of expired and valid
        let expired1 = create_expired_session();
        let valid1 = create_valid_session();
        let expired2 = create_expired_session();
        let valid2 = create_valid_session();

        storage.create_session(&expired1).await.unwrap();
        storage.create_session(&valid1).await.unwrap();
        storage.create_session(&expired2).await.unwrap();
        storage.create_session(&valid2).await.unwrap();

        // Run cleanup
        let count = cleanup_expired_sessions(&storage, &metrics).await.unwrap();

        // Should have cleaned up 2 expired sessions
        assert_eq!(count, 2);

        // Expired should be gone
        assert!(storage.get_session(&expired1.id).await.unwrap().is_none());
        assert!(storage.get_session(&expired2.id).await.unwrap().is_none());

        // Valid should still exist
        assert!(storage.get_session(&valid1.id).await.unwrap().is_some());
        assert!(storage.get_session(&valid2.id).await.unwrap().is_some());
    }

    /// Build a permissive `DefaultKeyedRateLimiter<IpAddr>` suitable for tests.
    ///
    /// Uses the same `GovernorConfigBuilder` path as `main.rs` so the returned
    /// `Arc<DefaultKeyedRateLimiter<IpAddr>>` is exactly the type that
    /// `start_cleanup_task` expects.
    fn make_test_rate_limiter() -> Arc<DefaultKeyedRateLimiter<std::net::IpAddr>> {
        let governor_conf = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(100)
                .burst_size(100)
                .finish()
                .expect("Failed to build test governor config"),
        );
        governor_conf.limiter().clone()
    }

    // -----------------------------------------------------------------------
    // start_cleanup_task — lifecycle tests
    // -----------------------------------------------------------------------

    /// Verify that the cleanup task starts, runs at least one tick, and
    /// shuts down cleanly when the `CancellationToken` is cancelled.
    ///
    /// A 2-second `tokio::time::timeout` guards against the test hanging
    /// indefinitely should the shutdown branch be broken.
    #[tokio::test]
    async fn test_start_cleanup_task_runs_and_shuts_down() {
        let storage = setup_test_storage().await;
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let rate_limiter = make_test_rate_limiter();
        let shutdown_token = CancellationToken::new();

        // Use a 100 ms interval so the task ticks quickly in CI.
        let handle = start_cleanup_task(
            storage,
            // interval_seconds is u64 — pass the smallest positive value (1s)
            // and cancel before the second tick to keep the test fast.
            1,
            metrics,
            rate_limiter,
            shutdown_token.clone(),
        );

        // Give the task time to reach its first interval tick.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Signal the task to shut down.
        shutdown_token.cancel();

        // The task must complete within 2 seconds after cancellation.
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(
            result.is_ok(),
            "cleanup task did not complete within the timeout after cancellation"
        );

        let join_result = result.unwrap();
        assert!(
            join_result.is_ok(),
            "cleanup task panicked: {:?}",
            join_result.unwrap_err()
        );
    }

    /// Verify that after the cleanup task fires at least one tick, expired
    /// sessions are removed from the database and valid sessions are preserved.
    #[tokio::test]
    async fn test_start_cleanup_task_cleans_expired_sessions() {
        let storage = setup_test_storage().await;
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let rate_limiter = make_test_rate_limiter();
        let shutdown_token = CancellationToken::new();

        // Insert one expired and one valid session before starting the task.
        let expired = create_expired_session();
        let valid = create_valid_session();
        storage.create_session(&expired).await.unwrap();
        storage.create_session(&valid).await.unwrap();

        // Confirm both rows are present before the task runs.
        assert!(
            storage.get_session(&expired.id).await.unwrap().is_some(),
            "expired session should be in the database before task starts"
        );
        assert!(
            storage.get_session(&valid.id).await.unwrap().is_some(),
            "valid session should be in the database before task starts"
        );

        // Start the task with a 1-second interval.  The interval fires
        // immediately on the first tick, so we only need a short wait.
        let handle = start_cleanup_task(
            storage.clone(),
            1,
            metrics,
            rate_limiter,
            shutdown_token.clone(),
        );

        // Wait long enough for the first interval tick to execute
        // (tokio's interval fires immediately the first time, so 200 ms is
        // generous while staying well below any CI timeout).
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Cancel the task now that at least one tick has occurred.
        shutdown_token.cancel();

        // Wait for the task to finish (2-second safety net).
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "cleanup task did not complete within the timeout"
        );
        assert!(
            result.unwrap().is_ok(),
            "cleanup task panicked during shutdown"
        );

        // The expired session must have been deleted by the background task.
        assert!(
            storage.get_session(&expired.id).await.unwrap().is_none(),
            "expired session should have been cleaned up by the task"
        );

        // The valid session must still be present.
        assert!(
            storage.get_session(&valid.id).await.unwrap().is_some(),
            "valid session should not have been removed by the task"
        );
    }
}
