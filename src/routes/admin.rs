use crate::config::params::{by_env, by_field, Persist, Reload, PARAMS};
use crate::config::sources::redact;
use crate::config::sources::{Layer, Sources};
use crate::config::{Config, ConfigHandle, Source};
use crate::error::{AppError, Result};
use crate::metrics::Metrics;
use crate::middleware::MasterKeyMiddleware;
use crate::restart::{preflight_bind, RestartHandle};
use crate::services::{ConfigStore, StorageService, WrittenBy};
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
    pub store: ConfigStore,
    /// `None` in tests and anywhere the server is not the one that owns the process, in which
    /// case a restart request is refused rather than silently doing nothing.
    pub restart: Option<RestartHandle>,
}

pub fn admin_routes(state: AdminState, master_middleware: MasterKeyMiddleware) -> Router {
    Router::new()
        .route("/cleanup", post(trigger_cleanup))
        .route("/config", get(get_config).patch(patch_config))
        .route("/config/reload", post(reload_config))
        .route("/config/stored", get(get_stored).put(put_stored))
        .route(
            "/config/stored/{field}",
            axum::routing::delete(delete_stored),
        )
        .route("/restart", post(restart_server))
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
    /// Whether `PUT /config/stored` will accept this field.
    ///
    /// A different question from `editable`, and the console needs both. `editable` asks
    /// whether the running process can take a new value now; this asks whether one can be made
    /// to survive a restart. A boot field is storable but never editable; a live field can be
    /// both. Pinning does *not* disqualify — see `shadowed_by`.
    pub storable: bool,
    /// The layer outranking a stored value for this field, when one exists.
    ///
    /// A stored value that a higher layer shadows is not wasted: it persists, and takes effect
    /// as soon as that layer stops answering — the command line or environment being dropped,
    /// or a runtime override being cleared by a reload. What must never happen is storing it
    /// and implying it took effect, so it is reported rather than refused.
    ///
    /// Every layer above `stored` counts, not just the pinned ones. `admin` outranks the
    /// command line, so a `PATCH` masks a stored value exactly as an environment variable
    /// does; testing for pinned-ness alone reported that case as being in effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by: Option<&'static str>,
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
    /// Boot-only fields whose configured value differs from the running one, so a restart
    /// would change them. Covers every layer, not just the store: an edited env file shows up
    /// here too.
    pub pending_restart: Vec<&'static str>,
}

/// The persisted settings, as `GET`/`PUT /config/stored` speak them.
#[derive(Debug, Serialize)]
pub struct StoredResponse {
    /// Field name to raw stored value, filtered to the rows the configuration stack honours.
    ///
    /// Secrets can never appear. That is true twice over: they are `Persist::Never` so this API
    /// refuses to store them, and the same rule filters this map, so a row planted by hand is
    /// dropped here exactly as it is dropped from the running configuration.
    pub stored: BTreeMap<String, String>,
    /// Stored fields a higher layer currently overrides, so they are not in effect.
    pub shadowed: Vec<String>,
    /// Stored boot fields that a restart would apply.
    pub pending_restart: Vec<&'static str>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct RestartResponse {
    /// Boot fields the restart is expected to apply.
    pub applying: Vec<&'static str>,
    pub message: String,
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
    let stored = config.stored();
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
                    storable: param.persist == Persist::Allowed,
                    // Only meaningful when something *is* stored for the field: a field an
                    // environment variable answers for, with nothing stored, is not being
                    // shadowed — it simply has no stored value to shadow.
                    shadowed_by: (stored.contains_key(param.env) && source != Source::Stored)
                        .then(|| source.label()),
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
        pending_restart: config.pending_restart(),
    }
}

/// Turn a JSON body into the `(field, raw string)` pairs every layer speaks.
///
/// Shared by `PATCH` and `PUT` so both accept `600` and `"600"` identically, and so neither
/// grows its own idea of what a configuration value looks like.
fn coerce(req: &BTreeMap<String, serde_json::Value>) -> Result<Vec<(String, String)>> {
    req.iter()
        .map(|(field, value)| {
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
            Ok((field.clone(), raw))
        })
        .collect()
}

/// Which stored fields a higher layer currently overrides.
///
/// The test is "something other than the store answered for this field", not "a pinned layer
/// did". `Source::Admin` sits above `Source::Cli`, so a runtime override masks a stored value
/// just as an environment variable does — and reporting that one as in effect made both this
/// list and the console claim a value the server was demonstrably not using.
fn shadowed(config: &ConfigHandle, stored: &BTreeMap<String, String>) -> Vec<String> {
    let sources = config.sources();
    let mut names: Vec<String> = stored
        .keys()
        .filter(|field| sources.get(field) != Source::Stored)
        .cloned()
        .collect();
    names.sort_unstable();
    names
}

