//! The second phase of startup: folding the persisted settings into the configuration.
//!
//! Lives here rather than inline in `main.rs` because it makes a decision with a sharp edge —
//! whether the generation this process booted with may be confirmed — and `main.rs` is
//! effectively invisible to the test suite.

use super::sources::{Layer, Sources};
use super::Config;
use crate::cli::{resolve_with_stored, Cli};

/// The outcome of folding the store into the first-pass configuration.
#[derive(Debug)]
pub struct Booted {
    pub config: Config,
    pub sources: Sources,
    /// The generation to confirm once the process has proved it can serve, or `None` when
    /// there is nothing to confirm — including when the stored settings were not applied.
    pub confirmable: Option<i64>,
    /// Whether the stored layer made it into `config`.
    pub applied: bool,
    /// Why the stored layer was skipped, for the caller to log.
    pub error: Option<String>,
}

/// Re-resolve with `stored` in place, and decide whether `generation` may be confirmed.
///
/// A stored configuration that does not resolve is skipped rather than fatal: it would take the
/// service down over a row in a table the service itself is the only supported way to edit.
///
/// The subtle part is what that does to the generation. Serving proves nothing about settings
/// that were never applied, so a skipped layer means the generation must *not* be confirmed —
/// otherwise a configuration that does not resolve gets recorded as the known-good one, and
/// since a rollback restores the newest confirmed snapshot, the broken settings become the
/// thing every later rollback restores to. A safety mechanism that poisons itself.
pub fn apply_stored(
    cli: &Cli,
    config: Config,
    sources: Sources,
    stored: &Layer,
    generation: Option<i64>,
) -> Booted {
    if stored.is_empty() {
        return Booted {
            config,
            sources,
            confirmable: generation,
            applied: true,
            error: None,
        };
    }

    match resolve_with_stored(cli, stored.clone()) {
        Ok((config, sources)) => Booted {
            config,
            sources,
            confirmable: generation,
            applied: true,
            error: None,
        },
        Err(e) => Booted {
            config,
            sources,
            confirmable: None,
            applied: false,
            error: Some(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> Cli {
        Cli {
            no_env_file: true,
            // Every secret has to come from somewhere, and the process environment is not
            // available to tests, so they arrive as command-line values.
            values: [
                ("API_KEY_SALT", "test-salt-minimum-16chars"),
                ("MASTER_API_KEY", "test-master-key-16chars"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
            ..Cli::default()
        }
    }

    fn layer(pairs: &[(&str, &str)]) -> Layer {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_an_empty_store_leaves_the_first_pass_alone() {
        let booted = apply_stored(
            &cli(),
            Config::for_test(),
            Sources::default(),
            &Layer::new(),
            Some(7),
        );

        assert!(booted.applied);
        assert_eq!(booted.confirmable, Some(7));
        assert_eq!(booted.config.captcha_compression, 40);
    }

    #[test]
    fn test_stored_settings_are_applied_and_reported_as_stored() {
        let booted = apply_stored(
            &cli(),
            Config::for_test(),
            Sources::default(),
            &layer(&[("CAPTCHA_COMPRESSION", "66"), ("SERVER_PORT", "4321")]),
            Some(7),
        );

        assert!(booted.applied);
        assert_eq!(booted.confirmable, Some(7));
        assert_eq!(booted.config.captcha_compression, 66);
        // A boot field: at this point the listener is unbound, so the store still decides.
        assert_eq!(booted.config.server_port, 4321);
        assert_eq!(
            booted.sources.get("captcha_compression"),
            crate::config::Source::Stored
        );
    }

    #[test]
    fn test_an_unresolvable_store_is_skipped_and_never_confirmed() {
        // The bug this function exists to make testable. Skipping the layer keeps the service
        // up; confirming the generation anyway would record a configuration that does not
        // resolve as the one every later rollback restores to.
        let booted = apply_stored(
            &cli(),
            Config::for_test(),
            Sources::default(),
            &layer(&[("SERVER_PORT", "99999")]),
            Some(7),
        );

        assert!(!booted.applied);
        assert_eq!(
            booted.confirmable, None,
            "a generation whose settings were skipped must not be confirmed"
        );
        assert_eq!(booted.config.server_port, 3000, "first pass survives");
        assert!(booted.error.unwrap().contains("SERVER_PORT"));
    }

    #[test]
    fn test_nothing_to_confirm_stays_nothing_to_confirm() {
        let booted = apply_stored(
            &cli(),
            Config::for_test(),
            Sources::default(),
            &layer(&[("CAPTCHA_COMPRESSION", "66")]),
            None,
        );

        assert!(booted.applied);
        assert_eq!(booted.confirmable, None);
    }
}
