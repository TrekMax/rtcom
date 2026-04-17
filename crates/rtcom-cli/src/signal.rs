//! Signal handling and exit-code computation.
//!
//! `rtcom` translates the usual termination signals into a tripped
//! [`CancellationToken`] rather than calling `process::exit`. That keeps
//! every `Drop` along the stack (`RawModeGuard`, `UucpLock`, the
//! `Session` task) intact, which is the whole reason CLAUDE.md §3
//! mandates RAII ownership in the first place.
//!
//! Exit-code convention follows the POSIX shell tradition:
//!
//! - normal completion -> `0`
//! - any error -> `1` (set by `main`)
//! - killed by signal `N` -> `128 + N` (e.g. SIGINT -> 130)
//!
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

/// `kill -INT` (interrupt). Sent by Ctrl-C in cooked mode and by
/// `kill -2 <pid>`.
pub const SIGINT_NUM: i32 = 2;
/// `kill -HUP` (hang-up). Sent when the controlling terminal closes.
/// Unix-only; allowed dead on Windows where the listener does not
/// register a SIGHUP equivalent.
#[allow(
    dead_code,
    reason = "consumed by spawn_unix_listener; tests use it cross-platform"
)]
pub const SIGHUP_NUM: i32 = 1;
/// `kill -TERM` (default `kill`).
pub const SIGTERM_NUM: i32 = 15;

/// Maps an optional signal number to a process exit code.
///
/// `None` means the program completed without being signalled and
/// therefore exits 0 (or 1 on error — that lives in `main`, not here).
#[must_use]
pub const fn exit_code_from_signal(signum: Option<i32>) -> i32 {
    match signum {
        None => 0,
        Some(s) => 128 + s,
    }
}

/// Diagnostic name for a signal number. Returns `"unknown"` for signals
/// the listener does not handle. Used by the Unix listener tasks; the
/// Windows path uses string literals directly.
#[allow(
    dead_code,
    reason = "called by spawn_unix_listener; tests use it cross-platform"
)]
#[must_use]
pub const fn signal_name(signum: i32) -> &'static str {
    match signum {
        SIGHUP_NUM => "SIGHUP",
        SIGINT_NUM => "SIGINT",
        SIGTERM_NUM => "SIGTERM",
        _ => "unknown",
    }
}

/// Holds the atomic that records which signal (if any) tripped the
/// cancellation token, so `main` can pick the right exit code.
pub struct SignalListener {
    received: Arc<AtomicI32>,
}

impl SignalListener {
    /// Installs handlers for SIGINT / SIGTERM / SIGHUP (Unix) or
    /// Ctrl-C / Ctrl-Break (Windows). The first signal to arrive trips
    /// `cancel`; subsequent signals are recorded but do not retrigger.
    ///
    /// Must be called from inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`](std::io::Error) if any
    /// per-signal subscription fails (typically because the runtime is
    /// not multi-thread or the OS rejected the registration).
    pub fn install(cancel: CancellationToken) -> std::io::Result<Self> {
        let received = Arc::new(AtomicI32::new(0));

        #[cfg(unix)]
        install_unix(&received, &cancel)?;
        #[cfg(not(unix))]
        install_windows(&received, &cancel)?;

        // The token is consumed (and cloned into the listener tasks)
        // even when no signals fire. Drop it explicitly so the
        // signature stays "pass by value" — clearer at the call site
        // than handing in a borrow that we then have to clone anyway.
        drop(cancel);

        Ok(Self { received })
    }

    /// Returns the signum of the first signal that arrived, or `None`
    /// if none did.
    #[must_use]
    pub fn received(&self) -> Option<i32> {
        let n = self.received.load(Ordering::SeqCst);
        if n == 0 {
            None
        } else {
            Some(n)
        }
    }

    /// Convenience: the exit code `main` should hand to
    /// `process::exit`.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        exit_code_from_signal(self.received())
    }
}

#[cfg(unix)]
fn install_unix(received: &Arc<AtomicI32>, cancel: &CancellationToken) -> std::io::Result<()> {
    use tokio::signal::unix::SignalKind;

    spawn_unix_listener(SignalKind::interrupt(), SIGINT_NUM, received, cancel)?;
    spawn_unix_listener(SignalKind::terminate(), SIGTERM_NUM, received, cancel)?;
    spawn_unix_listener(SignalKind::hangup(), SIGHUP_NUM, received, cancel)?;
    spawn_winch_logger()?;
    Ok(())
}