/// Render the store, with everything a client needs to know about whether it is in effect.
async fn describe_stored(state: &AdminState, message: String) -> Result<Json<StoredResponse>> {
    // `visible`, not `all`: a row the configuration stack drops must not be echoed here, or the
    // response describes a server that does not exist — and since every secret is
    // `Persist::Never`, echoing raw rows is also the one way a secret could reach this API.
    let stored = state.store.visible().await?;
    Ok(Json(StoredResponse {
        shadowed: shadowed(&state.config, &stored),
        pending_restart: state.config.pending_restart(),
        stored,
        message,
    }))
}

/// Return the persisted settings.
async fn get_stored(State(state): State<AdminState>) -> Result<Json<StoredResponse>> {
    describe_stored(&state, "Stored configuration".to_string()).await
}

/// Persist settings so they survive a restart.
///
/// Live fields take effect immediately, exactly as a `PATCH` would. Boot fields cannot, and
/// come back in `pending_restart` rather than being silently accepted as though they had.
async fn put_stored(
    State(state): State<AdminState>,
    Json(req): Json<BTreeMap<String, serde_json::Value>>,
) -> Result<Json<StoredResponse>> {
    if !state.config.get().admin_config_write {
        state.metrics.system.config_patch_failures.add(1, &[]);
        return Err(AppError::Forbidden(
            "runtime configuration writes are disabled (ADMIN_CONFIG_WRITE=false)".to_string(),
        ));
    }
    if req.is_empty() {
        state.metrics.system.config_patch_failures.add(1, &[]);
        return Err(AppError::InvalidConfig(
            "no fields given; supply at least one storable field".to_string(),
        ));
    }

    let updates = coerce(&req).inspect_err(|_| {
        state.metrics.system.config_patch_failures.add(1, &[]);
    })?;

    // Refuse anything unstorable before the candidate is assembled, so the error names the
    // field rather than surfacing as a resolution failure further down.
    for (field, _) in &updates {
        match by_field(field) {
            Some(param) if param.persist == Persist::Allowed => {}
            Some(param) => {
                state.metrics.system.config_patch_failures.add(1, &[]);
                return Err(AppError::ConfigNotPersistable(format!(
                    "`{}` cannot be stored: it is either a secret, needed to open the database \
                     the store lives in, or consumed before the store is read",
                    param.field
                )));
            }
            None => {
                state.metrics.system.config_patch_failures.add(1, &[]);
                return Err(AppError::InvalidConfig(format!(
                    "`{field}` is not a configuration field"
                )));
            }
        }
    }

    // Validate before writing. Writing first would persist a configuration the server had
    // already refused, leaving the rollback machinery to undo it on the next restart — a far
    // worse way to find out a value was a typo.
    let mut candidate: Layer = state.config.stored();
    for (field, value) in &updates {
        if let Some(param) = by_field(field) {
            candidate.insert(param.env.to_string(), value.clone());
        }
    }
    let handle = state.config.clone();
    let probe = candidate.clone();
    tokio::task::spawn_blocking(move || handle.dry_run(&probe))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("config dry run failed: {e}")))?
        .map_err(|e| {
            state.metrics.system.config_patch_failures.add(1, &[]);
            AppError::InvalidConfig(e)
        })?;

    let generation = state.store.set(&updates, WrittenBy::AdminApi).await?;

    // Adopt rather than reload: writing to the store says nothing about whether an unrelated
    // runtime override should be discarded, so the overlay is left alone.
    let handle = state.config.clone();
    let outcome = tokio::task::spawn_blocking(move || handle.adopt_stored(candidate))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("config adopt task failed: {e}")))?
        .map_err(AppError::InvalidConfig)?;

    for (field, value) in &updates {
        tracing::info!("Stored config change: {field} = {value} (generation {generation})");
    }
    state.metrics.system.config_patches.add(1, &[]);

    // Only the fields *this* request touched. `outcome.drift` is every boot field that differs,
    // which can include one an unrelated env-file edit changed — reporting that as something
    // this write caused would be a plain lie.
    let touched: Vec<&'static str> = updates
        .iter()
        .filter_map(|(field, _)| by_field(field))
        .map(|param| param.field)
        .collect();
    let message = put_message(&touched, &outcome.drift, &state.config.sources());
    describe_stored(&state, message).await
}

/// Say what actually happened to the fields a write touched.
///
/// A stored value has three possible fates and only one of them is "done": it is live now, or
/// it is waiting for a restart, or a higher layer answers for the field and it is not in effect
/// at all. Reporting only the first two is how this endpoint came to answer "all are in effect"
/// for a field a runtime override was masking.
fn put_message(touched: &[&'static str], drift: &[&'static str], sources: &Sources) -> String {
    let masked: Vec<&'static str> = touched
        .iter()
        .copied()
        .filter(|field| sources.get(field) != Source::Stored)
        .collect();
    // A masked field is not also "pending a restart": the restart would resolve to the same
    // higher layer and change nothing, so naming it twice would just be noise.
    let waiting: Vec<&'static str> = drift
        .iter()
        .copied()
        .filter(|field| touched.contains(field) && !masked.contains(field))
        .collect();

    let mut notes = Vec::new();
    if !waiting.is_empty() {
        notes.push(format!(
            "{} need(s) a restart to take effect ({})",
            waiting.len(),
            waiting.join(", ")
        ));
    }
    if !masked.is_empty() {
        notes.push(format!(
            "{} not in effect, shadowed by a higher layer ({})",
            masked.len(),
            masked.join(", ")
        ));
    }

    if notes.is_empty() {
        format!("Stored {} setting(s); all are in effect", touched.len())
    } else {
        format!("Stored {} setting(s); {}", touched.len(), notes.join("; "))
    }
}

