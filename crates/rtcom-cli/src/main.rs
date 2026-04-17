//! `rtcom` command-line entry point.
//!
//! v0.1 placeholder pipeline:
//!
//! 1. Parse args.
//! 2. Initialise tracing (early — so signal/IO bringup is observable).
//! 3. Build a tokio runtime and an [`async_main`] that
//!    - installs the [`signal::SignalListener`] against a fresh
//!      [`CancellationToken`],
//!    - runs the raw-mode demo loop (Issue #4) until either a key
//!      sequence or the cancel token tells it to stop,
//!    - returns the listener's exit code.
//!
//! The Session/Stdin/Mapper wiring lives in later issues; this file is
//! the smallest thing that exercises every cleanup path end-to-end.

#![forbid(unsafe_code)]

mod args;
mod signal;
mod stdin;
mod tty;

use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::args::Cli;
use crate::signal::SignalListener;
use crate::tty::RawModeGuard;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    if !cli.quiet {
        print_config_summary(&cli);
    }

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

    let exit_code = runtime.block_on(async_main());
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    ExitCode::from(exit_code as u8)
}

/// Async body of `main`. Returns the desired process exit code.
async fn async_main() -> i32 {
    let cancel = CancellationToken::new();

    let listener = match SignalListener::install(cancel.clone()) {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(%err, "failed to install signal handlers");
            return 1;
        }
    };

    if let Err(err) = run_demo(cancel.clone()).await {
        tracing::error!(error = %err, "demo loop failed");
        eprintln!("rtcom: {err:#}");
        return 1;
    }

    listener.exit_code()
}

/// Initialises the global `tracing` subscriber, mapping `-v` count to a
/// default filter level and respecting `RUST_LOG` if set.
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

/// Raw-mode demo loop, runs on a blocking thread so the synchronous
/// `crossterm::event::poll` does not stall the tokio runtime.
async fn run_demo(cancel: CancellationToken) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let _guard =
            RawModeGuard::install().context("failed to enable raw mode (is stdin a TTY?)")?;

        let mut stdout = io::stdout();
        writeln!(
            stdout,
            "rtcom raw-mode demo — q or Ctrl-C to quit, p to panic (cleanup test)\r"
        )?;
        stdout.flush()?;

        loop {
            // Cooperative cancellation between event polls. 200 ms is a
            // tradeoff: short enough that an external SIGTERM exits in
            // well under a second; long enough that the busy-wait cost
            // is negligible.
            if cancel.is_cancelled() {
                break;
            }
            if event::poll(Duration::from_millis(200))? {
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

        writeln!(stdout, "rtcom: bye\r")?;
        stdout.flush()?;
        Ok(())
    })
    .await
    .context("demo task join failed")?
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
