//! Shared, reloadable configuration.
//!
//! The running [`Config`] lives behind a [`tokio::sync::watch`] channel: readers take a cheap
//! snapshot, and the cleanup task gets change *notification* for free, which it needs to pick
//! up a new cleanup interval. `tokio` is already compiled with the `sync` feature, so this
//! costs no new dependency.
//!
//! Only the fields read per request or per tick can move. Everything else is captured during
//! startup — into the listener, the connection pool, the middleware, and the rate limiter —
//! and a reload reports drift on those rather than pretending to apply it.

use super::params::{by_env, Reload};
use super::sources::Layer;
use super::{Config, RealEnv};
use crate::cli::{load_layers, Cli};
use crate::models::SessionConfig;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// State that a reload mutates, kept behind one lock so a SIGHUP and an admin request cannot
/// interleave into a lost update.
struct ReloadState {
    /// The command-line arguments this process started with, retained so a reload resolves
    /// from exactly the same sources as boot.
    cli: Cli,
    /// Whether re-resolution reads the process environment.
    ///
    /// Always true in production. `from_static` sets it false so a test handle resolves against
    /// the layers it was given and nothing else — otherwise a developer or CI job that happens
    /// to export `CAPTCHA_COMPRESSION` would fail unrelated reload tests.
    read_process_env: bool,
    /// Values set through the admin API, keyed by canonical environment key.
    ///
    /// Ephemeral by design: a reload means "re-read the sources of truth", so it clears these.
    overlay: Layer,
}

struct Inner {
    tx: watch::Sender<Arc<Config>>,
    state: Mutex<ReloadState>,
}

/// A cloneable handle to the running configuration.
#[derive(Clone)]
pub struct ConfigHandle {
    rx: watch::Receiver<Arc<Config>>,
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ConfigHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegates to Config's redacting Debug impl.
        f.debug_struct("ConfigHandle")
            .field("config", &*self.get())
            .finish()
    }
}

impl ConfigHandle {
    /// Create a handle that can be reloaded from the same arguments the process started with.
    pub fn new(config: Config, cli: Cli) -> Self {
        let (tx, rx) = watch::channel(Arc::new(config));
        Self {
            rx,
            inner: Arc::new(Inner {
                tx,
                state: Mutex::new(ReloadState {
                    cli,
                    read_process_env: true,
                    overlay: Layer::new(),
                }),
            }),
        }
    }

    /// Create a handle over a fixed configuration.
    ///
    /// Reloading re-resolves from no command-line arguments and no env file, so a test harness
    /// never picks up a developer's local `.env`.
    pub fn from_static(config: Config) -> Self {
        let handle = Self::new(
            config,
            Cli {
                no_env_file: true,
                ..Cli::default()
            },
        );
        handle
            .inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_process_env = false;
        handle
    }

    /// Take a snapshot of the current configuration.
    ///
    /// Clones the `Arc` immediately so the `watch` read guard is never held by the caller —
    /// holding one across an `.await` would block every writer.
    pub fn get(&self) -> Arc<Config> {
        self.rx.borrow().clone()
    }

    /// Take a snapshot of just the per-request fields.
    ///
    /// [`SessionConfig`] is `Copy`, so this is a plain register copy rather than an `Arc` clone.
    pub fn session_config(&self) -> SessionConfig {
        self.rx.borrow().session_config()
    }