/// Remove one persisted setting.
async fn delete_stored(
    State(state): State<AdminState>,
    axum::extract::Path(field): axum::extract::Path<String>,
) -> Result<Json<StoredResponse>> {
    if !state.config.get().admin_config_write {
        state.metrics.system.config_patch_failures.add(1, &[]);
        return Err(AppError::Forbidden(
            "runtime configuration writes are disabled (ADMIN_CONFIG_WRITE=false)".to_string(),
        ));
    }

    let Some(param) = by_field(&field) else {
        state.metrics.system.config_patch_failures.add(1, &[]);
        return Err(AppError::InvalidConfig(format!(
            "`{field}` is not a configuration field"
        )));
    };

    let mut candidate: Layer = state.config.stored();
    candidate.remove(param.env);

    // Validate before writing, for the same reason `put_stored` does. Removing a value can fail
    // to resolve just as setting one can — the layer underneath it is free to have become
    // invalid since boot — and deleting the row first would leave the store changed while the
    // request reports failure and the running configuration is untouched.
    let handle = state.config.clone();
    let probe = candidate.clone();
    tokio::task::spawn_blocking(move || handle.dry_run(&probe))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("config dry run failed: {e}")))?
        .map_err(|e| {
            state.metrics.system.config_patch_failures.add(1, &[]);
            AppError::InvalidConfig(e)
        })?;

    if !state.store.unset(&field).await? {
        state.metrics.system.config_patch_failures.add(1, &[]);
        return Err(AppError::InvalidConfig(format!(
            "`{field}` is not stored, so there is nothing to remove"
        )));
    }

    let handle = state.config.clone();
    let outcome = tokio::task::spawn_blocking(move || handle.adopt_stored(candidate))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("config adopt task failed: {e}")))?
        .map_err(AppError::InvalidConfig)?;

    tracing::info!("Removed stored config: {field}");

    // Same reasoning as `put_stored`: a restart may well be pending for some other field, but
    // that is not something removing this one did.
    let message = if outcome.drift.contains(&param.field) {
        format!("Removed `{field}`; a restart is needed to take effect")
    } else {
        format!("Removed `{field}`")
    };
    describe_stored(&state, message).await
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

/// Restart the server so stored boot settings take effect.
///
/// Off by default. A remote restart endpoint is an availability lever and, if the master key
/// ever leaks, a denial-of-service amplifier, so it has to be turned on deliberately.
async fn restart_server(State(state): State<AdminState>) -> Result<Json<RestartResponse>> {
    let running = state.config.get();
    if !running.admin_restart_enabled {
        return Err(AppError::RestartNotEnabled(
            "restarting from the API is disabled (ADMIN_RESTART_ENABLED=false)".to_string(),
        ));
    }

    let Some(handle) = state.restart.clone() else {
        return Err(AppError::RestartNotEnabled(
            "this server was not started in a way that can restart itself".to_string(),
        ));
    };

    // The failure validation cannot see. `server_port` is exactly the field an operator edits
    // through a web form, and "already in use" is only discoverable by trying — after which a
    // restart would leave the service down rather than merely unchanged.
    let stored = state.config.stored();
    let candidate = tokio::task::spawn_blocking({
        let config = state.config.clone();
        move || config.dry_run(&stored)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("config dry run failed: {e}")))?
    .map_err(AppError::InvalidConfig)?;

    let (next, current) = (candidate.server_address(), running.server_address());
    let (next, current) = (
        next.parse::<std::net::SocketAddr>()
            .map_err(|e| AppError::InvalidConfig(format!("invalid address {next}: {e}")))?,
        current
            .parse::<std::net::SocketAddr>()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("running address unparseable: {e}")))?,
    );
    tokio::task::spawn_blocking(move || preflight_bind(next, current))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("preflight task failed: {e}")))?
        .map_err(AppError::AddressUnavailable)?;

    let applying = state.config.pending_restart();
    let first = handle.request();

    tracing::warn!(
        "Restart requested through the admin API (applying: {})",
        if applying.is_empty() {
            "nothing".to_string()
        } else {
            applying.join(", ")
        }
    );
    state.metrics.system.config_restarts.add(1, &[]);

    let message = if first {
        "Restarting; the server will be unavailable briefly".to_string()
    } else {
        "A restart is already in progress".to_string()
    };
    Ok(Json(RestartResponse { applying, message }))
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
