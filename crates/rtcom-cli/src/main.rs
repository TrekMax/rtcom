//! `rtcom` command-line entry point.
//!
//! v0.2 wiring (post-task-16):
//!
//! 1. Parse CLI args + initialise tracing.
//! 2. Resolve profile path and load it (missing = defaults, malformed
//!    = warn + defaults — profile never blocks startup).
//! 3. Optionally persist the effective config via `--save`.
//! 4. Acquire a UUCP lock for the device path (Unix only; no-op on
//!    Windows).
//! 5. Build a tokio runtime and run `async_main`, which opens the
//!    device, constructs a [`Session`], seeds a [`rtcom_tui::TuiApp`],
//!    and drives [`rtcom_tui::run()`] until the user quits or a signal
//!    trips the cancellation token.
//!
//! `UucpLock` is an RAII handle bound to the synchronous `main` so its
//! `Drop` fires after the runtime block returns — even on
//! signal-driven shutdown. Raw mode / alt screen are owned by
//! [`rtcom_tui::run()`] and restore on every exit path it takes.
//!
//! Dialog-level actions (apply-live, save-profile, …) are *logged*
//! in this task and wired into [`Session`] / profile IO by follow-up
//! tasks T17 and T18.

#![forbid(unsafe_code)]

mod args;
mod signal;

use std::io;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use rtcom_config::{ModalStyle, Profile};
use rtcom_core::{
    LineEnding, LineEndingConfig, LineEndingMapper, ModemLineSnapshot, SerialDevice,
    SerialPortDevice, Session, UucpLock,
};
use rtcom_tui::{summarise, TuiApp};
use tracing_subscriber::EnvFilter;

use crate::args::Cli;
use crate::signal::SignalListener;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    // Resolve the profile path: CLI `-c PATH` wins; otherwise fall back
    // to the XDG default. `None` means no home dir was discoverable —
    // profile-save then becomes a hard error.
    let profile_path = cli
        .config
        .clone()
        .or_else(rtcom_config::default_profile_path);

    // Load the profile with "missing file is fine, malformed is a warn".
    // Hard-failing on an unreadable profile would turn a rarely-touched
    // TOML quirk into a total outage — users on CI with no `$HOME` or
    // typos in their profile should still be able to `rtcom /dev/ttyUSB0`.
    let profile = load_profile(profile_path.as_deref(), cli.quiet);

    let serial_cfg = cli.to_serial_config(&profile);

    if cli.save {
        let Some(path) = profile_path.as_ref() else {
            eprintln!(
                "rtcom: --save requested but no profile path is available \
                 (pass `-c PATH` or set HOME/XDG_CONFIG_HOME)"
            );
            return ExitCode::from(1);
        };
        // Clone so we keep the loaded `profile` around for the session
        // path (`cli.resolved_omap(&profile)` etc.). Only the serial
        // section gets overwritten on --save; other sections pass
        // through unchanged (line_endings, modem, screen will become
        // menu-editable in a later task).
        let updated = Profile {
            serial: serial_config_to_section(&serial_cfg),
            line_endings: profile.line_endings.clone(),
            modem: profile.modem.clone(),
            screen: profile.screen.clone(),
        };
        if let Err(err) = rtcom_config::write(path, &updated) {
            eprintln!("rtcom: --save failed: {err}");
            return ExitCode::from(1);
        }
        if !cli.quiet {
            eprintln!("rtcom: saved profile to {}", path.display());
        }
    }

    let lock = match UucpLock::acquire(&cli.device) {
        Ok(lock) => lock,
        Err(err) => {
            eprintln!("rtcom: {err}");
            return ExitCode::from(1);
        }
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

    let exit_code = runtime.block_on(async_main(cli, profile, serial_cfg));

    drop(lock);

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    ExitCode::from(exit_code as u8)
}

