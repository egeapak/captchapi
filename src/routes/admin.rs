use crate::config::params::{by_field, Reload, PARAMS};
use crate::config::sources::redact;
use crate::config::ConfigHandle;
use crate::error::{AppError, Result};
use crate::metrics::Metrics;
use crate::middleware::MasterKeyMiddleware;
use crate::services::StorageService;
use crate::tasks::cleanup_expired_sessions;
use axum::{
    extract::State,
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AdminState {
    pub storage: StorageService,
    pub metrics: Arc<Metrics>,
    pub config: ConfigHandle,
}

pub fn admin_routes(state: AdminState, master_middleware: MasterKeyMiddleware) -> Router {
    Router::new()
        .route("/cleanup", post(trigger_cleanup))
        .route("/config", get(get_config).patch(patch_config))
        .route("/config/reload", post(reload_config))
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

    let sessions_deleted = cleanup_expired_sessions(&state.storage, &state.metrics).await?;

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

/// One parameter as reported by `GET /config`.
#[derive(Debug, Serialize)]
pub struct ConfigEntry {
    /// Effective value, redacted when the parameter is a secret.
    pub value: String,
    /// Whether a reload or a PATCH can change this field.
    pub reloadable: bool,
    /// Whether the value is hidden because it is a secret.
    pub secret: bool,
}

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub config: BTreeMap<&'static str, ConfigEntry>,
    /// Fields currently overridden through this API. These are cleared by a reload.
    pub overrides: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ReloadResponse {
    #[serde(flatten)]
    pub config: ConfigResponse,
    /// Boot-only fields whose configured value differs from the running one. Reported, never
    /// applied — changing them needs a restart.
    pub ignored: Vec<&'static str>,
    pub message: String,
}

/// Render the running configuration, redacting secrets.
///
/// Secrets are redacted even for the master key holder: this endpoint exists to explain the
/// server's behaviour, not to read credentials back out of it.
fn describe(config: &ConfigHandle) -> ConfigResponse {
    let snapshot = config.get();
    let entries = PARAMS
        .iter()
        .filter_map(|param| {
            let value = snapshot.field_value(param.field)?;
            Some((
                param.field,
                ConfigEntry {
                    value: redact(param, &value),
                    reloadable: param.reload == Reload::Live,
                    secret: param.secret,
                },
            ))
        })
        .collect();

    ConfigResponse {
        config: entries,
        overrides: config.overlay_keys(),
    }
}

/// Return the effective configuration.
async fn get_config(State(state): State<AdminState>) -> Result<Json<ConfigResponse>> {
    Ok(Json(describe(&state.config)))
}

/// Update reloadable configuration fields at runtime.
///
/// Values live in memory only: they are never written back to a config file, and the next
/// reload clears them. Boot-only fields are rejected rather than silently ignored.
async fn patch_config(
    State(state): State<AdminState>,
    Json(req): Json<BTreeMap<String, serde_json::Value>>,
) -> Result<Json<ConfigResponse>> {
    if req.is_empty() {
        return Err(AppError::InvalidConfig(
            "no fields given; supply at least one reloadable field".to_string(),
        ));
    }

    let mut updates = Vec::with_capacity(req.len());
    for (field, value) in &req {
        let param = by_field(field).ok_or_else(|| {
            AppError::InvalidConfig(format!("`{field}` is not a configuration field"))
        })?;

        if param.reload != Reload::Live {
            return Err(AppError::ConfigNotReloadable(format!(
                "`{field}` is applied at startup and cannot be changed at runtime; restart with a new value"
            )));
        }

        // Accept both `600` and `"600"`; everything funnels into the same string form the
        // environment would have provided, so validation stays in exactly one place.
        let raw = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            other => {
                return Err(AppError::InvalidConfig(format!(
                    "`{field}` must be a string, number or boolean, not `{other}`"
                )))
            }
        };
        updates.push((param.env.to_string(), raw));
    }

    let before = state.config.get();

    // Resolution reads files, so it runs off the async worker threads.
    let handle = state.config.clone();
    let applied = tokio::task::spawn_blocking(move || handle.patch(&updates))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("config patch task failed: {e}")))?
        .map_err(AppError::InvalidConfig)?;

    // Audit every accepted change: this endpoint can alter security-relevant limits.
    for field in req.keys() {
        let (old, new) = (before.field_value(field), applied.field_value(field));
        if old != new {
            tracing::info!(
                "Admin config change: {field} {} -> {}",
                old.unwrap_or_default(),
                new.unwrap_or_default()
            );
        }
    }

    state.metrics.system.config_reloads.add(1, &[]);
    Ok(Json(describe(&state.config)))
}

/// Re-read every configuration source, discarding any runtime overrides.
async fn reload_config(State(state): State<AdminState>) -> Result<Json<ReloadResponse>> {
    tracing::info!("Configuration reload requested through the admin API");

    let handle = state.config.clone();
    let outcome = tokio::task::spawn_blocking(move || handle.reload())
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("config reload task failed: {e}")))?
        .map_err(|e| {
            state.metrics.system.config_reload_failures.add(1, &[]);
            AppError::InvalidConfig(e)
        })?;

    for field in &outcome.drift {
        tracing::warn!("`{field}` changed but is applied only at startup; restart to change it");
    }

    state.metrics.system.config_reloads.add(1, &[]);

    let message = if outcome.drift.is_empty() {
        "Configuration reloaded".to_string()
    } else {
        format!(
            "Configuration reloaded; {} field(s) require a restart and were not applied",
            outcome.drift.len()
        )
    };

    Ok(Json(ReloadResponse {
        config: describe(&state.config),
        ignored: outcome.drift,
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
        let metrics = crate::metrics::Metrics::new();

        // Create expired session
        let expired = create_expired_session();
        storage.create_session(&expired).await.unwrap();

        // Create valid session
        let valid = create_valid_session();
        storage.create_session(&valid).await.unwrap();

        // Trigger cleanup
        let result = cleanup_expired_sessions(&storage, &metrics).await.unwrap();

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
        let metrics = crate::metrics::Metrics::new();

        // Create only valid sessions
        let valid1 = create_valid_session();
        let valid2 = create_valid_session();

        storage.create_session(&valid1).await.unwrap();
        storage.create_session(&valid2).await.unwrap();

        // Trigger cleanup
        let result = cleanup_expired_sessions(&storage, &metrics).await.unwrap();

        // Should have cleaned up 0 sessions
        assert_eq!(result, 0);

        // Both should still exist
        assert!(storage.get_session(&valid1.id).await.unwrap().is_some());
        assert!(storage.get_session(&valid2.id).await.unwrap().is_some());
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

        // Trigger cleanup
        let result = cleanup_expired_sessions(&storage, &metrics).await.unwrap();

        // Should have cleaned up all 3
        assert_eq!(result, 3);

        // All should be gone
        assert!(storage.get_session(&expired1.id).await.unwrap().is_none());
        assert!(storage.get_session(&expired2.id).await.unwrap().is_none());
        assert!(storage.get_session(&expired3.id).await.unwrap().is_none());
    }
}
