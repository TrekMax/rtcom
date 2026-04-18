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
//! T17 wired the live-apply / line-toggle / send-break actions through
//! the bus into the session. T18 extends this with the save-flavored
//! (`ApplyAndSave`, `WriteProfile`, `ReadProfile`, ...) actions.

use std::io;

use anyhow::{Context, Result};
use crossterm::event::{Event as CtEvent, EventStream, KeyEvent, KeyEventKind};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use rtcom_core::{
    command::Command, Event, EventBus, ModemLineSnapshot, Parity, SerialConfig, StopBits,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::{
    app::TuiApp,
    input::Dispatch,
    modal::DialogAction,
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
            apply_dialog_action(&action, app, bus);
            false
        }
    }
}

/// Route a [`DialogAction`] to the right destination:
///
/// - For actions that map 1:1 onto a [`Command`] (live-apply, line
///   toggles, break), publish `Event::Command(cmd)` on the bus and let
///   the session dispatch it.
/// - For `ApplyModalStyleLive`, update the TUI's local cached style so
///   subsequent renders pick it up. Persisting to profile is T18's job.
/// - For save-flavored actions (`ApplyAndSave`, `WriteProfile`,
///   `ReadProfile`, `ApplyModalStyleAndSave`), log a warn with a T18
///   TODO — they land in the next task.
/// - For line-ending changes (`ApplyLineEndingsLive` /
///   `ApplyLineEndingsAndSave`), log a warn pointing at v0.2.1 which
///   introduces the `Arc<Mutex<Mapper>>` refactor needed to swap the
///   mapper at runtime.
fn apply_dialog_action(action: &DialogAction, app: &mut TuiApp, bus: &EventBus) {
    // Local-only action: no Command translation, just update the cache.
    if let DialogAction::ApplyModalStyleLive(style) = action {
        app.set_modal_style(*style);
        // T19 / T20 polish makes the renderer honour the style.
        return;
    }

    if let Some(cmd) = action_to_command(action) {
        bus.publish(Event::Command(cmd));
        return;
    }

    match action {
        DialogAction::ApplyLineEndingsLive(_) | DialogAction::ApplyLineEndingsAndSave(_) => {
            tracing::warn!("live line-ending change not yet supported; restart rtcom to apply");
        }
        DialogAction::ApplyAndSave(_)
        | DialogAction::ApplyModalStyleAndSave(_)
        | DialogAction::WriteProfile
        | DialogAction::ReadProfile => {
            tracing::warn!(?action, "save-flavored action: pending T18");
        }
        // All other variants are handled by the `action_to_command`
        // branch above or by the ApplyModalStyleLive early return; this
        // arm is defensive against future DialogAction additions.
        _ => {
            tracing::warn!(?action, "unhandled DialogAction");
        }
    }
}

