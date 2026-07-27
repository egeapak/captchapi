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
use super::sources::{Layer, Source, Sources};
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
    /// Where re-resolution reads environment variables from.
    ///
    /// Always [`EnvSource::Process`] in production. `from_static` uses [`EnvSource::Empty`] so a
    /// test handle resolves against the layers it was given and nothing else — otherwise a
    /// developer or CI job that happens to export `CAPTCHA_COMPRESSION` would fail unrelated
    /// reload tests.
    env: EnvSource,
    /// Values set through the admin API, keyed by canonical environment key.
    ///
    /// Ephemeral by design: a reload means "re-read the sources of truth", so it clears these.
    overlay: Layer,
    /// Which layer answered for each field, as of the last resolve. Kept beside the overlay
    /// under the same lock so the two can never disagree about what the running config is.
    sources: Sources,
    /// The persisted settings, as of the last time a caller read them.
    ///
    /// Retained rather than fetched, because re-resolving happens under this lock in a
    /// synchronous function and the store is async. Callers that know the store has changed —
    /// a reload, a write to it — pass a fresh layer in; `patch` reuses whatever is here, so an
    /// ephemeral override always lands on top of the same stored values the server is running.
    stored: Layer,
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
    ///
    /// `sources` is the provenance captured during the boot resolve. It is passed in rather
    /// than recomputed here so the handle reports the layers this process actually started
    /// from, not what a second read of the same files would say a moment later.
    pub fn new(config: Config, cli: Cli, sources: Sources, stored: Layer) -> Self {
        let (tx, rx) = watch::channel(Arc::new(config));
        Self {
            rx,
            inner: Arc::new(Inner {
                tx,
                state: Mutex::new(ReloadState {
                    cli,
                    env: EnvSource::Process,
                    overlay: Layer::new(),
                    sources,
                    stored,
                }),
            }),
        }
    }

    /// Create a handle over a fixed configuration.
    ///
    /// Reloading re-resolves from no command-line arguments and no env file, so a test harness
    /// never picks up a developer's local `.env`.
    pub fn from_static(config: Config) -> Self {
        // No layers, so every field reports `Source::Default` and nothing is pinned. That is
        // both accurate for a hand-built config and the right default for tests, which should
        // have to opt into pinning rather than trip over it.
        let handle = Self::new(
            config,
            Cli {
                no_env_file: true,
                ..Cli::default()
            },
            Sources::default(),
            Layer::new(),
        );
        handle
            .inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .env = EnvSource::Empty;
        handle
    }

    /// Like [`Self::from_static`], but as if the process had been started with these
    /// command-line values, so a test can exercise provenance and the pinning it drives.
    ///
    /// Resolves once to capture that provenance, against an empty environment for the same
    /// reason `from_static` does: a developer who happens to export `CAPTCHA_COMPRESSION`
    /// must not be able to change what an unrelated test sees.
    pub fn from_static_with_cli(config: Config, values: &[(&str, &str)]) -> Result<Self, String> {
        let handle = Self::from_static(config);
        {
            let mut state = handle.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            state.cli.values = values
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        }
        handle.reload(Layer::new())?;
        Ok(handle)
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
    pub fn reload(&self, stored: Layer) -> Result<Outcome, String> {
        // Held across resolve *and* publish, so two concurrent reloads serialise instead of
        // racing to `send_replace` in the wrong order.
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());

        let current = self.rx.borrow().clone();
        let (resolved, sources) =
            resolve_with_carry(&state.cli, &current, &Layer::new(), &stored, &state.env)?;

        let drift = current.boot_drift(&resolved);
        let merged = Arc::new(current.with_boot_fields_from(&resolved));

        state.overlay.clear();
        state.sources = sources;
        state.stored = stored;
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

        // Checked under the lock, against the provenance of the last resolve, so a concurrent
        // reload cannot move a field between pinned and free between the check and the write.
        for (key, _) in updates {
            let Some(param) = by_env(key) else { continue };
            let source = state.sources.get(param.field);
            if source.is_pinned() {
                return Err(format!(
                    "`{}` is set on the {} and cannot be changed at runtime; \
                     the override would be discarded by the next reload",
                    param.field,
                    match source {
                        Source::Cli => "command line",
                        _ => "environment",
                    }
                ));
            }
        }

        // `log_level` is the one live field whose value has a syntax that
        // `Config::from_env_provider` does not check: it stores the directives verbatim, and
        // `log_filter` quietly substitutes a default if they turn out to be unparseable.
        //
        // That fallback is right at boot — refusing to start over a log directive would be
        // worse than ignoring it — and wrong here, because now there is a running filter to
        // diverge from. Accepting a value that cannot be installed would leave `GET /config`
        // reporting a filter the server is not using.
        for (key, value) in updates {
            if key == "RUST_LOG" {
                value
                    .parse::<tracing_subscriber::filter::Targets>()
                    .map_err(|e| format!("Invalid RUST_LOG: unparseable log filter: {e}"))?;
            }
        }

        let mut overlay = state.overlay.clone();
        for (key, value) in updates {
            overlay.insert(key.clone(), value.clone());
        }

        let current = self.rx.borrow().clone();
        let (resolved, sources) = resolve_with_carry(
            &state.cli,
            &current,
            &overlay,
            &state.stored.clone(),
            &state.env,
        )?;
        let merged = Arc::new(current.with_boot_fields_from(&resolved));

        state.overlay = overlay;
        state.sources = sources;
        self.inner.tx.send_replace(merged.clone());

        Ok(merged)
    }

    /// The persisted layer this handle last resolved against.
    pub fn stored(&self) -> Layer {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.stored.clone()
    }

    /// The canonical keys currently overridden through the admin API.
    pub fn overlay_keys(&self) -> Vec<String> {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.overlay.keys().cloned().collect()
    }

    /// Which layer supplied each field, as of the last resolve.
    pub fn sources(&self) -> Sources {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.sources.clone()
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
    stored: &Layer,
    env: &EnvSource,
) -> Result<(Config, Sources), String> {
    let mut layers = load_layers(cli)?;
    layers.overlay = overlay.clone();
    layers.stored = stored.clone();
    layers.carried = current.boot_layer();

    let stack = layers.stack(env);
    Ok((Config::from_env_provider(&stack)?, Sources::capture(&stack)))
}

