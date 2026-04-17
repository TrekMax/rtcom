//! `rtcom` command-line entry point.
//!
//! v0.1 wiring (post-Issue #11):
//!
//! 1. Parse CLI args + initialise tracing.
//! 2. Acquire a UUCP lock for the device path (Unix only; no-op on
//!    Windows).
//! 3. Enable raw mode if stdin is a TTY (skip otherwise — pipes and
//!    CI shells need byte-mode reads for `run_stdin_reader`).
//! 4. Build a tokio runtime and:
//!    - install [`signal::SignalListener`] against the session's
//!      cancellation token;
//!    - open the device, build a [`Session`] with omap/imap from the
//!      CLI, spawn the run loop;
//!    - spawn [`stdin::run_stdin_reader`] feeding the session's bus;
//!    - spawn [`terminal::run_terminal_renderer`] writing the bus
//!      back to stdout;
//!    - await all three tasks.
//! 5. Return `SignalListener::exit_code()` as the process exit code.
//!
//! `RawModeGuard` and `UucpLock` are RAII handles bound to the
//! synchronous `main` so their `Drop` fires after the runtime block
//! returns — even on signal-driven shutdown.

#![forbid(unsafe_code)]

mod args;
mod signal;
mod stdin;
mod terminal;
mod tty;

use std::io::{self, IsTerminal};
use std::process::ExitCode;

use clap::Parser;
use rtcom_core::{LineEndingMapper, SerialPortDevice, Session, UucpLock};
use tracing_subscriber::EnvFilter;

use crate::args::Cli;
use crate::signal::SignalListener;
use crate::stdin::run_stdin_reader;
use crate::terminal::run_terminal_renderer;
use crate::tty::RawModeGuard;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    if !cli.quiet {
        print_config_summary(&cli);
        if io::stdin().is_terminal() {
            // Raw mode swallows Ctrl-C (it is forwarded to the wire as
            // a regular 0x03 byte, matching picocom/tio). Users who
            // don't know the command-key convention will spam Ctrl-C
            // and conclude rtcom is wedged — print the actual way out.
            eprintln!(
                "rtcom: press {} q to quit (Ctrl-C is sent to the device in raw mode)",
                format_escape_key(cli.escape),
            );
        }
    }

    let lock = match UucpLock::acquire(&cli.device) {
        Ok(lock) => lock,
        Err(err) => {
            eprintln!("rtcom: {err}");
            return ExitCode::from(1);
        }
    };

    // Only enter raw mode if we're hooked to a real terminal. Piped
    // stdin (CI, scripts, the e2e tests) is read byte-by-byte via
    // tokio::io::stdin, no termios changes needed.
    let raw_guard = if io::stdin().is_terminal() {
        match RawModeGuard::install() {
            Ok(g) => Some(g),
            Err(err) => {
                tracing::warn!(%err, "could not enable raw mode; continuing without it");
                None
            }
        }
    } else {
        tracing::info!("stdin is not a TTY — skipping raw mode");
        None
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("rtcom: failed to start tokio runtime: {err}");
            return ExitCode::from(1);
        }
    };

    let exit_code = runtime.block_on(async_main(cli));

    // Explicit drops to make the cleanup order obvious: termios first,
    // then the lock file. Both are Drop-safe so falling out of scope
    // would do the same thing — kept for documentation value.
    drop(raw_guard);
    drop(lock);

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    ExitCode::from(exit_code as u8)
}

async fn async_main(cli: Cli) -> i32 {
    let device = match SerialPortDevice::open(&cli.device, cli.to_serial_config()) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("rtcom: open {} failed: {err}", cli.device);
            return 1;
        }
    };

    let session = Session::new(device)
        .with_omap(LineEndingMapper::new(cli.omap.into()))
        .with_imap(LineEndingMapper::new(cli.imap.into()));

    let bus = session.bus().clone();
    let cancel = session.cancellation_token();

    // Pre-subscribe BEFORE spawning the renderer so it sees every
    // event published from this point on (broadcast channels do not
    // replay history).
    let renderer_rx = bus.subscribe();

    let listener = match SignalListener::install(cancel.clone()) {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(%err, "failed to install signal handlers");
            return 1;
        }
    };

    // Keep a clone of the cancel token so main can trip it *after*
    // session.run returns. Without this, a device disconnect ends
    // Session cleanly but leaves stdin (blocked on a read) and the
    // renderer (blocked on recv) running forever — the whole process
    // hangs with no feedback.
    let shutdown = cancel.clone();

    let session_handle = tokio::spawn(session.run());
    let renderer_handle = tokio::spawn(run_terminal_renderer(
        renderer_rx,
        cancel.clone(),
        tokio::io::stdout(),
    ));
    let stdin_handle = tokio::spawn(run_stdin_reader(
        tokio::io::stdin(),
        bus,
        cancel,
        cli.escape,
    ));

    // The session loop terminates on a Quit command, a fatal I/O
    // error (device disconnect), or a signal. We own the "session is
    // done" authority here — trip cancel so stdin / renderer unwind
    // through the same code path regardless of which trigger fired.
    if let Err(err) = session_handle.await {
        tracing::error!(error = %err, "session task panicked");
        shutdown.cancel();
        let _ = renderer_handle.await;
        let _ = stdin_handle.await;
        return 1;
    }
    shutdown.cancel();
    let _ = renderer_handle.await;
    let _ = stdin_handle.await;

    listener.exit_code()
}

fn init_tracing(verbosity: u8) {
    let default_level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}

fn print_config_summary(cli: &Cli) {
    let cfg = cli.to_serial_config();
    eprintln!(
        "rtcom — device: {} | {} {}{}{} | flow: {:?} | no-reset: {} | echo: {} | escape: 0x{:02x} | verbose: {}",
        cli.device,
        cfg.baud_rate,
        cfg.data_bits.bits(),
        parity_letter(cfg.parity),
        stop_bits_number(cfg.stop_bits),
        cfg.flow_control,
        cli.no_reset,
        cli.echo,
        cli.escape,
        cli.verbose,
    );
}

/// Pretty-prints an escape byte in the same caret notation `--escape`
/// accepts: `^T` for 0x14, `'a'` for a printable ASCII character.
fn format_escape_key(b: u8) -> String {
    match b {
        0..=0x1f => format!("^{}", char::from(b + 0x40)),
        0x7f => "^?".into(),
        _ => format!("'{}'", char::from(b)),
    }
}

const fn parity_letter(p: rtcom_core::Parity) -> char {
    match p {
        rtcom_core::Parity::None => 'N',
        rtcom_core::Parity::Even => 'E',
        rtcom_core::Parity::Odd => 'O',
        rtcom_core::Parity::Mark => 'M',
        rtcom_core::Parity::Space => 'S',
    }
}

const fn stop_bits_number(s: rtcom_core::StopBits) -> u8 {
    match s {
        rtcom_core::StopBits::One => 1,
        rtcom_core::StopBits::Two => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::format_escape_key;

    #[test]
    fn format_escape_key_control_char() {
        assert_eq!(format_escape_key(0x14), "^T");
        assert_eq!(format_escape_key(0x01), "^A");
        assert_eq!(format_escape_key(0x00), "^@");
    }

    #[test]
    fn format_escape_key_printable() {
        assert_eq!(format_escape_key(b'a'), "'a'");
        assert_eq!(format_escape_key(b'?'), "'?'");
    }

    #[test]
    fn format_escape_key_del() {
        assert_eq!(format_escape_key(0x7f), "^?");
    }
}
