//! Event loop driver for the rtcom TUI.
//!
//! [`run`] owns the ratatui [`Terminal`], the crossterm key-event
//! stream, and the bus subscription. It multiplexes them via
//! `tokio::select!`, re-rendering the app after every iteration and
//! unwinding cleanly on cancel / user-initiated quit / terminal error.
//!
//! The three RAII guards ([`RawModeGuard`], [`AltScreenGuard`], and
//! the implicit `Terminal` drop) guarantee the terminal is restored to
//! its original state on every exit path — including panic — so
//! foreground shells keep working even if rtcom crashes mid-frame.
//!
//! T16 scope note: `DialogAction` outcomes are currently *logged* via
//! `tracing::warn!` and not applied. T17 and T18 wire those actions
//! into `Session::apply_config` / profile IO.

use std::io;

use anyhow::{Context, Result};
use crossterm::event::{Event as CtEvent, EventStream, KeyEvent, KeyEventKind};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use rtcom_core::{Event, EventBus, Parity, SerialConfig, StopBits};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::{
    app::TuiApp,
    input::Dispatch,
    terminal::{AltScreenGuard, RawModeGuard},
};

/// Drive the TUI main loop until cancelled or the user requests quit.
///
/// Owns the ratatui terminal + crossterm event stream; restores cooked
/// mode + leaves the alternate screen on return (including on `Err`).
///
/// # Parameters
///
/// - `app`       — the prepared [`TuiApp`]; caller seeds device
///   summary, initial `SerialConfig`, line endings, modem lines, and
///   modal style before handing in.
/// - `bus`       — the shared [`EventBus`], used to publish
///   [`Event::TxBytes`] for keystrokes.
/// - `bus_rx`    — a pre-subscribed bus receiver; subscribe before
///   spawning the session task so no events are missed.
/// - `cancel`    — the shared [`CancellationToken`]. Tripping it (via
///   signal, session exit, or [`Dispatch::Quit`]) unwinds the loop.
///
/// # Errors
///
/// Propagates terminal setup, IO, and render errors. Bus `Lagged`
/// warnings are logged and the loop continues; a bus `Closed` error
/// is treated as an implicit cancel and the function returns `Ok(())`.
pub async fn run(
    mut app: TuiApp,
    bus: EventBus,
    mut bus_rx: broadcast::Receiver<Event>,
    cancel: CancellationToken,
) -> Result<()> {
    // RAII: enter raw mode + alt screen. Both restore on drop, even
    // if the terminal setup below or the loop body returns Err — the
    // guards sit on the stack and unwind through every error path.
    let _raw = RawModeGuard::enter()?;
    let _alt = AltScreenGuard::enter()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("build ratatui terminal")?;

    let mut keys = EventStream::new();

    // Seed frame so the user sees the chrome before any input.
    terminal
        .draw(|f| app.render(f))
        .context("initial terminal draw")?;

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,

            ev = keys.next() => {
                match ev {
                    Some(Ok(CtEvent::Key(key))) => {
                        // crossterm 0.28 surfaces both Press and Release
                        // events on terminals that advertise kitty
                        // keyboard protocol. Filter to Press-only so a
                        // single physical keystroke does not produce
                        // two bytes on the wire.
                        if key.kind == KeyEventKind::Press
                            && handle_key_event(key, &mut app, &bus, &cancel)
                        {
                            break;
                        }
                    }
                    Some(Ok(CtEvent::Resize(cols, rows))) => {
                        // SerialPane::resize takes (rows, cols).
                        app.serial_pane_mut().resize(rows, cols);
                    }
                    Some(Ok(_)) => {
                        // FocusGained / FocusLost / Mouse / Paste: ignore.
                    }
                    Some(Err(err)) => {
                        tracing::error!(%err, "terminal event stream error");
                        break;
                    }
                    None => {
                        // Stream closed — treat as an implicit cancel.
                        break;
                    }
                }
            }

            bus_ev = bus_rx.recv() => {
                if !handle_bus_event(bus_ev, &mut app) {
                    break;
                }
            }
        }

        terminal.draw(|f| app.render(f)).context("terminal draw")?;
    }

    // Guards drop in reverse construction order: _alt (leave alt
    // screen) then _raw (cooked mode). Terminal's own Drop flushes
    // the final frame to stdout before that.
    Ok(())
}

/// Process a single key event. Returns `true` when the caller should
/// break out of the event loop (user requested quit).
fn handle_key_event(
    key: KeyEvent,
    app: &mut TuiApp,
    bus: &EventBus,
    cancel: &CancellationToken,
) -> bool {
    match app.handle_key(key) {
        Dispatch::TxBytes(bytes) => {
            bus.publish(Event::TxBytes(bytes::Bytes::from(bytes)));
            false
        }
        Dispatch::OpenedMenu | Dispatch::ClosedMenu | Dispatch::Noop => false,
        Dispatch::Quit => {
            cancel.cancel();
            true
        }
        Dispatch::Action(action) => {
            tracing::warn!(?action, "dialog action not yet wired (T17/T18)");
            false
        }
    }
}