async fn async_main(cli: Cli, profile: Profile, serial_cfg: rtcom_core::SerialConfig) -> i32 {
    let mut device = match SerialPortDevice::open(&cli.device, serial_cfg) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("rtcom: open {} failed: {err}", cli.device);
            return 1;
        }
    };

    // Apply --lower-dtr / --raise-dtr / --lower-rts / --raise-rts
    // immediately after open and before Session takes ownership of
    // the device. Picocom's "do not reset the MCU when I open the
    // port" recipe is `--lower-dtr --lower-rts`, which only works if
    // the deassert happens here — once Session.run starts, the
    // device is moved into the loop and the only way back is via a
    // ToggleDtr / ToggleRts command.
    if let Err(err) = apply_initial_lines(&mut device, &cli) {
        eprintln!("rtcom: failed to set initial DTR/RTS state: {err}");
        return 1;
    }
    let initial_dtr = !cli.lower_dtr;
    let initial_rts = !cli.lower_rts;

    let line_endings = resolved_line_endings(&cli, &profile);

    let session = Session::new(device)
        .with_omap(LineEndingMapper::new(line_endings.omap))
        .with_imap(LineEndingMapper::new(line_endings.imap))
        .with_initial_dtr(initial_dtr)
        .with_initial_rts(initial_rts);

    let bus = session.bus().clone();
    let cancel = session.cancellation_token();

    // Pre-subscribe BEFORE spawning the session so the TUI sees
    // every event published from this point on (broadcast channels
    // do not replay history).
    let tui_rx = bus.subscribe();

    let listener = match SignalListener::install(cancel.clone()) {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(%err, "failed to install signal handlers");
            return 1;
        }
    };

    // Seed the TUI state with everything the runner already knows —
    // the device path + config summary, the live SerialConfig, the
    // resolved line endings, the intended modem snapshot (based on
    // --lower/--raise-dtr/-rts intent; Session has no query API), and
    // the modal render style from the profile.
    let mut app = TuiApp::new(bus.clone());
    app.set_device_summary(cli.device.clone(), summarise(&serial_cfg));
    app.set_serial_config(serial_cfg);
    app.set_line_endings(line_endings);
    app.set_modem_lines(ModemLineSnapshot {
        dtr: initial_dtr,
        rts: initial_rts,
    });
    app.set_modal_style(profile_modal_style(&profile));

    // Spawn the session loop. A clone of the cancel token stays here
    // so the TUI can trip it from a Dispatch::Quit.
    let session_handle = tokio::spawn(session.run());

    // Drive the TUI until the user quits, a signal cancels, or the
    // session's bus closes. On any of these, the TUI returns and the
    // shutdown path below unwinds the spawned tasks.
    let tui_result = rtcom_tui::run(app, bus, tui_rx, cancel.clone()).await;

    // Cancel either a running session (TUI returned first because
    // the user quit) or nothing (session already returned, tripping
    // cancel itself). Either way the session task is done after this
    // await.
    cancel.cancel();
    match session_handle.await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            tracing::error!(error = %err, "session returned error");
        }
        Err(err) => {
            tracing::error!(error = %err, "session task panicked");
        }
    }

    if let Err(err) = tui_result {
        tracing::error!(error = %err, "tui exited with error");
        return 1;
    }

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

/// Applies `--lower-dtr` / `--raise-dtr` / `--lower-rts` / `--raise-rts`
/// to the freshly-opened device. Each flag pair is mutually exclusive
/// at the clap level, so the precedence here (lower-then-raise) only
/// matters as a tiebreaker that can never trigger.
fn apply_initial_lines(device: &mut SerialPortDevice, cli: &Cli) -> Result<(), rtcom_core::Error> {
    if cli.lower_dtr {
        device.set_dtr(false)?;
    } else if cli.raise_dtr {
        device.set_dtr(true)?;
    }
    if cli.lower_rts {
        device.set_rts(false)?;
    } else if cli.raise_rts {
        device.set_rts(true)?;
    }
    Ok(())
}

/// Reads the profile at `path`, or returns the built-in default.
///
/// Missing file → silently use defaults (typical fresh install).
/// Malformed TOML or I/O error → warn to stderr (unless quiet) and
/// still continue with defaults. Profile load must never be the
/// reason rtcom refuses to start.
fn load_profile(path: Option<&Path>, quiet: bool) -> Profile {
    let Some(path) = path else {
        return Profile::default();
    };
    if !path.exists() {
        return Profile::default();
    }
    match rtcom_config::read(path) {
        Ok(p) => p,
        Err(err) => {
            if !quiet {
                eprintln!(
                    "rtcom: profile at {} unreadable ({err}); using defaults",
                    path.display()
                );
            }
            Profile::default()
        }
    }
}

/// Projects a runtime [`rtcom_core::SerialConfig`] back into the TOML-facing
/// [`rtcom_config::profile::SerialSection`] used for `--save`. The
/// inverse of the CLI's merge step: runtime enums → stable strings.
fn serial_config_to_section(
    cfg: &rtcom_core::SerialConfig,
) -> rtcom_config::profile::SerialSection {
    rtcom_config::profile::SerialSection {
        baud: cfg.baud_rate,
        data_bits: cfg.data_bits.bits(),
        stop_bits: stop_bits_number(cfg.stop_bits),
        parity: parity_word(cfg.parity).into(),
        flow: flow_word(cfg.flow_control).into(),
    }
}

