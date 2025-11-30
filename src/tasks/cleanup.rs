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
    async fn test_cleanup_preserves_valid_sessions() {
        let storage = setup_test_storage().await;
        let metrics = crate::metrics::Metrics::new();

        // Create only valid sessions
        let valid1 = create_valid_session();
        let valid2 = create_valid_session();

        storage.create_session(&valid1).await.unwrap();
        storage.create_session(&valid2).await.unwrap();

        // Run cleanup
        let count = cleanup_expired_sessions(&storage, &metrics).await.unwrap();

        // Should have cleaned up 0 sessions
        assert_eq!(count, 0);

        // Both should still exist
        assert!(storage.get_session(&valid1.id).await.unwrap().is_some());
        assert!(storage.get_session(&valid2.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_cleanup_with_empty_database() {
        let storage = setup_test_storage().await;
        let metrics = crate::metrics::Metrics::new();

        // Run cleanup on empty database
        let result = cleanup_expired_sessions(&storage, &metrics).await;

        // Should succeed with 0 count
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_cleanup_multiple_expired_sessions() {
        let storage = setup_test_storage().await;
        let metrics = crate::metrics::Metrics::new();

        // Create multiple expired sessions
        let expired1 = create_expired_session();
        let expired2 = create_expired_session();
        let expired3 = create_expired_session();

        storage.create_session(&expired1).await.unwrap();
        storage.create_session(&expired2).await.unwrap();
        storage.create_session(&expired3).await.unwrap();

        // Run cleanup
        let count = cleanup_expired_sessions(&storage, &metrics).await.unwrap();

        // Should have cleaned up all 3
        assert_eq!(count, 3);

        // All should be gone
        assert!(storage.get_session(&expired1.id).await.unwrap().is_none());
        assert!(storage.get_session(&expired2.id).await.unwrap().is_none());
        assert!(storage.get_session(&expired3.id).await.unwrap().is_none());
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
}