/// Where a handle's reload reads environment variables from.
///
/// This is a value rather than a `bool` so the environment is a seam the tests can drive,
/// upholding the rule stated in `sources.rs`: tests must never call `std::env::set_var`.
/// `cargo llvm-cov` runs the threaded `cargo test` harness rather than nextest's
/// process-per-test, so a test that exports a variable and restores it afterwards is a data
/// race against every concurrent `env::var` in the same binary — and skips the restore
/// entirely if anything between the two panics.
enum EnvSource {
    /// The process environment. Always this in production.
    Process,
    /// Nothing at all, for handles that must resolve from their own layers only.
    Empty,
    /// A fixed set, so a test can prove the environment layer is consulted without touching
    /// the real one.
    #[cfg(test)]
    Fixed(std::collections::HashMap<String, String>),
}

impl crate::config::EnvProvider for EnvSource {
    fn get(&self, key: &str) -> Result<String, std::env::VarError> {
        match self {
            Self::Process => RealEnv.get(key),
            Self::Empty => Err(std::env::VarError::NotPresent),
            #[cfg(test)]
            Self::Fixed(values) => values
                .get(key)
                .cloned()
                .ok_or(std::env::VarError::NotPresent),
        }
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

    /// Build a handle whose environment holds `pairs`, resolved so provenance is real rather
    /// than hand-stubbed — the pinning check is only worth anything if it reads the same
    /// provenance a live boot would produce.
    fn handle_with_env(pairs: &[(&str, &str)]) -> ConfigHandle {
        let h = handle();
        with_env(&h, pairs);
        h.reload(Layer::new()).expect("the fixture must resolve");
        h
    }

    #[test]
    fn test_patch_accepts_a_valid_log_filter() {
        let h = handle();
        let updated = h
            .patch(&[("RUST_LOG".into(), "captchapi=trace".into())])
            .unwrap();
        assert_eq!(updated.log_level, "captchapi=trace");
    }

    #[test]
    fn test_patch_refuses_a_log_filter_it_could_not_install() {
        // Boot tolerates unparseable directives and falls back to a default. At runtime that
        // would leave the reported configuration describing a filter the server never
        // installed, so the patch is refused instead.
        let h = handle();
        let before = h.get().log_level.clone();

        let err = h
            .patch(&[("RUST_LOG".into(), "=:=nonsense=:=".into())])
            .unwrap_err();

        assert!(err.contains("unparseable log filter"), "{err}");
        assert_eq!(h.get().log_level, before, "config must be untouched");
        assert!(
            h.overlay_keys().is_empty(),
            "a failed patch is not recorded"
        );
    }

    #[test]
    fn test_patch_is_refused_for_a_field_set_in_the_environment() {
        let h = handle_with_env(&[("CAPTCHA_COMPRESSION", "70")]);
        assert_eq!(h.get().captcha_compression, 70);

        let err = h
            .patch(&[("CAPTCHA_COMPRESSION".into(), "90".into())])
            .unwrap_err();

        assert!(err.contains("environment"), "{err}");
        assert_eq!(h.get().captcha_compression, 70, "config must be untouched");
        assert!(h.overlay_keys().is_empty());
    }

    #[test]
    fn test_patch_is_refused_for_a_field_set_on_the_command_line() {
        let h = ConfigHandle::new(
            Config::for_test(),
            Cli {
                values: [("CAPTCHA_COMPRESSION".to_string(), "70".to_string())]
                    .into_iter()
                    .collect(),
                no_env_file: true,
                ..Cli::default()
            },
            Sources::default(),
            Layer::new(),
        );
        h.reload(Layer::new()).unwrap();

        let err = h
            .patch(&[("CAPTCHA_COMPRESSION".into(), "90".into())])
            .unwrap_err();

        assert!(err.contains("command line"), "{err}");
        assert_eq!(h.get().captcha_compression, 70);
    }

    #[test]
    fn test_a_field_the_environment_does_not_set_stays_patchable() {
        // The negative control: the refusal above must come from provenance, not from the
        // mere presence of *some* environment.
        let h = handle_with_env(&[("CAPTCHA_COMPRESSION", "70")]);

        h.patch(&[("MAX_VALIDATION_ATTEMPTS".into(), "7".into())])
            .expect("an unpinned field is still patchable");

        assert_eq!(h.get().max_validation_attempts, 7);
    }

    #[test]
    fn test_patching_a_field_does_not_pin_it_against_being_patched_again() {
        // The admin overlay outranks the command line, so if it were merged into that layer a
        // field would report itself as `cli`-set after one patch and refuse the next.
        let h = handle();
        h.patch(&[("CAPTCHA_COMPRESSION".into(), "70".into())])
            .unwrap();

        assert_eq!(h.sources().get("captcha_compression"), Source::Admin);

        h.patch(&[("CAPTCHA_COMPRESSION".into(), "90".into())])
            .expect("a field this API set must remain settable by it");
        assert_eq!(h.get().captcha_compression, 90);
    }

    #[test]
    fn test_a_rejected_batch_leaves_the_unpinned_fields_alone() {
        let h = handle_with_env(&[("CAPTCHA_COMPRESSION", "70")]);

        let err = h
            .patch(&[
                ("MAX_VALIDATION_ATTEMPTS".into(), "7".into()),
                ("CAPTCHA_COMPRESSION".into(), "90".into()),
            ])
            .unwrap_err();

        assert!(err.contains("captcha_compression"), "{err}");
        assert_eq!(
            h.get().max_validation_attempts,
            3,
            "all or nothing: the whole batch is refused"
        );
        assert!(h.overlay_keys().is_empty());
    }

    #[test]
    fn test_reload_unpins_a_field_once_the_environment_stops_setting_it() {
        let h = handle_with_env(&[("CAPTCHA_COMPRESSION", "70")]);
        assert!(h.sources().get("captcha_compression").is_pinned());

        with_env(&h, &[]);
        h.reload(Layer::new()).unwrap();

        assert!(!h.sources().get("captcha_compression").is_pinned());
        h.patch(&[("CAPTCHA_COMPRESSION".into(), "90".into())])
            .expect("no longer pinned");
    }

    #[test]
    fn test_sources_reports_the_layer_each_field_came_from() {
        let h = handle_with_env(&[("CAPTCHA_COMPRESSION", "70")]);
        let sources = h.sources();

        assert_eq!(sources.get("captcha_compression"), Source::Env);
        assert_eq!(sources.get("max_validation_attempts"), Source::Default);
        // A field nobody has heard of is unpinned, so the check fails open to "editable".
        assert!(!sources.is_pinned("not_a_field"));
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

        let outcome = h.reload(Layer::new()).unwrap();

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

        let outcome = h.reload(Layer::new()).unwrap();

        assert_eq!(outcome.config.server_port, 4321);
        assert_eq!(outcome.config.api_key_salt, "a-distinctive-salt-value");
    }

    #[test]
    fn test_reload_succeeds_without_any_secret_in_the_environment() {
        // The scenario the carried layer exists for: secrets came from files that have since
        // been rotated away. Reload must still work, because those fields are boot-only.
        let h = handle();
        assert!(h.reload(Layer::new()).is_ok());
    }

    #[test]
    fn test_reload_reports_no_drift_when_nothing_changed() {
        assert!(handle().reload(Layer::new()).unwrap().drift.is_empty());
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
            Sources::default(),
            Layer::new(),
        );

        let outcome = h.reload(Layer::new()).unwrap();

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

    /// Replace a handle's environment with a fixed set, as `from_static` replaces it with none.
    fn with_env(h: &ConfigHandle, pairs: &[(&str, &str)]) {
        h.inner.state.lock().unwrap_or_else(|e| e.into_inner()).env = EnvSource::Fixed(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
    }

    /// Positive control for the two tests below.
    ///
    /// Without this, "the environment was ignored" could equally mean the environment layer is
    /// not wired into a reload at all, or that 7 is not a value that can take effect.
    #[test]
    fn test_reload_reads_the_environment_layer() {
        let h = handle();
        with_env(&h, &[("CAPTCHA_COMPRESSION", "7")]);

        let outcome = h.reload(Layer::new()).unwrap();

        assert_eq!(
            outcome.config.captcha_compression, 7,
            "a reload must consult its environment source"
        );
    }

    #[test]
    fn test_an_empty_environment_leaves_the_layers_to_answer() {
        let h = handle();
        with_env(&h, &[("CAPTCHA_COMPRESSION", "7")]);
        // Back to what `from_static` installs.
        h.inner.state.lock().unwrap_or_else(|e| e.into_inner()).env = EnvSource::Empty;

        let outcome = h.reload(Layer::new()).unwrap();

        assert_eq!(
            outcome.config.captcha_compression, 40,
            "an empty environment must not answer for CAPTCHA_COMPRESSION"
        );
    }

    /// Ask a handle's environment source for a key, by the same trait method `resolve_with_carry`
    /// calls. Asserting on what the source *answers* rather than on which variant it is keeps the
    /// test honest if the seam is ever reshaped.
    fn env_get(h: &ConfigHandle, key: &str) -> Result<String, std::env::VarError> {
        use crate::config::EnvProvider;
        h.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .env
            .get(key)
    }

    /// The invariant the two tests above exist to support: a static handle is wired to an empty
    /// environment, so a developer or CI job that happens to export `CAPTCHA_COMPRESSION` cannot
    /// fail unrelated reload tests. Production keeps the process environment.
    ///
    /// `PATH` is the discriminator. It is set in every environment this suite can run in, and no
    /// config parameter reads it, so it distinguishes the two wirings without any test needing to
    /// export a variable — reading the environment is safe, only writing it is banned. Using a
    /// real, already-present variable is also the only way to cover `EnvSource::Process`'s
    /// passthrough to `RealEnv` end-to-end.
    #[test]
    fn test_static_handles_ignore_the_process_environment() {
        let in_process = std::env::var("PATH").expect("this test presumes PATH is set");

        let statik = ConfigHandle::from_static(Config::for_test());
        assert_eq!(
            env_get(&statik, "PATH"),
            Err(std::env::VarError::NotPresent),
            "from_static must not read the process environment, but it answered for PATH"
        );

        let live = ConfigHandle::new(
            Config::for_test(),
            Cli::default(),
            Sources::default(),
            Layer::new(),
        );
        assert_eq!(
            env_get(&live, "PATH").as_deref(),
            Ok(in_process.as_str()),
            "a production handle must read the process environment"
        );
        assert_eq!(
            env_get(&live, "CAPTCHAPI_DEFINITELY_NOT_SET_IN_ANY_ENVIRONMENT"),
            Err(std::env::VarError::NotPresent),
            "a production handle must report a genuinely absent variable as absent"
        );
    }

    #[test]
    fn test_empty_env_source_answers_nothing() {
        use crate::config::EnvProvider;
        for key in ["CAPTCHA_COMPRESSION", "SERVER_PORT", "PATH"] {
            assert!(
                EnvSource::Empty.get(key).is_err(),
                "{key} should be absent from an empty environment"
            );
        }
    }

    #[test]
    fn test_reload_reports_no_drift_when_a_boot_field_is_simply_unset() {
        // An unspecified boot field resolves to the running value via the carried layer, so
        // there is nothing to warn about — only an explicit, conflicting source is drift.
        let h = ConfigHandle::from_static(Config {
            server_port: 4321,
            ..Config::for_test()
        });

        let outcome = h.reload(Layer::new()).unwrap();

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
            tokio::task::spawn_blocking(move || a
                .reload(Layer::new())
                .map(|o| o.config.captcha_compression)),
            tokio::task::spawn_blocking(move || b
                .reload(Layer::new())
                .map(|o| o.config.captcha_compression)),
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
            tokio::task::spawn_blocking(move || b
                .reload(Layer::new())
                .map(|o| o.config.captcha_compression)),
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
