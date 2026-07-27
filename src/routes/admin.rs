use crate::config::params::{by_env, by_field, Reload, PARAMS};
use crate::config::sources::redact;
use crate::config::{Config, ConfigHandle, Source};
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
    /// The layer the effective value came from: `admin`, `cli`, `env`, `env-file`, `file`,
    /// `carried` or `default`.
    pub source: &'static str,
    /// Whether PATCH will accept this field. False for boot-only fields, and for reloadable
    /// fields this process was given explicitly on the command line or in the environment —
    /// an override there would work until the next reload discarded it, and could never be
    /// made durable without a restart.
    pub editable: bool,
    /// One sentence on what this parameter does, so a client does not have to ship its own
    /// copy of the documentation and let it drift from the server's.
    pub description: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub config: BTreeMap<&'static str, ConfigEntry>,
    /// Fields currently overridden through this API, named exactly as `config`'s keys and as
    /// PATCH expects them, so they can be fed straight back in. Cleared by a reload.
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
    describe_config(&config.get(), config)
}

/// Render a specific configuration snapshot, with the override list from `config`.
///
/// Callers that just produced a snapshot pass it here rather than re-reading the handle, so a
/// concurrent reload cannot make the response describe a different configuration than the one
/// the request produced.
fn describe_config(snapshot: &Config, config: &ConfigHandle) -> ConfigResponse {
    let sources = config.sources();
    let entries = PARAMS
        .iter()
        .filter_map(|param| {
            let value = snapshot.field_value(param.field)?;
            let source = sources.get(param.field);
            Some((
                param.field,
                ConfigEntry {
                    value: redact(param, &value),
                    reloadable: param.reload == Reload::Live,
                    secret: param.secret,
                    source: source.label(),
                    editable: param.reload == Reload::Live && !source.is_pinned(),
                    description: param.about,
                },
            ))
        })
        .collect();

    // The overlay is keyed by canonical environment key, but every other part of this API —
    // the `config` map above and the PATCH request body — speaks `Config` field names. Translate
    // here so a client can feed `overrides` straight back into PATCH.
    let mut overrides: Vec<String> = config
        .overlay_keys()
        .iter()
        .filter_map(|key| by_env(key).map(|param| param.field.to_string()))
        .collect();
    overrides.sort_unstable();

    ConfigResponse {
        config: entries,
        overrides,
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
    // Deployments that want file-driven reload but no remote writes set ADMIN_CONFIG_WRITE=false.
    // Reload and GET stay available; only mutation is refused.
    if !state.config.get().admin_config_write {
        state.metrics.system.config_patch_failures.add(1, &[]);
        return Err(AppError::Forbidden(
            "runtime configuration writes are disabled (ADMIN_CONFIG_WRITE=false)".to_string(),
        ));
    }

    if req.is_empty() {
        state.metrics.system.config_patch_failures.add(1, &[]);
        return Err(AppError::InvalidConfig(
            "no fields given; supply at least one reloadable field".to_string(),
        ));
    }

    // One snapshot for the whole batch: `sources()` takes the handle's lock and clones the map,
    // so reading it per field would do both once per key for no benefit. It is only a
    // fast-fail with a precise error code either way — `ConfigHandle::patch` re-checks under
    // its own lock, which is what actually makes the rule hold against a concurrent reload.
    let sources = state.config.sources();

    let mut updates = Vec::with_capacity(req.len());
    for (field, value) in &req {
        let Some(param) = by_field(field) else {
            state.metrics.system.config_patch_failures.add(1, &[]);
            return Err(AppError::InvalidConfig(format!(
                "`{field}` is not a configuration field"
            )));
        };

        if param.reload != Reload::Live {
            state.metrics.system.config_patch_failures.add(1, &[]);
            return Err(AppError::ConfigNotReloadable(format!(
                "`{field}` is applied at startup and cannot be changed at runtime; restart with a new value"
            )));
        }

        // A reloadable field can still be off limits: if this process was started with an
        // explicit value on the command line or in the environment, an override would hold
        // only until the next reload and could never be made durable. Rejected here, ahead of
        // `ConfigHandle::patch`, so the response carries its own error code rather than being
        // flattened into `invalid_config` with everything else the handle refuses.
        let source = sources.get(param.field);
        if source.is_pinned() {
            state.metrics.system.config_patch_failures.add(1, &[]);
            return Err(AppError::ConfigPinned(format!(
                "`{field}` was set on the {} this server was started with; \
                 change it there and restart, or remove it to manage `{field}` from here",
                if source == Source::Cli {
                    "command line"
                } else {
                    "environment"
                }
            )));
        }

        // Accept both `600` and `"600"`; everything funnels into the same string form the
        // environment would have provided, so validation stays in exactly one place.
        let raw = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            other => {
                state.metrics.system.config_patch_failures.add(1, &[]);
                return Err(AppError::InvalidConfig(format!(
                    "`{field}` must be a string, number or boolean, not `{other}`"
                )));
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
        .map_err(|e| {
            state.metrics.system.config_patch_failures.add(1, &[]);
            AppError::InvalidConfig(e)
        })?;

    // Audit every accepted change: this endpoint can alter security-relevant limits.
    // Values go through `redact` — no secret is reloadable today (a PARAMS invariant test
    // enforces that), but the audit trail must not become the one place that leaks if it changes.
    for field in req.keys() {
        let Some(param) = by_field(field) else {
            continue;
        };
        let (old, new) = (before.field_value(field), applied.field_value(field));
        if old != new {
            tracing::info!(
                "Admin config change: {field} {} -> {}",
                redact(param, &old.unwrap_or_default()),
                redact(param, &new.unwrap_or_default())
            );
        }
    }

    state.metrics.system.config_patches.add(1, &[]);
    // Report the config this patch actually produced rather than re-reading the handle, which a
    // concurrent reload could have moved out from under us.
    Ok(Json(describe_config(&applied, &state.config)))
}

/// Re-read every configuration source, discarding any runtime overrides.
async fn reload_config(State(state): State<AdminState>) -> Result<Json<ReloadResponse>> {
    tracing::info!("Configuration reload requested through the admin API");

    let handle = state.config.clone();
    let stored = state.config.stored();
    let outcome = tokio::task::spawn_blocking(move || handle.reload(stored))
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
        config: describe_config(&outcome.config, &state.config),
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
        let now = Utc::now().timestamp();
        session.expires_at = now - 10;
        session
    }

    fn create_valid_session() -> Session {
        Session::new(
            Uuid::new_v4().to_string(),
            "hashed-VALID".to_string(),
            vec![4, 5, 6],
            3600,
            5,
            220,
            120,
            false,
        )
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
