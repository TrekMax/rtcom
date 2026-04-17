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
//! Stub: pure helpers and the public type are in place; the stateful
//! listener body lands in the next commit.

// All public items in this module are exercised by tests in this commit
// and by main.rs in the follow-up wiring commit; the bin build has no
// callers yet.
#![allow(
    dead_code,
    reason = "main.rs wiring lands in a follow-up commit; tests already cover these"
)]

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

/// `kill -INT` (interrupt). Sent by Ctrl-C in cooked mode and by
/// `kill -2 <pid>`.
pub const SIGINT_NUM: i32 = 2;
/// `kill -HUP` (hang-up). Sent when the controlling terminal closes.
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
/// the listener does not handle.
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
    pub fn install(_cancel: CancellationToken) -> std::io::Result<Self> {
        todo!("SignalListener::install — implementation lands in the next commit")
    }

    /// Returns the signum of the first signal that arrived, or `None`
    /// if none did.
    #[must_use]
    pub fn received(&self) -> Option<i32> {
        let n = self.received.load(std::sync::atomic::Ordering::SeqCst);
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