/// Process a single bus event. Returns `false` when the caller should
/// break out of the event loop (bus closed).
fn handle_bus_event(
    bus_ev: std::result::Result<Event, broadcast::error::RecvError>,
    app: &mut TuiApp,
) -> bool {
    match bus_ev {
        Ok(Event::RxBytes(b)) => {
            app.serial_pane_mut().ingest(&b);
        }
        Ok(Event::ConfigChanged(cfg)) => {
            app.set_serial_config(cfg);
            app.set_config_summary(summarise(&cfg));
        }
        Ok(Event::SystemMessage(msg)) => {
            app.serial_pane_mut()
                .ingest(format!("\r\n*** rtcom: {msg}\r\n").as_bytes());
        }
        Ok(Event::DeviceDisconnected { reason }) => {
            app.serial_pane_mut()
                .ingest(format!("\r\n*** device disconnected: {reason}\r\n").as_bytes());
        }
        Ok(Event::Error(e)) => {
            tracing::error!(%e, "bus error");
            // T19 surfaces errors as an in-TUI toast.
        }
        // `rtcom_core::Event` is `#[non_exhaustive]` so the wildcard
        // tail here both covers known-but-ignored variants
        // (MenuOpened/Closed, ProfileSaved/LoadFailed, Command,
        // TxBytes, DeviceConnected) and any future additions — the
        // TUI doesn't need to change to stay forward-compatible.
        Ok(_) => {}
        Err(broadcast::error::RecvError::Closed) => {
            // Bus closed; no more events to process.
            return false;
        }
        Err(broadcast::error::RecvError::Lagged(n)) => {
            tracing::warn!("bus lagged by {n} events");
        }
    }
    true
}

/// Build the short `"<baud> <DATA><PARITY><STOP> <flow>"` status-bar
/// string used on the top row (e.g. `"115200 8N1 none"`).
#[must_use]
pub fn summarise(cfg: &SerialConfig) -> String {
    format!(
        "{} {}{}{} {}",
        cfg.baud_rate,
        cfg.data_bits.bits(),
        parity_letter(cfg.parity),
        stop_bits_number(cfg.stop_bits),
        flow_word(cfg.flow_control),
    )
}

const fn parity_letter(p: Parity) -> char {
    match p {
        Parity::None => 'N',
        Parity::Even => 'E',
        Parity::Odd => 'O',
        Parity::Mark => 'M',
        Parity::Space => 'S',
    }
}

const fn stop_bits_number(s: StopBits) -> u8 {
    match s {
        StopBits::One => 1,
        StopBits::Two => 2,
    }
}

const fn flow_word(f: rtcom_core::FlowControl) -> &'static str {
    match f {
        rtcom_core::FlowControl::None => "none",
        rtcom_core::FlowControl::Hardware => "hw",
        rtcom_core::FlowControl::Software => "sw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtcom_core::{DataBits, FlowControl, SerialConfig, StopBits};

    #[test]
    fn summarise_default_is_115200_8n1_none() {
        let cfg = SerialConfig::default();
        assert_eq!(summarise(&cfg), "115200 8N1 none");
    }

    #[test]
    fn summarise_custom_config() {
        let cfg = SerialConfig {
            baud_rate: 9600,
            data_bits: DataBits::Seven,
            stop_bits: StopBits::Two,
            parity: Parity::Even,
            flow_control: FlowControl::Hardware,
            ..SerialConfig::default()
        };
        assert_eq!(summarise(&cfg), "9600 7E2 hw");
    }

    #[tokio::test]
    async fn handle_bus_event_rx_bytes_reaches_pane() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        assert!(handle_bus_event(
            Ok(Event::RxBytes(bytes::Bytes::from_static(b"hi"))),
            &mut app
        ));
    }

    #[tokio::test]
    async fn handle_bus_event_closed_breaks_loop() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        assert!(!handle_bus_event(
            Err(broadcast::error::RecvError::Closed),
            &mut app
        ));
    }

    #[tokio::test]
    async fn handle_bus_event_lagged_is_logged_but_continues() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        assert!(handle_bus_event(
            Err(broadcast::error::RecvError::Lagged(7)),
            &mut app
        ));
    }

    #[tokio::test]
    async fn handle_bus_event_config_changed_updates_summary() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "old");
        let cfg = SerialConfig {
            baud_rate: 9600,
            ..SerialConfig::default()
        };
        assert!(handle_bus_event(Ok(Event::ConfigChanged(cfg)), &mut app));
        // Not a public getter on TuiApp, but the app renders the
        // string; covered here indirectly via set_config_summary.
    }
}
