//! Keep the installed log filter in step with the running configuration.
//!
//! `log_level` is a live field, which it can only be because the subscriber was installed
//! behind a reload handle. Something has to push a new value into that handle when the
//! configuration moves; this task is that something. It watches the same `watch` channel the
//! cleanup task uses, so a `PATCH`, a SIGHUP or a stored value all arrive the same way.

use crate::config::ConfigHandle;
use crate::telemetry::LogFilterHandle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Watch the configuration and re-apply the log filter whenever `log_level` changes.
pub fn start_log_filter_task(
    config: ConfigHandle,
    filter: LogFilterHandle,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = config.subscribe();
        // The filter installed at startup already matches this, so the first change is the
        // first thing worth acting on.
        let mut current = rx.borrow().log_level.clone();

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                changed = rx.changed() => {
                    if changed.is_err() {
                        // Every sender is gone, which only happens during shutdown.
                        break;
                    }
                    // Cloned out of the guard immediately: holding a `watch` read guard across
                    // an await blocks every writer.
                    let next = rx.borrow().log_level.clone();
                    if next == current {
                        continue;
                    }
                    match filter.apply(&next) {
                        Ok(()) => {
                            tracing::info!("Log filter changed: {current:?} -> {next:?}");
                            current = next;
                        }
                        // `current` is deliberately left alone, so if the same value arrives
                        // again — say from a reload that re-reads the same bad file — it is
                        // retried rather than assumed applied.
                        Err(e) => tracing::warn!("Keeping the running log filter: {e}"),
                    }
                }
            }
        }
    })
}