    /// Subscribe to configuration changes, for tasks that must react to them.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Config>> {
        self.rx.clone()
    }

    /// Re-read every configuration source and publish the result.
    ///
    /// Live fields are adopted; boot-only fields are pinned to the running process and any
    /// difference is returned so the caller can report it. The admin overlay is cleared.
    ///
    /// Blocking: reads files. Async callers must wrap this in `spawn_blocking`.
    /// On failure the running configuration is left untouched.
    pub fn reload(&self) -> Result<Outcome, String> {
        // Held across resolve *and* publish, so two concurrent reloads serialise instead of
        // racing to `send_replace` in the wrong order.
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());

        let current = self.rx.borrow().clone();
        let resolved =
            resolve_with_carry(&state.cli, &current, &Layer::new(), state.read_process_env)?;

        let drift = current.boot_drift(&resolved);
        let merged = Arc::new(current.with_boot_fields_from(&resolved));

        state.overlay.clear();
        self.inner.tx.send_replace(merged.clone());

        Ok(Outcome {
            config: merged,
            drift,
        })
    }

    /// Apply an in-memory override to live fields, on top of the resolved configuration.
    ///
    /// Keys are canonical environment keys. Boot-only keys are **rejected** rather than
    /// accepted-and-discarded: silently recording an override that provably cannot take effect
    /// would make `overlay_keys` — and so the admin API — lie about the running state.
    ///
    /// Rejected values leave the running configuration untouched. The override survives until
    /// the next reload or restart, and is never written back to any file.
    ///
    /// Blocking: reads files. Async callers must wrap this in `spawn_blocking`.
    pub fn patch(&self, updates: &[(String, String)]) -> Result<Arc<Config>, String> {
        for (key, _) in updates {
            match by_env(key) {
                Some(param) if param.reload == Reload::Live => {}
                Some(param) => {
                    return Err(format!(
                        "`{}` is applied at startup and cannot be changed at runtime",
                        param.field
                    ))
                }
                None => return Err(format!("`{key}` is not a configuration key")),
            }
        }

        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());

        let mut overlay = state.overlay.clone();
        for (key, value) in updates {
            overlay.insert(key.clone(), value.clone());
        }

        let current = self.rx.borrow().clone();
        let resolved = resolve_with_carry(&state.cli, &current, &overlay, state.read_process_env)?;
        let merged = Arc::new(current.with_boot_fields_from(&resolved));

        state.overlay = overlay;
        self.inner.tx.send_replace(merged.clone());

        Ok(merged)
    }

    /// The canonical keys currently overridden through the admin API.
    pub fn overlay_keys(&self) -> Vec<String> {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.overlay.keys().cloned().collect()
    }
}

/// The result of a successful reload.
#[derive(Debug)]
pub struct Outcome {
    /// The newly published configuration.
    pub config: Arc<Config>,
    /// Boot-only fields whose freshly resolved value differs from the running one. These were
    /// *not* applied; the caller should tell the operator why nothing happened.
    pub drift: Vec<&'static str>,
}

/// Resolve from the process's original arguments, with two extra layers.
///
/// `overlay` sits at the top (above the command line) because an operator changing a value
/// through the admin API means it now. `current.boot_layer()` sits at the bottom so a reload
/// cannot fail merely because a secret file was rotated away or unmounted after startup —
/// those fields are boot-only and would have been discarded regardless.
fn resolve_with_carry(
    cli: &Cli,
    current: &Config,
    overlay: &Layer,
    read_process_env: bool,
) -> Result<Config, String> {
    let mut layers = load_layers(cli)?;
    for (key, value) in overlay {
        layers.cli.insert(key.clone(), value.clone());
    }
    layers.carried = current.boot_layer();

    if read_process_env {
        Config::from_env_provider(&layers.stack(&RealEnv))
    } else {
        Config::from_env_provider(&layers.stack(&EmptyEnv))
    }
}

/// An environment with nothing in it, for handles that must not read the process environment.
struct EmptyEnv;

