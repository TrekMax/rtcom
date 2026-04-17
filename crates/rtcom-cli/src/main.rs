//! `rtcom` command-line entry point.
//!
//! v0.1 scope (post-Issue #4): parse CLI arguments, print the derived
//! [`rtcom_core::SerialConfig`], then enter a minimal raw-mode demo loop so
//! we can manually verify that [`tty::RawModeGuard`] restores `termios` on
//! every exit path (normal return, panic, signal). Real session wiring
//! (orchestrator, command parsing, mappers) lands in subsequent issues.

#![forbid(unsafe_code)]

mod args;
mod stdin;
mod tty;

use std::io::{self, Write};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};

use crate::args::Cli;
use crate::tty::RawModeGuard;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = cli.to_serial_config();

    if !cli.quiet {
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

    interactive_demo()
}

/// Minimal raw-mode demo for Issue #4.
///
/// Enters raw mode via [`RawModeGuard`], polls keyboard events with
/// `crossterm`, and exits cleanly on `q` or `Ctrl-C`. `p` triggers a panic
/// so the panic-hook recovery path can be eyeballed.
///
/// This is *placeholder* main loop wiring — Issue #5 replaces it with the
/// real session orchestrator.
fn interactive_demo() -> Result<()> {
    let _guard = RawModeGuard::install().context("failed to enable raw mode (is stdin a TTY?)")?;

    let mut stdout = io::stdout();
    write!(
        stdout,
        "rtcom raw-mode demo — q or Ctrl-C to quit, p to panic (cleanup test)\r\n"
    )?;
    stdout.flush()?;

    loop {
        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q') | KeyCode::Esc, _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    (KeyCode::Char('p'), _) => panic!("rtcom: panic-recovery test"),
                    _ => {}
                }
            }
        }
    }

    write!(stdout, "rtcom: bye\r\n")?;
    stdout.flush()?;
    Ok(())
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
