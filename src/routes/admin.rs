use crate::error::Result;
use crate::middleware::MasterKeyMiddleware;
use crate::services::StorageService;
use crate::tasks::cleanup_expired_sessions;
use axum::{extract::State, middleware, routing::post, Json, Router};
use serde::Serialize;

#[derive(Clone)]
pub struct AdminState {
    pub storage: StorageService,
}

pub fn admin_routes(state: AdminState, master_middleware: MasterKeyMiddleware) -> Router {
    Router::new()
        .route("/cleanup", post(trigger_cleanup))
        .route_layer(middleware::from_fn_with_state(
            master_middleware,
            MasterKeyMiddleware::authenticate,
        ))
        .with_state(state)
}

#[derive(Debug, Serialize)]
pub struct CleanupResponse {
    pub sessions_deleted: u64,
    pub message: String,
}

/// Trigger manual cleanup of expired sessions
///
/// This endpoint allows administrators to manually trigger cleanup of expired
/// sessions and their associated JPEG blobs from the database.
///
/// Protected by master key authentication.
async fn trigger_cleanup(State(state): State<AdminState>) -> Result<Json<CleanupResponse>> {
    tracing::info!("Manual cleanup triggered");

    let sessions_deleted = cleanup_expired_sessions(&state.storage).await?;

    let message = if sessions_deleted > 0 {
        format!(
            "Successfully cleaned up {} expired session(s)",
            sessions_deleted
        )
    } else {
        "No expired sessions found to clean up".to_string()
    };

    tracing::info!(
        "Manual cleanup completed: {} session(s) deleted",
        sessions_deleted
    );

    Ok(Json(CleanupResponse {
        sessions_deleted,
        message,
    }))
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
            "file:test_admin_{}?mode=memory&cache=shared",
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
        let mut session = Session::new("EXPIRED".to_string(), vec![1, 2, 3], 0, 5, 220, 120, false);
        let now = Utc::now().timestamp();
        session.expires_at = now - 10;
        session
    }

    fn create_valid_session() -> Session {
        Session::new("VALID".to_string(), vec![4, 5, 6], 3600, 5, 220, 120, false)
    }

    #[tokio::test]
    async fn test_cleanup_deletes_expired_sessions() {
        let storage = setup_test_storage().await;

        // Create expired session
        let expired = create_expired_session();
        storage.create_session(&expired).await.unwrap();

        // Create valid session
        let valid = create_valid_session();
        storage.create_session(&valid).await.unwrap();

        // Trigger cleanup
        let result = cleanup_expired_sessions(&storage).await.unwrap();

        // Should have cleaned up 1 session
        assert_eq!(result, 1);

        // Expired should be gone
        assert!(storage.get_session(&expired.id).await.unwrap().is_none());

        // Valid should still exist
        assert!(storage.get_session(&valid.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_cleanup_with_no_expired_sessions() {
        let storage = setup_test_storage().await;

        // Create only valid sessions
        let valid1 = create_valid_session();
        let valid2 = create_valid_session();

        storage.create_session(&valid1).await.unwrap();
        storage.create_session(&valid2).await.unwrap();

        // Trigger cleanup
        let result = cleanup_expired_sessions(&storage).await.unwrap();

        // Should have cleaned up 0 sessions
        assert_eq!(result, 0);

        // Both should still exist
        assert!(storage.get_session(&valid1.id).await.unwrap().is_some());
        assert!(storage.get_session(&valid2.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_cleanup_multiple_expired_sessions() {
        let storage = setup_test_storage().await;

        // Create multiple expired sessions
        let expired1 = create_expired_session();
        let expired2 = create_expired_session();
        let expired3 = create_expired_session();

        storage.create_session(&expired1).await.unwrap();
        storage.create_session(&expired2).await.unwrap();
        storage.create_session(&expired3).await.unwrap();

        // Trigger cleanup
        let result = cleanup_expired_sessions(&storage).await.unwrap();

        // Should have cleaned up all 3
        assert_eq!(result, 3);

        // All should be gone
        assert!(storage.get_session(&expired1.id).await.unwrap().is_none());
        assert!(storage.get_session(&expired2.id).await.unwrap().is_none());
        assert!(storage.get_session(&expired3.id).await.unwrap().is_none());
    }
}
