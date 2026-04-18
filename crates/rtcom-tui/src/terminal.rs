//! Terminal mode guards.
//!
//! [`RawModeGuard`] puts the terminal into raw mode on construction and
//! restores cooked mode on drop. [`AltScreenGuard`] enters the alternate
//! screen on construction and leaves it on drop.
//! [`MouseCaptureGuard`] enables mouse-event reporting on construction
//! and disables it on drop. Use in RAII order — drop in the reverse of
//! construction for clean restoration.

use std::io::{self, Stdout, Write};

use anyhow::{Context, Result};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

/// RAII guard that keeps the terminal in raw mode.
#[must_use = "dropping this guard disables raw mode immediately"]
pub struct RawModeGuard {
    _private: (),
}

impl RawModeGuard {
    /// Enable raw mode on the controlling terminal. Returns an error if
    /// the terminal API rejects the call (e.g. not a TTY).
    ///
    /// # Errors
    ///
    /// Propagates the crossterm error from `enable_raw_mode`.
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("enable raw mode")?;
        Ok(Self { _private: () })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

/// RAII guard that keeps the terminal on the alternate screen.
#[must_use = "dropping this guard leaves the alternate screen immediately"]
pub struct AltScreenGuard {
    stdout: Stdout,
}

impl AltScreenGuard {
    /// Enter the alternate screen on stdout.
    ///
    /// # Errors
    ///
    /// Propagates the crossterm error from `EnterAlternateScreen`.
    pub fn enter() -> Result<Self> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("enter alt screen")?;
        Ok(Self { stdout })
    }
}

impl Drop for AltScreenGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, LeaveAlternateScreen);
        let _ = self.stdout.flush();
    }
}

/// RAII guard that enables terminal mouse-event reporting.
///
/// While this guard is alive, mouse click / drag / wheel events arrive
/// via the crossterm event stream as [`crossterm::event::Event::Mouse`].
/// On drop, mouse capture is disabled so the terminal emulator's own
/// selection + copy bindings work again once rtcom exits.
///
/// Note: with capture enabled, the terminal's native text selection
/// typically does *not* work on bare click-drag — most emulators
/// (xterm, gnome-terminal, kitty, alacritty, iterm2) treat `Shift+drag`
/// as a bypass, returning native selection to the user on demand. That
/// is the v0.2 workflow for copying text; a native rtcom selection
/// implementation ships in v0.2.1.
#[must_use = "dropping this guard disables mouse capture immediately"]
pub struct MouseCaptureGuard {
    stdout: Stdout,
}

impl MouseCaptureGuard {
    /// Enable mouse-event reporting on stdout.
    ///
    /// # Errors
    ///
    /// Propagates the crossterm error from `EnableMouseCapture`.
    pub fn enable() -> Result<Self> {
        let mut stdout = io::stdout();
        execute!(stdout, EnableMouseCapture).context("enable mouse capture")?;
        Ok(Self { stdout })
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, DisableMouseCapture);
        let _ = self.stdout.flush();
    }
}
