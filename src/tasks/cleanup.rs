use crate::services::StorageService;
use std::time::Duration;
use tokio::time;

pub fn start_cleanup_task(storage: StorageService, interval_seconds: u64) {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(interval_seconds));

        loop {
            interval.tick().await;

            match storage.delete_expired_sessions().await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!("Cleaned up {} expired sessions", count);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to cleanup expired sessions: {:?}", e);
                }
            }
        }
    });
}