impl crate::config::EnvProvider for EmptyEnv {
    fn get(&self, _key: &str) -> Result<String, std::env::VarError> {
        Err(std::env::VarError::NotPresent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> ConfigHandle {
        ConfigHandle::from_static(Config::for_test())
    }

    #[test]
    fn test_get_returns_the_current_config() {
        let h = handle();
        assert_eq!(h.get().server_port, 3000);
        assert_eq!(h.get().captcha_compression, 40);
    }

    #[test]
    fn test_session_config_snapshot_matches_config() {
        let h = handle();
        assert_eq!(h.session_config(), h.get().session_config());
    }

    #[test]
    fn test_clones_share_one_configuration() {
        let a = handle();
        let b = a.clone();
        a.patch(&[("CAPTCHA_COMPRESSION".into(), "88".into())])
            .unwrap();
        assert_eq!(b.get().captcha_compression, 88);
    }

    #[test]
    fn test_patch_changes_a_live_field() {
        let h = handle();
        let updated = h
            .patch(&[("MAX_VALIDATION_ATTEMPTS".into(), "7".into())])
            .unwrap();

        assert_eq!(updated.max_validation_attempts, 7);
        assert_eq!(h.get().max_validation_attempts, 7);
        assert_eq!(h.session_config().max_validation_attempts, 7);
    }

    #[test]
    fn test_patch_accumulates_across_calls() {
        let h = handle();
        h.patch(&[("CAPTCHA_COMPRESSION".into(), "70".into())])
            .unwrap();
        h.patch(&[("MAX_VALIDATION_ATTEMPTS".into(), "5".into())])
            .unwrap();

        let config = h.get();
        assert_eq!(config.captcha_compression, 70);
        assert_eq!(config.max_validation_attempts, 5);
    }

    #[test]
    fn test_patch_rejects_a_boot_field_outright() {
        // Accepting it and quietly discarding the value would leave the key in the overlay,
        // so the admin API would report an override that provably has no effect.
        let h = handle();
        let before = h.get().server_port;

        let err = h
            .patch(&[("SERVER_PORT".into(), "9999".into())])
            .unwrap_err();

        assert!(err.contains("server_port"), "{err}");
        assert!(err.contains("applied at startup"), "{err}");
        assert_eq!(h.get().server_port, before);
        assert!(
            h.overlay_keys().is_empty(),
            "a rejected patch must not be recorded: {:?}",
            h.overlay_keys()
        );
    }

    #[test]
    fn test_patch_rejects_an_unknown_key() {
        let h = handle();
        let err = h
            .patch(&[("NOT_A_SETTING".into(), "1".into())])
            .unwrap_err();
        assert!(err.contains("not a configuration key"), "{err}");
        assert!(h.overlay_keys().is_empty());
    }

    #[test]
    fn test_patch_rejects_the_whole_batch_if_any_key_is_invalid() {
        // All-or-nothing: a partially applied batch would be worse than a clean rejection.
        let h = handle();
        let err = h
            .patch(&[
                ("CAPTCHA_COMPRESSION".into(), "90".into()),
                ("SERVER_PORT".into(), "9999".into()),
            ])
            .unwrap_err();

        assert!(err.contains("server_port"), "{err}");
        assert_eq!(
            h.get().captcha_compression,
            40,
            "nothing should have applied"
        );
        assert!(h.overlay_keys().is_empty());
    }

    #[test]
    fn test_patch_rejects_an_invalid_value_and_keeps_the_old_config() {
        let h = handle();
        let err = h
            .patch(&[("MAX_VALIDATION_ATTEMPTS".into(), "not-a-number".into())])
            .unwrap_err();

        assert!(err.contains("Invalid MAX_VALIDATION_ATTEMPTS"), "{err}");
        assert_eq!(
            h.get().max_validation_attempts,
            3,
            "config must be untouched"
        );
        assert!(
            h.overlay_keys().is_empty(),
            "a failed patch must not be recorded"
        );
    }

    #[test]
    fn test_overlay_keys_reports_what_was_patched() {
        let h = handle();
        h.patch(&[("CAPTCHA_COMPRESSION".into(), "70".into())])
            .unwrap();
        assert_eq!(h.overlay_keys(), vec!["CAPTCHA_COMPRESSION".to_string()]);
    }

    #[test]
    fn test_reload_clears_the_overlay() {
        let h = handle();
        h.patch(&[("CAPTCHA_COMPRESSION".into(), "88".into())])
            .unwrap();
        assert_eq!(h.get().captcha_compression, 88);

        let outcome = h.reload().unwrap();

        assert_eq!(
            outcome.config.captcha_compression, 40,
            "reload re-reads the sources of truth, discarding admin overrides"
        );
        assert!(h.overlay_keys().is_empty());
    }

    #[test]
    fn test_reload_preserves_boot_fields() {
        let h = ConfigHandle::from_static(Config {
            server_port: 4321,
            api_key_salt: "a-distinctive-salt-value".to_string(),
            ..Config::for_test()
        });

        let outcome = h.reload().unwrap();

        assert_eq!(outcome.config.server_port, 4321);
        assert_eq!(outcome.config.api_key_salt, "a-distinctive-salt-value");
    }

    #[test]
    fn test_reload_succeeds_without_any_secret_in_the_environment() {
        // The scenario the carried layer exists for: secrets came from files that have since
        // been rotated away. Reload must still work, because those fields are boot-only.
        let h = handle();
        assert!(h.reload().is_ok());
    }

    #[test]
    fn test_reload_reports_no_drift_when_nothing_changed() {
        assert!(handle().reload().unwrap().drift.is_empty());
    }

    #[test]
    fn test_reload_reports_drift_when_a_source_names_a_different_boot_value() {
        // Models an operator editing the port and sending SIGHUP: the source now says 9999 but
        // the listener is already bound to 3000. The reload must say so rather than pretend.
        let h = ConfigHandle::new(
            Config::for_test(),
            Cli {
                values: [("SERVER_PORT".to_string(), "9999".to_string())]
                    .into_iter()
                    .collect(),
                no_env_file: true,
                ..Cli::default()
            },
        );

        let outcome = h.reload().unwrap();

        assert!(
            outcome.drift.contains(&"server_port"),
            "expected server_port in {:?}",
            outcome.drift
        );
        assert_eq!(
            outcome.config.server_port, 3000,
            "drift is reported, never applied"
        );
    }

    #[test]
    fn test_static_handles_ignore_the_process_environment() {
        // Otherwise a developer (or CI job) with CAPTCHA_COMPRESSION exported would fail every
        // reload test in this module for reasons that have nothing to do with the code.
        //
        // Safety: this is the only test that touches process env, and it restores it. The
        // assertion is precisely that the handle does not observe it.
        let key = "CAPTCHA_COMPRESSION";
        let previous = std::env::var(key).ok();
        std::env::set_var(key, "7");

        let h = handle();
        let outcome = h.reload().unwrap();

        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }

        assert_eq!(
            outcome.config.captcha_compression, 40,
            "a static handle must resolve from its own layers only"
        );
    }

    #[test]
    fn test_reload_reports_no_drift_when_a_boot_field_is_simply_unset() {
        // An unspecified boot field resolves to the running value via the carried layer, so
        // there is nothing to warn about — only an explicit, conflicting source is drift.
        let h = ConfigHandle::from_static(Config {
            server_port: 4321,
            ..Config::for_test()
        });

        let outcome = h.reload().unwrap();

        assert!(outcome.drift.is_empty(), "{:?}", outcome.drift);
        assert_eq!(outcome.config.server_port, 4321);
    }

    #[tokio::test]
    async fn test_subscribers_are_notified_of_changes() {
        let h = handle();
        let mut rx = h.subscribe();

        h.patch(&[("CLEANUP_INTERVAL_SECONDS".into(), "5".into())])
            .unwrap();

        rx.changed().await.expect("sender is alive");
        assert_eq!(rx.borrow().cleanup_interval_seconds, 5);
    }

    #[tokio::test]
    async fn test_concurrent_reloads_converge() {
        // Two reloads racing (SIGHUP plus an admin request) must serialise, not interleave
        // resolve/publish and leave the older result winning.
        let h = handle();
        let (a, b) = (h.clone(), h.clone());

        let (ra, rb) = tokio::join!(
            tokio::task::spawn_blocking(move || a.reload().map(|o| o.config.captcha_compression)),
            tokio::task::spawn_blocking(move || b.reload().map(|o| o.config.captcha_compression)),
        );

        assert_eq!(ra.unwrap().unwrap(), 40);
        assert_eq!(rb.unwrap().unwrap(), 40);
        assert_eq!(h.get().captcha_compression, 40);
    }

    #[tokio::test]
    async fn test_concurrent_patch_and_reload_leave_a_consistent_config() {
        let h = handle();
        let (a, b) = (h.clone(), h.clone());

        let (_, _) = tokio::join!(
            tokio::task::spawn_blocking(move || a
                .patch(&[("CAPTCHA_COMPRESSION".into(), "77".into())])
                .map(|c| c.captcha_compression)),
            tokio::task::spawn_blocking(move || b.reload().map(|o| o.config.captcha_compression)),
        );

        // Whichever ran last, the published value must be one of the two legitimate outcomes
        // and must agree with the recorded overlay.
        let compression = h.get().captcha_compression;
        let patched = h
            .overlay_keys()
            .contains(&"CAPTCHA_COMPRESSION".to_string());
        if patched {
            assert_eq!(compression, 77);
        } else {
            assert_eq!(compression, 40);
        }
    }

    #[test]
    fn test_debug_does_not_leak_secrets() {
        let rendered = format!("{:?}", handle());
        assert!(
            !rendered.contains("test-salt-minimum-16chars"),
            "{rendered}"
        );
        assert!(rendered.contains("<redacted"), "{rendered}");
    }
}
