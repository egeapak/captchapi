use crate::config::ConfigHandle;
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

/// Build the tick interval for a given period.
///
/// Uses `interval_at` with the first tick a full period away: `time::interval` fires
/// immediately, which would turn every unrelated configuration change into a spurious cleanup.
fn interval_for(seconds: u64) -> time::Interval {
    // A zero period would panic inside tokio; treat it as "as fast as one second".
    let period = Duration::from_secs(seconds.max(1));
    time::interval_at(time::Instant::now() + period, period)
}

/// Start a background task that periodically cleans up expired sessions and rate limiter entries.
///
/// The task watches `config` so a reload changes the cleanup interval without a restart.
/// Returns a JoinHandle that can be awaited for graceful shutdown.
pub fn start_cleanup_task(
    storage: StorageService,
    config: ConfigHandle,
    metrics: Arc<Metrics>,
    rate_limiter: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    shutdown_token: CancellationToken,
) -> JoinHandle<()> {
    // Subscribe, then drop the handle: holding it would keep a sender alive inside this task
    // forever, so the closed-channel path below could never be reached — or tested.
    let mut updates = config.subscribe();
    let mut period = config.get().cleanup_interval_seconds;
    drop(config);

    tokio::spawn(async move {
        let mut interval = interval_for(period);
        // Once every sender is gone `changed()` resolves with an error immediately and forever.
        // The branch has to be disabled by a guard: awaiting inside the arm body instead would
        // suspend the whole `select!`, including the shutdown branch, and hang the process.
        let mut config_closed = false;

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
                changed = updates.changed(), if !config_closed => {
                    if changed.is_err() {
                        tracing::debug!(
                            "Configuration channel closed; cleanup task keeps its current interval"
                        );
                        config_closed = true;
                        continue;
                    }
                    let updated = updates.borrow_and_update().cleanup_interval_seconds;
                    if updated != period {
                        tracing::info!(
                            "Cleanup interval changed from {}s to {}s",
                            period,
                            updated
                        );
                        period = updated;
                        interval = interval_for(period);
                    }
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

    /// A config handle with the given cleanup interval.
    fn test_config(interval_seconds: u64) -> ConfigHandle {
        ConfigHandle::from_static(crate::config::Config {
            cleanup_interval_seconds: interval_seconds,
            ..crate::config::Config::for_test()
        })
    }

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
        let mut session = Session::new(
            Uuid::new_v4().to_string(),
            "hashed-EXPIRED".to_string(),
            vec![1, 2, 3],
            0,
            5,
            220,
            120,
            false,
        );
        // Manually set expires_at to be in the past
        let now = Utc::now().timestamp();
        session.expires_at = now - 10;
        session
    }

    fn create_valid_session() -> Session {
        Session::new(
            Uuid::new_v4().to_string(),
            "hashed-VALID".to_string(),
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
            // The smallest positive interval (1s); the test cancels before the second tick.
            test_config(1),
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

        // Start the task with a 1-second interval. The first tick lands one full period out —
        // the task deliberately does not fire immediately, so that a configuration change
        // cannot trigger a spurious cleanup.
        let handle = start_cleanup_task(
            storage.clone(),
            test_config(1),
            metrics,
            rate_limiter,
            shutdown_token.clone(),
        );

        // Wait past the first tick, with margin for a loaded CI runner.
        tokio::time::sleep(Duration::from_millis(1_400)).await;

        // Assert before cancelling: the shutdown path also runs a cleanup, so checking
        // afterwards would pass even if the periodic tick never fired.
        assert!(
            storage.get_session(&expired.id).await.unwrap().is_none(),
            "the periodic tick should have cleaned up the expired session"
        );

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

    #[tokio::test]
    async fn test_interval_does_not_fire_immediately() {
        // `time::interval` ticks straight away; this task must not, or every unrelated
        // configuration change would trigger a spurious cleanup.
        let mut interval = interval_for(60);
        assert!(
            tokio::time::timeout(Duration::from_millis(150), interval.tick())
                .await
                .is_err(),
            "the first tick should be a full period away"
        );
    }

    #[tokio::test]
    async fn test_interval_for_treats_zero_as_one_second() {
        // A zero period panics inside tokio, so it must be clamped rather than trusted.
        let mut interval = interval_for(0);
        assert!(
            tokio::time::timeout(Duration::from_millis(1_400), interval.tick())
                .await
                .is_ok(),
            "a zero interval should behave as one second, not panic or hang"
        );
    }

    #[tokio::test]
    async fn test_task_adopts_a_new_interval_without_restarting() {
        let storage = setup_test_storage().await;
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let rate_limiter = make_test_rate_limiter();
        let shutdown_token = CancellationToken::new();

        // Start with an interval far too long to fire during this test...
        let config = test_config(3_600);
        let handle = start_cleanup_task(
            storage.clone(),
            config.clone(),
            metrics,
            rate_limiter,
            shutdown_token.clone(),
        );

        let expired = create_expired_session();
        storage.create_session(&expired).await.unwrap();

        // ...then shorten it. The task must pick this up without a restart.
        config
            .patch(&[("CLEANUP_INTERVAL_SECONDS".into(), "1".into())])
            .unwrap();

        tokio::time::sleep(Duration::from_millis(1_400)).await;

        assert!(
            storage.get_session(&expired.id).await.unwrap().is_none(),
            "the task should have adopted the shorter interval and cleaned up"
        );

        shutdown_token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }

    #[tokio::test]
    async fn test_task_still_shuts_down_after_the_config_channel_closes() {
        // `changed()` errors immediately and forever once every sender is gone, so the arm has
        // to be disabled by a guard. Awaiting inside the arm body instead would suspend the
        // whole `select!` — including the shutdown branch — and this test would time out.
        //
        // `start_cleanup_task` drops its own `ConfigHandle` after subscribing, so dropping the
        // one below really does close the channel. That is what makes this test exercise the
        // path rather than pass vacuously.
        let storage = setup_test_storage().await;
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let rate_limiter = make_test_rate_limiter();
        let shutdown_token = CancellationToken::new();

        let config = test_config(3_600);
        let handle = start_cleanup_task(
            storage,
            config.clone(),
            metrics,
            rate_limiter,
            shutdown_token.clone(),
        );

        drop(config);
        tokio::time::sleep(Duration::from_millis(200)).await;

        // The task must still be responsive to shutdown rather than stuck in a hot loop.
        shutdown_token.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "task did not shut down after losing its config sender"
        );
        assert!(result.unwrap().is_ok(), "task panicked");
    }
}
