//! Restarting the server in place, so a stored boot field can take effect.
//!
//! The mechanism is `execve` on this process rather than a supervisor that spawns and kills a
//! child. That keeps one process and one PID, so `docker stop`, Kubernetes, systemd and the PID
//! file all keep working with no extra code — where a parent would inherit PID 1's obligations
//! to reap orphans and forward signals, and getting that wrong means containers that ignore
//! `docker stop` and take the SIGKILL every time.
//!
//! In-flight requests are dropped at the exec boundary. True zero-downtime needs two overlapping
//! processes sharing a listening socket, which is a much larger feature than this.

use std::ffi::OsString;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Shared handle the admin API uses to ask for a restart.
#[derive(Clone, Debug)]
pub struct RestartHandle {
    requested: Arc<AtomicBool>,
    token: CancellationToken,
}

impl RestartHandle {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            token,
        }
    }

    /// Ask the server to shut down gracefully and come back.
    ///
    /// Returns whether this call was the one that requested it, so a second request while the
    /// first is already draining does not read as a fresh one.
    pub fn request(&self) -> bool {
        let first = !self.requested.swap(true, Ordering::SeqCst);
        self.token.cancel();
        first
    }

    /// Whether a restart was asked for, checked once the server has stopped.
    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

/// The arguments this process was started with, captured before anything consumed them.
///
/// Kept whole rather than reconstructed from the parsed `Cli`: the point of a restart is to
/// come back as the same command, and re-rendering flags from a parsed structure is a chance to
/// get that subtly wrong — a dropped `--config`, a secret file re-read from a path that has
/// since changed.
#[derive(Clone, Debug)]
pub struct Argv(Vec<OsString>);

impl Argv {
    /// Capture `std::env::args_os()`, including the binary name.
    pub fn capture() -> Self {
        Self(std::env::args_os().collect())
    }
}

/// Why a restart could not be carried out.
#[derive(Debug)]
pub enum RestartError {
    /// The running executable could not be located, or no longer exists — which is what
    /// `/proc/self/exe` reports after the binary is replaced on disk during an upgrade.
    MissingExecutable(String),
    /// `execv` returned, which it only does on failure.
    ExecFailed(std::io::Error),
}

impl std::fmt::Display for RestartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingExecutable(e) => write!(f, "cannot find the running executable: {e}"),
            Self::ExecFailed(e) => write!(f, "exec failed: {e}"),
        }
    }
}

/// Replace this process with a fresh copy of itself.
///
/// On success this never returns — the process image is gone. The environment carries across
/// automatically, so the environment layer is preserved exactly.
///
/// **What this actually requires**, since it is narrower than it looks: every resource whose
/// state outlives the process must already be flushed and closed — the connection pool, the
/// trace exporter, anything mid-write to disk. `execve` replaces the whole image, so surviving
/// threads are not a hazard the way they are after `fork`: there is no inherited lock to
/// deadlock on, only work that silently never finishes. That is why `main` closes the pool and
/// shuts telemetry down first, and why it is fine for this to run inside `#[tokio::main]` with
/// the runtime's worker threads still parked. Descriptors are the other half, and they are
/// already handled: `std` opens sockets `SOCK_CLOEXEC` and SQLite uses `O_CLOEXEC`, so nothing
/// leaks into the new image.
#[cfg(unix)]
pub fn exec_self(argv: &Argv) -> Result<std::convert::Infallible, RestartError> {
    use std::os::unix::process::CommandExt;

    // On Linux this resolves /proc/self/exe, which yields a path suffixed "(deleted)" once the
    // binary has been replaced on disk. Exiting is recoverable by whatever supervises the
    // service; exec'ing a path that is not there is not.
    let exe =
        std::env::current_exe().map_err(|e| RestartError::MissingExecutable(e.to_string()))?;
    if !exe.exists() {
        return Err(RestartError::MissingExecutable(format!(
            "{} no longer exists (was the binary replaced?)",
            exe.display()
        )));
    }

    let mut command = std::process::Command::new(&exe);
    command.args(argv.0.iter().skip(1));
    Err(RestartError::ExecFailed(command.exec()))
}

#[cfg(not(unix))]
pub fn exec_self(_argv: &Argv) -> Result<std::convert::Infallible, RestartError> {
    Err(RestartError::MissingExecutable(
        "in-place restart is only supported on unix".to_string(),
    ))
}

/// Check that an address can actually be bound, before a restart is allowed to depend on it.
///
/// This is the failure validation cannot see: "port already in use" is exactly what an operator
/// editing `server_port` through a web form will hit, and the only way to find out is to try.
/// Returns `Ok(())` when `addr` matches `current`, since this process already holds that one.
pub fn preflight_bind(addr: SocketAddr, current: SocketAddr) -> Result<(), String> {
    if addr == current {
        return Ok(());
    }
    match std::net::TcpListener::bind(addr) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(e) => Err(format!("cannot bind {addr}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requesting_a_restart_cancels_the_token() {
        let token = CancellationToken::new();
        let handle = RestartHandle::new(token.clone());

        assert!(!handle.requested());
        assert!(!token.is_cancelled());

        assert!(handle.request(), "the first request is the one that counts");

        assert!(handle.requested());
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_a_second_request_does_not_read_as_fresh() {
        let handle = RestartHandle::new(CancellationToken::new());
        assert!(handle.request());
        assert!(
            !handle.request(),
            "already draining, so this is not a new request"
        );
        assert!(handle.requested());
    }

    #[test]
    fn test_binding_the_address_already_held_is_allowed() {
        // The common case: a restart that does not move the listener must not be refused
        // because this very process is holding the port.
        let addr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
        assert!(preflight_bind(addr, addr).is_ok());
    }

    #[test]
    fn test_a_free_address_passes_and_is_released_again() {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let current: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(preflight_bind(addr, current).is_ok());
        // Released, so the real listener can have it after the restart.
        assert!(preflight_bind(addr, current).is_ok());
    }

    #[test]
    fn test_an_address_someone_else_holds_is_refused() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = held.local_addr().unwrap();
        let current: SocketAddr = "127.0.0.1:1".parse().unwrap();

        let err = preflight_bind(addr, current).unwrap_err();

        assert!(err.contains("cannot bind"), "{err}");
        drop(held);
    }
}