const fn parity_word(p: rtcom_core::Parity) -> &'static str {
    match p {
        rtcom_core::Parity::None => "none",
        rtcom_core::Parity::Even => "even",
        rtcom_core::Parity::Odd => "odd",
        rtcom_core::Parity::Mark => "mark",
        rtcom_core::Parity::Space => "space",
    }
}

const fn flow_word(f: rtcom_core::FlowControl) -> &'static str {
    match f {
        rtcom_core::FlowControl::None => "none",
        rtcom_core::FlowControl::Hardware => "hw",
        rtcom_core::FlowControl::Software => "sw",
    }
}

const fn stop_bits_number(s: rtcom_core::StopBits) -> u8 {
    match s {
        rtcom_core::StopBits::One => 1,
        rtcom_core::StopBits::Two => 2,
    }
}

/// Resolve the CLI + profile line-ending mappers into a
/// [`LineEndingConfig`] suitable for seeding [`TuiApp::set_line_endings`].
fn resolved_line_endings(cli: &Cli, profile: &Profile) -> LineEndingConfig {
    LineEndingConfig {
        omap: cli.resolved_omap(profile),
        imap: cli.resolved_imap(profile),
        // The echo-map path has no runtime wire-up in the session loop
        // yet (it lands alongside the local-echo feature), but the TUI
        // snapshot is what the Line endings dialog opens with — pull
        // directly from the profile so the dialog reflects what
        // `--save` persists.
        emap: line_ending_from_profile_str(&profile.line_endings.emap),
    }
}

/// Duplicate of the `line_ending_from_profile` helper in `args.rs`.
/// Kept small and local here; sharing would require promoting the
/// helper to a `pub(crate) use`, which is not worth the churn for a
/// six-line function.
fn line_ending_from_profile_str(s: &str) -> LineEnding {
    match s.to_ascii_lowercase().as_str() {
        "crlf" => LineEnding::AddCrToLf,
        "lfcr" => LineEnding::AddLfToCr,
        "igncr" => LineEnding::DropCr,
        "ignlf" => LineEnding::DropLf,
        _ => LineEnding::None,
    }
}

/// Pulls the modal style from the loaded profile's screen section.
/// Kept as its own function so call sites read top-to-bottom without
/// the nested field access hiding what's actually being fetched.
const fn profile_modal_style(profile: &Profile) -> ModalStyle {
    profile.screen.modal_style
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtcom_core::{DataBits, FlowControl, Parity as CoreParity, SerialConfig, StopBits};

    #[test]
    fn serial_config_to_section_round_trips_parity_and_flow() {
        let cfg = SerialConfig {
            baud_rate: 9600,
            data_bits: DataBits::Seven,
            stop_bits: StopBits::Two,
            parity: CoreParity::Even,
            flow_control: FlowControl::Hardware,
            ..SerialConfig::default()
        };
        let section = serial_config_to_section(&cfg);
        assert_eq!(section.baud, 9600);
        assert_eq!(section.data_bits, 7);
        assert_eq!(section.stop_bits, 2);
        assert_eq!(section.parity, "even");
        assert_eq!(section.flow, "hw");
    }

    #[test]
    fn line_ending_from_profile_str_parses_all_known_forms() {
        assert_eq!(line_ending_from_profile_str("crlf"), LineEnding::AddCrToLf);
        assert_eq!(line_ending_from_profile_str("lfcr"), LineEnding::AddLfToCr);
        assert_eq!(line_ending_from_profile_str("igncr"), LineEnding::DropCr);
        assert_eq!(line_ending_from_profile_str("ignlf"), LineEnding::DropLf);
        assert_eq!(line_ending_from_profile_str("none"), LineEnding::None);
        assert_eq!(line_ending_from_profile_str("bogus"), LineEnding::None);
    }

    #[test]
    fn profile_modal_style_picks_up_screen_section() {
        let mut profile = Profile::default();
        profile.screen.modal_style = ModalStyle::Fullscreen;
        assert_eq!(profile_modal_style(&profile), ModalStyle::Fullscreen);
    }

    #[test]
    fn resolved_line_endings_cli_overrides_profile() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--omap", "crlf", "--imap", "igncr"]);
        let mut profile = Profile::default();
        profile.line_endings.emap = "lfcr".into();
        let le = resolved_line_endings(&cli, &profile);
        assert_eq!(le.omap, LineEnding::AddCrToLf);
        assert_eq!(le.imap, LineEnding::DropCr);
        // emap is sourced directly from profile.
        assert_eq!(le.emap, LineEnding::AddLfToCr);
    }
}
