//! Terminal raw-mode guard.
//!
//! `tio` and `picocom` both leave the terminal wrecked if they crash
//! mid-session — every embedded engineer has typed `reset` blindly at
//! least once. [`RawModeGuard`] makes a best-effort promise to restore
//! `termios` on every exit path:
//!
//! - normal `Drop` (main returns, `?` early return, scope exit, ...);
//! - unwinding panic (a panic hook is chained in front of the existing one).
//!
//! External signals (`SIGINT`, `SIGTERM`, `SIGHUP`) are *not* handled
//! here — see [`crate::signal::SignalListener`], which trips the
//! session's [`CancellationToken`](tokio_util::sync::CancellationToken)
//! so `main` returns normally and `Drop` fires through the same code
//! path as a clean shutdown.
//!
//! In raw mode the kernel no longer translates `Ctrl-C` to `SIGINT` —
//! that key arrives as a regular byte (`0x03`) the application must
//! handle.
//!
//! Calling [`RawModeGuard::install`] more than once per process is
//! supported: only the first call wires up the panic hook; subsequent
//! calls just toggle raw mode.

use std::marker::PhantomData;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

/// `true` when raw mode is currently active *because of us*.
///
/// Written by the guard's `Drop` and by the panic hook.
static RAW_MODE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Ensures the panic hook is installed exactly once per process.
static HOOKS: Once = Once::new();

/// RAII handle that holds the terminal in raw mode for its lifetime.
///
/// The guard is `!Send` and `!Sync` on purpose — keeping it bound to
/// the thread that owns `main` makes the cleanup story easier to reason
/// about and rules out a class of multi-thread mistakes.
///
/// # Example
///
/// ```ignore
/// let _guard = RawModeGuard::install()?;
/// // ... raw-mode session ...
/// // Drop fires here; termios is restored even if we early-returned.
/// ```
pub struct RawModeGuard {
    _not_send: PhantomData<*const ()>,
}

impl RawModeGuard {
    /// Enables raw mode and installs the panic hook on first call.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`](std::io::Error) if the
    /// terminal cannot be reconfigured — most commonly because stdin is
    /// not a TTY (piped input, redirected file, CI environment without
    /// PTY).
    pub fn install() -> std::io::Result<Self> {
        HOOKS.call_once(install_panic_hook);
        enable_raw_mode()?;
        RAW_MODE_ACTIVE.store(true, Ordering::SeqCst);
        Ok(Self {
            _not_send: PhantomData,
        })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        restore_if_active();
    }
}

/// Disables raw mode if it is currently active (idempotent).
///
/// Errors from [`disable_raw_mode`] are deliberately swallowed: the
/// guard is a best-effort cleanup, and there is nothing useful to do if
/// the OS refuses to restore termios at process exit.
fn restore_if_active() {
    if RAW_MODE_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = disable_raw_mode();
    }
}

/// Chains a panic hook in front of the existing one so termios is
/// restored before the panic message lands.
fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_if_active();
        // Ensure the panic message starts on column 0 — a raw-mode
        // session may have left the cursor mid-line.
        eprint!("\r\n");
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_when_inactive_is_a_noop() {
        // Calling restore without an active guard must not panic, and
        // must leave the flag unset.
        restore_if_active();
        assert!(!RAW_MODE_ACTIVE.load(Ordering::SeqCst));
    }
}