#[cfg(unix)]
fn spawn_unix_listener(
    kind: tokio::signal::unix::SignalKind,
    signum: i32,
    received: &Arc<AtomicI32>,
    cancel: &CancellationToken,
) -> std::io::Result<()> {
    let mut sig = tokio::signal::unix::signal(kind)?;
    let received = Arc::clone(received);
    let cancel = cancel.clone();
    tokio::spawn(async move {
        // Loop so subsequent signals are still observed (e.g. user
        // hits Ctrl-C twice — the second one is just logged here, but
        // we never want to leave a dangling listener).
        loop {
            if sig.recv().await.is_none() {
                // Stream closed: shutting down, nothing more to do.
                break;
            }
            // First signal wins the "what killed us" race.
            let _ = received.compare_exchange(0, signum, Ordering::SeqCst, Ordering::SeqCst);
            tracing::info!(signal = signal_name(signum), "received signal");
            cancel.cancel();
        }
    });
    Ok(())
}

#[cfg(unix)]
fn spawn_winch_logger() -> std::io::Result<()> {
    let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
    tokio::spawn(async move {
        while sig.recv().await.is_some() {
            // v0.1 only logs; v1.0+'s TUI will react.
            tracing::debug!("SIGWINCH (terminal resize)");
        }
    });
    Ok(())
}

#[cfg(not(unix))]
fn install_windows(received: &Arc<AtomicI32>, cancel: &CancellationToken) -> std::io::Result<()> {
    use tokio::signal::windows::{ctrl_break, ctrl_c};

    let mut intr = ctrl_c()?;
    {
        let received = Arc::clone(received);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            while intr.recv().await.is_some() {
                let _ =
                    received.compare_exchange(0, SIGINT_NUM, Ordering::SeqCst, Ordering::SeqCst);
                tracing::info!(signal = "Ctrl-C", "received signal");
                cancel.cancel();
            }
        });
    }

    let mut brk = ctrl_break()?;
    {
        let received = Arc::clone(received);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            while brk.recv().await.is_some() {
                let _ =
                    received.compare_exchange(0, SIGTERM_NUM, Ordering::SeqCst, Ordering::SeqCst);
                tracing::info!(signal = "Ctrl-Break", "received signal");
                cancel.cancel();
            }
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn no_signal_means_exit_code_zero() {
        assert_eq!(exit_code_from_signal(None), 0);
    }

    #[test]
    fn sigint_produces_exit_code_130() {
        assert_eq!(exit_code_from_signal(Some(SIGINT_NUM)), 130);
    }

    #[test]
    fn sigterm_produces_exit_code_143() {
        assert_eq!(exit_code_from_signal(Some(SIGTERM_NUM)), 143);
    }

    #[test]
    fn sighup_produces_exit_code_129() {
        assert_eq!(exit_code_from_signal(Some(SIGHUP_NUM)), 129);
    }

    #[test]
    fn signum_constants_match_posix() {
        assert_eq!(SIGHUP_NUM, 1);
        assert_eq!(SIGINT_NUM, 2);
        assert_eq!(SIGTERM_NUM, 15);
    }

    #[test]
    fn signal_name_covers_handled_signals() {
        assert_eq!(signal_name(SIGINT_NUM), "SIGINT");
        assert_eq!(signal_name(SIGTERM_NUM), "SIGTERM");
        assert_eq!(signal_name(SIGHUP_NUM), "SIGHUP");
        assert_eq!(signal_name(99), "unknown");
    }

    /// install() must succeed inside a tokio runtime and return a
    /// listener with no signal yet recorded. Drives the green commit.
    #[tokio::test]
    async fn install_returns_listener_with_no_signal_received() {
        let cancel = CancellationToken::new();
        let listener = SignalListener::install(cancel).expect("install");
        assert_eq!(listener.received(), None);
        assert_eq!(listener.exit_code(), 0);
    }

    /// `received()` and `exit_code()` are observable without needing
    /// the `install()` side effect — manually populate the atomic and
    /// verify the projection.
    #[test]
    fn received_and_exit_code_project_from_atomic() {
        let received = Arc::new(AtomicI32::new(0));
        let listener = SignalListener {
            received: received.clone(),
        };
        assert_eq!(listener.received(), None);
        assert_eq!(listener.exit_code(), 0);

        received.store(SIGTERM_NUM, Ordering::SeqCst);
        assert_eq!(listener.received(), Some(SIGTERM_NUM));
        assert_eq!(listener.exit_code(), 143);
    }
}