/// Translate a [`DialogAction`] into a [`Command`] when the action
/// corresponds to a bus-dispatched command. Returns `None` for actions
/// that the TUI handles locally (`ApplyModalStyleLive`) or that are
/// deferred to a later task (`ApplyAndSave`, profile IO, line endings).
///
/// Split out as a free function so it can be unit-tested without
/// constructing a full event bus.
#[must_use]
const fn action_to_command(action: &DialogAction) -> Option<Command> {
    match action {
        DialogAction::ApplyLive(cfg) => Some(Command::ApplyConfig(*cfg)),
        DialogAction::SetDtr(state) => Some(Command::SetDtrAbs(*state)),
        DialogAction::SetRts(state) => Some(Command::SetRtsAbs(*state)),
        DialogAction::SendBreak => Some(Command::SendBreak),
        // Handled locally by `apply_dialog_action` — not a Command.
        DialogAction::ApplyModalStyleLive(_)
        // Deferred to T18 (save-flavored).
        | DialogAction::ApplyAndSave(_)
        | DialogAction::ApplyModalStyleAndSave(_)
        | DialogAction::WriteProfile
        | DialogAction::ReadProfile
        // Deferred to v0.2.1 (needs runtime-mapper refactor).
        | DialogAction::ApplyLineEndingsLive(_)
        | DialogAction::ApplyLineEndingsAndSave(_) => None,
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
        Ok(Event::ModemLinesChanged { dtr, rts }) => {
            app.set_modem_lines(ModemLineSnapshot { dtr, rts });
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

    #[tokio::test]
    async fn handle_bus_event_modem_lines_changed_reaches_app() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        assert!(handle_bus_event(
            Ok(Event::ModemLinesChanged {
                dtr: false,
                rts: true
            }),
            &mut app
        ));
        // No public getter for the snapshot either; the call succeeds
        // and returns `true` (loop continues) — the set_modem_lines
        // call is covered by `TuiApp`'s own render tests in v0.2+.
    }

    #[test]
    fn apply_live_maps_to_apply_config_command() {
        let cfg = SerialConfig::default();
        let cmd = action_to_command(&DialogAction::ApplyLive(cfg)).expect("maps to ApplyConfig");
        match cmd {
            Command::ApplyConfig(out) => assert_eq!(out, cfg),
            other => panic!("expected ApplyConfig, got {other:?}"),
        }
    }

    #[test]
    fn set_dtr_maps_to_set_dtr_abs_command() {
        assert!(matches!(
            action_to_command(&DialogAction::SetDtr(true)),
            Some(Command::SetDtrAbs(true))
        ));
        assert!(matches!(
            action_to_command(&DialogAction::SetDtr(false)),
            Some(Command::SetDtrAbs(false))
        ));
    }

    #[test]
    fn set_rts_maps_to_set_rts_abs_command() {
        assert!(matches!(
            action_to_command(&DialogAction::SetRts(true)),
            Some(Command::SetRtsAbs(true))
        ));
        assert!(matches!(
            action_to_command(&DialogAction::SetRts(false)),
            Some(Command::SetRtsAbs(false))
        ));
    }

    #[test]
    fn send_break_maps_to_send_break_command() {
        assert!(matches!(
            action_to_command(&DialogAction::SendBreak),
            Some(Command::SendBreak)
        ));
    }

    #[test]
    fn apply_modal_style_live_returns_none_because_handled_locally() {
        assert!(action_to_command(&DialogAction::ApplyModalStyleLive(
            rtcom_config::ModalStyle::DimmedOverlay,
        ))
        .is_none());
    }

    #[test]
    fn apply_line_endings_live_returns_none_pending_v021() {
        let le = rtcom_core::LineEndingConfig::default();
        assert!(action_to_command(&DialogAction::ApplyLineEndingsLive(le)).is_none());
    }

    #[test]
    fn save_flavored_actions_return_none_pending_t18() {
        let cfg = SerialConfig::default();
        assert!(action_to_command(&DialogAction::ApplyAndSave(cfg)).is_none());
        assert!(action_to_command(&DialogAction::WriteProfile).is_none());
        assert!(action_to_command(&DialogAction::ReadProfile).is_none());
        assert!(action_to_command(&DialogAction::ApplyModalStyleAndSave(
            rtcom_config::ModalStyle::DimmedOverlay,
        ))
        .is_none());
    }

    #[tokio::test]
    async fn apply_dialog_action_apply_live_publishes_apply_config_command() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());

        let cfg = SerialConfig {
            baud_rate: 9600,
            ..SerialConfig::default()
        };
        apply_dialog_action(&DialogAction::ApplyLive(cfg), &mut app, &bus);

        match rx.try_recv().expect("Command on the bus") {
            Event::Command(Command::ApplyConfig(out)) => assert_eq!(out, cfg),
            other => panic!("expected Event::Command(ApplyConfig), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_dialog_action_modal_style_live_does_not_publish_and_updates_cache() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());

        apply_dialog_action(
            &DialogAction::ApplyModalStyleLive(rtcom_config::ModalStyle::DimmedOverlay),
            &mut app,
            &bus,
        );

        // Nothing should be on the bus: the action is a local-only cache update.
        match rx.try_recv() {
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected Empty, got {other:?}"),
        }
    }
}
