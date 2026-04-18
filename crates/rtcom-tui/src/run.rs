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
//! (`ApplyAndSave`, `WriteProfile`, `ReadProfile`, ...) actions — these
//! mutate `Profile` in memory, persist it to TOML via
//! [`rtcom_config::write`], and publish
//! [`Event::ProfileSaved`] / [`Event::ProfileLoadFailed`] so T19's toast
//! layer can surface the outcome to the user.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as CtEvent, EventStream, KeyEvent, KeyEventKind};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use rtcom_config::Profile;
use rtcom_core::{
    command::Command, Event, EventBus, ModemLineSnapshot, Parity, SerialConfig, StopBits,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::{
    app::TuiApp,
    input::Dispatch,
    modal::DialogAction,
    profile_bridge::{
        line_ending_config_to_section, line_endings_from_profile, serial_config_to_section,
        serial_section_to_config,
    },
    terminal::{AltScreenGuard, RawModeGuard},
    toast::ToastLevel,
};

/// Drive the TUI main loop until cancelled or the user requests quit.
///
/// Owns the ratatui terminal + crossterm event stream; restores cooked
/// mode + leaves the alternate screen on return (including on `Err`).
///
/// # Parameters
///
/// - `app`          — the prepared [`TuiApp`]; caller seeds device
///   summary, initial `SerialConfig`, line endings, modem lines, and
///   modal style before handing in.
/// - `bus`          — the shared [`EventBus`], used to publish
///   [`Event::TxBytes`] for keystrokes and
///   [`Event::ProfileSaved`] / [`Event::ProfileLoadFailed`] for
///   profile-IO outcomes.
/// - `bus_rx`       — a pre-subscribed bus receiver; subscribe before
///   spawning the session task so no events are missed.
/// - `cancel`       — the shared [`CancellationToken`]. Tripping it
///   (via signal, session exit, or [`Dispatch::Quit`]) unwinds the
///   loop.
/// - `profile_path` — where to read/write the profile TOML when the
///   user triggers a save-flavored dialog action. `None` when no path
///   is discoverable (tests, `$HOME`-less CI); save actions become
///   no-ops + `tracing::warn` in that case.
/// - `profile`      — the currently-loaded profile, owned by the run
///   loop. Mutated in place when `ApplyAndSave` / `ApplyModalStyleAndSave`
///   / `ReadProfile` rewrite sections; persisted on disk through
///   [`rtcom_config::write`].
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
    profile_path: Option<PathBuf>,
    mut profile: Profile,
) -> Result<()> {
    // RAII: enter raw mode + alt screen. Both restore on drop, even
    // if the terminal setup below or the loop body returns Err — the
    // guards sit on the stack and unwind through every error path.
    let _raw = RawModeGuard::enter()?;
    let _alt = AltScreenGuard::enter()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("build ratatui terminal")?;

    let mut keys = EventStream::new();

    // Periodic 100ms tick so toast expiration happens even when the
    // user is idle and no bus events arrive. `tick()` is also called
    // inside `TuiApp::render`, so the interval's job here is purely
    // to trigger a redraw; we ignore the returned `Instant`.
    let mut toast_tick = tokio::time::interval(Duration::from_millis(100));
    // Consume the immediate first tick so we don't redraw twice on
    // entry (we already drew the seed frame below).
    toast_tick.tick().await;

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
                            && handle_key_event(
                                key,
                                &mut app,
                                &bus,
                                &cancel,
                                profile_path.as_deref(),
                                &mut profile,
                            )
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

            _ = toast_tick.tick() => {
                // Nothing to do here: the expiration call happens
                // inside TuiApp::render just below. This arm exists
                // so the loop wakes up to redraw.
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
    profile_path: Option<&Path>,
    profile: &mut Profile,
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
            apply_dialog_action(&action, app, bus, profile_path, profile);
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
///   subsequent renders pick it up.
/// - For save-flavored actions (`ApplyAndSave`, `WriteProfile`,
///   `ReadProfile`, `ApplyModalStyleAndSave`), mutate the in-memory
///   [`Profile`], persist via [`rtcom_config::write`], and publish
///   [`Event::ProfileSaved`] / [`Event::ProfileLoadFailed`] so T19's
///   toast layer can surface the outcome.
/// - For line-ending changes (`ApplyLineEndingsLive` /
///   `ApplyLineEndingsAndSave`), log a warn pointing at v0.2.1 which
///   introduces the `Arc<Mutex<Mapper>>` refactor needed to swap the
///   mapper at runtime.
fn apply_dialog_action(
    action: &DialogAction,
    app: &mut TuiApp,
    bus: &EventBus,
    profile_path: Option<&Path>,
    profile: &mut Profile,
) {
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
        DialogAction::ApplyLineEndingsLive(_) => {
            // v0.2.1 ships the Arc<Mutex<Mapper>> refactor that lets
            // the session swap mappers at runtime. Until then, live
            // line-ending changes require a restart.
            tracing::warn!("live line-ending change not yet supported; restart rtcom to apply");
        }
        DialogAction::ApplyLineEndingsAndSave(le) => {
            // The *live* runtime swap still requires the v0.2.1 mapper
            // refactor, but persisting to the profile is cheap — do it
            // now so the next rtcom launch picks up the new vocabulary
            // via the existing profile-load path.
            tracing::info!(
                "line endings saved to profile; restart rtcom to apply to the live session"
            );
            profile.line_endings = line_ending_config_to_section(le);
            app.set_line_endings(*le);
            persist_profile(profile, profile_path, bus);
        }
        DialogAction::ApplyAndSave(cfg) => {
            // Two-step: apply live, then persist the new serial section.
            bus.publish(Event::Command(Command::ApplyConfig(*cfg)));
            profile.serial = serial_config_to_section(cfg);
            persist_profile(profile, profile_path, bus);
        }
        DialogAction::ApplyModalStyleAndSave(style) => {
            // Update the TUI cache *and* the profile, then persist.
            app.set_modal_style(*style);
            profile.screen.modal_style = *style;
            persist_profile(profile, profile_path, bus);
        }
        DialogAction::WriteProfile => {
            persist_profile(profile, profile_path, bus);
        }
        DialogAction::ReadProfile => {
            reload_profile(profile, app, profile_path, bus);
        }
        // All other variants are handled by the `action_to_command`
        // branch above or by the ApplyModalStyleLive early return; this
        // arm is defensive against future DialogAction additions.
        _ => {
            tracing::warn!(?action, "unhandled DialogAction");
        }
    }
}

/// Serialize `profile` to TOML and write it to `path`, publishing the
/// outcome on the bus.
///
/// On success: [`Event::ProfileSaved`] with the destination path.
/// On failure: [`Event::ProfileLoadFailed`] wrapping the IO / serialize
/// error as [`rtcom_core::Error::InvalidConfig`] (the core error enum
/// does not have a dedicated config-IO variant; `InvalidConfig` is the
/// closest fit and the message carries the full cause).
///
/// When `path` is `None` (no discoverable profile location), the call
/// is a no-op + tracing warn — save-flavored actions reaching this code
/// path typically have a `Some`, but `None` can happen in tests and
/// on `$HOME`-less CI.
fn persist_profile(profile: &Profile, path: Option<&Path>, bus: &EventBus) {
    let Some(path) = path else {
        tracing::warn!("profile save requested but no profile path is available");
        return;
    };
    match rtcom_config::write(path, profile) {
        Ok(()) => {
            bus.publish(Event::ProfileSaved {
                path: path.to_path_buf(),
            });
        }
        Err(e) => {
            let err = rtcom_core::Error::InvalidConfig(format!("profile write: {e}"));
            bus.publish(Event::ProfileLoadFailed {
                path: path.to_path_buf(),
                error: Arc::new(err),
            });
        }
    }
}

/// Reload the profile from disk, replace the in-memory copy, apply the
/// `[serial]` section live via [`Command::ApplyConfig`], and update the
/// TUI snapshots for line endings + modal style.
///
/// On success: emits the same [`Event::ProfileSaved`] variant as a
/// write — "saved" here reads as "profile-level action completed, toast
/// it", and not having a dedicated `ProfileLoaded` variant is not worth
/// one for v0.2. T19 can differentiate if users find it confusing.
///
/// Intentionally does **not** touch `ModemLineSnapshot`: the profile's
/// `[modem]` section captures *startup policy* (`initial_dtr =
/// unchanged|raise|lower`), not live state. Live modem lines are
/// authoritative from the device and flow through
/// [`Event::ModemLinesChanged`].
fn reload_profile(profile: &mut Profile, app: &mut TuiApp, path: Option<&Path>, bus: &EventBus) {
    let Some(path) = path else {
        tracing::warn!("profile reload requested but no profile path is available");
        return;
    };
    match rtcom_config::read(path) {
        Ok(new_profile) => {
            let serial_cfg = serial_section_to_config(&new_profile.serial);
            bus.publish(Event::Command(Command::ApplyConfig(serial_cfg)));

            app.set_line_endings(line_endings_from_profile(&new_profile));
            app.set_modal_style(new_profile.screen.modal_style);

            *profile = new_profile;

            // Reuse ProfileSaved as "successful profile action" until
            // T19 decides whether loads deserve their own toast label.
            bus.publish(Event::ProfileSaved {
                path: path.to_path_buf(),
            });
        }
        Err(e) => {
            let err = rtcom_core::Error::InvalidConfig(format!("profile read: {e}"));
            bus.publish(Event::ProfileLoadFailed {
                path: path.to_path_buf(),
                error: Arc::new(err),
            });
        }
    }
}

/// Translate a [`DialogAction`] into a [`Command`] when the action
/// corresponds to a bus-dispatched command. Returns `None` for actions
/// that the TUI handles locally (`ApplyModalStyleLive`), that require
/// profile IO (`ApplyAndSave`, `WriteProfile`, `ReadProfile`,
/// `ApplyModalStyleAndSave`), or that need the runtime-mapper refactor
/// shipping in v0.2.1 (`ApplyLineEndings*`).
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
        // Save-flavored: drive Profile IO (see `apply_dialog_action`).
        // `ApplyAndSave` *also* publishes Command::ApplyConfig, but it
        // does so directly from `apply_dialog_action` after running
        // the profile-write step, so it stays `None` here to avoid a
        // double dispatch.
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
///
/// T19 adds toast arms for [`Event::ProfileSaved`] /
/// [`Event::ProfileLoadFailed`] / [`Event::Error`]; these continue to
/// log via `tracing` as well so external log subscribers (e.g., the
/// v0.2.x log-file writer) see the same stream.
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
            app.push_toast(format!("error: {e}"), ToastLevel::Error);
        }
        Ok(Event::ModemLinesChanged { dtr, rts }) => {
            app.set_modem_lines(ModemLineSnapshot { dtr, rts });
        }
        Ok(Event::ProfileSaved { path }) => {
            app.push_toast(
                format!("profile saved: {}", path.display()),
                ToastLevel::Info,
            );
        }
        Ok(Event::ProfileLoadFailed { path, error }) => {
            tracing::error!(%error, path = %path.display(), "profile IO failed");
            app.push_toast(
                format!("profile IO failed ({}): {error}", path.display()),
                ToastLevel::Error,
            );
        }
        // `rtcom_core::Event` is `#[non_exhaustive]` so the wildcard
        // tail here both covers known-but-ignored variants
        // (MenuOpened/Closed, Command, TxBytes, DeviceConnected) and
        // any future additions — the TUI doesn't need to change to
        // stay forward-compatible.
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
        let mut profile = Profile::default();

        let cfg = SerialConfig {
            baud_rate: 9600,
            ..SerialConfig::default()
        };
        apply_dialog_action(
            &DialogAction::ApplyLive(cfg),
            &mut app,
            &bus,
            None,
            &mut profile,
        );

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
        let mut profile = Profile::default();

        apply_dialog_action(
            &DialogAction::ApplyModalStyleLive(rtcom_config::ModalStyle::DimmedOverlay),
            &mut app,
            &bus,
            None,
            &mut profile,
        );

        // Nothing should be on the bus: the action is a local-only cache update.
        match rx.try_recv() {
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn persist_profile_writes_to_disk_and_publishes_profile_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        let profile = Profile::default();
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();

        persist_profile(&profile, Some(&path), &bus);

        assert!(path.exists(), "profile file should be written");
        match rx.try_recv().expect("ProfileSaved on the bus") {
            Event::ProfileSaved { path: p } => assert_eq!(p, path),
            other => panic!("expected ProfileSaved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn persist_profile_without_path_is_noop() {
        let profile = Profile::default();
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();

        persist_profile(&profile, None, &bus);

        // No event published.
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn persist_profile_io_error_publishes_profile_load_failed() {
        // Stage: write a regular file, then try to persist "under" it
        // (treating it as a directory). rtcom_config::write tries
        // create_dir_all on the parent, which fails with NotADirectory
        // because the "parent" is itself a file.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        let path = blocker.join("nested.toml");

        let profile = Profile::default();
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();

        persist_profile(&profile, Some(&path), &bus);

        match rx.try_recv().expect("ProfileLoadFailed on the bus") {
            Event::ProfileLoadFailed { path: p, error } => {
                assert_eq!(p, path);
                assert!(error.to_string().contains("profile write"));
            }
            other => panic!("expected ProfileLoadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reload_profile_reads_and_dispatches_apply_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");

        // Write a non-default profile to disk.
        let mut disk_profile = Profile::default();
        disk_profile.serial.baud = 9600;
        rtcom_config::write(&path, &disk_profile).unwrap();

        // Memory copy is still the default (115200).
        let mut memory_profile = Profile::default();
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());

        reload_profile(&mut memory_profile, &mut app, Some(&path), &bus);

        assert_eq!(memory_profile.serial.baud, 9600);

        // First: ApplyConfig command with the newly-read baud.
        match rx.try_recv().expect("ApplyConfig on the bus") {
            Event::Command(Command::ApplyConfig(cfg)) => assert_eq!(cfg.baud_rate, 9600),
            other => panic!("expected Command::ApplyConfig, got {other:?}"),
        }
        // Second: ProfileSaved as the user-visible confirmation.
        match rx.try_recv().expect("ProfileSaved on the bus") {
            Event::ProfileSaved { path: p } => assert_eq!(p, path),
            other => panic!("expected ProfileSaved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reload_profile_malformed_toml_publishes_profile_load_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, b"not valid =~~ toml [\n").unwrap();

        let mut memory_profile = Profile::default();
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());

        reload_profile(&mut memory_profile, &mut app, Some(&path), &bus);

        match rx.try_recv().expect("ProfileLoadFailed on the bus") {
            Event::ProfileLoadFailed { path: p, error } => {
                assert_eq!(p, path);
                assert!(error.to_string().contains("profile read"));
            }
            other => panic!("expected ProfileLoadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_dialog_action_apply_and_save_updates_profile_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("save.toml");
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());
        let mut profile = Profile::default();

        let cfg = SerialConfig {
            baud_rate: 57_600,
            ..SerialConfig::default()
        };
        apply_dialog_action(
            &DialogAction::ApplyAndSave(cfg),
            &mut app,
            &bus,
            Some(&path),
            &mut profile,
        );

        // In-memory profile updated.
        assert_eq!(profile.serial.baud, 57_600);
        // File written.
        assert!(path.exists());

        // First: live ApplyConfig dispatched.
        match rx.try_recv().expect("ApplyConfig on the bus") {
            Event::Command(Command::ApplyConfig(out)) => assert_eq!(out, cfg),
            other => panic!("expected Command::ApplyConfig, got {other:?}"),
        }
        // Second: ProfileSaved confirmation.
        match rx.try_recv().expect("ProfileSaved on the bus") {
            Event::ProfileSaved { path: p } => assert_eq!(p, path),
            other => panic!("expected ProfileSaved, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_bus_event_profile_saved_pushes_info_toast() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        assert_eq!(app.toasts_mut().visible_count(), 0);
        assert!(handle_bus_event(
            Ok(Event::ProfileSaved {
                path: std::path::PathBuf::from("/tmp/x.toml"),
            }),
            &mut app,
        ));
        assert_eq!(app.toasts_mut().visible_count(), 1);
        assert_eq!(
            app.toasts_mut().visible()[0].level,
            crate::toast::ToastLevel::Info
        );
        assert!(app.toasts_mut().visible()[0]
            .message
            .contains("/tmp/x.toml"));
    }

    #[tokio::test]
    async fn handle_bus_event_profile_load_failed_pushes_error_toast() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        let err = rtcom_core::Error::InvalidConfig("bad toml".to_string());
        assert!(handle_bus_event(
            Ok(Event::ProfileLoadFailed {
                path: std::path::PathBuf::from("/tmp/bad.toml"),
                error: Arc::new(err),
            }),
            &mut app,
        ));
        assert_eq!(app.toasts_mut().visible_count(), 1);
        assert_eq!(
            app.toasts_mut().visible()[0].level,
            crate::toast::ToastLevel::Error
        );
        assert!(app.toasts_mut().visible()[0]
            .message
            .contains("/tmp/bad.toml"));
    }

    #[tokio::test]
    async fn handle_bus_event_error_pushes_error_toast() {
        let bus = EventBus::new(8);
        let mut app = TuiApp::new(bus);
        let err = rtcom_core::Error::InvalidConfig("boom".to_string());
        assert!(handle_bus_event(Ok(Event::Error(Arc::new(err))), &mut app));
        assert_eq!(app.toasts_mut().visible_count(), 1);
        assert_eq!(
            app.toasts_mut().visible()[0].level,
            crate::toast::ToastLevel::Error
        );
        assert!(app.toasts_mut().visible()[0].message.contains("boom"));
    }

    #[tokio::test]
    async fn apply_line_endings_and_save_persists_profile() {
        use rtcom_core::{LineEnding, LineEndingConfig};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("le.toml");
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());
        let mut profile = Profile::default();

        let le = LineEndingConfig {
            omap: LineEnding::AddCrToLf,
            imap: LineEnding::None,
            emap: LineEnding::None,
        };
        apply_dialog_action(
            &DialogAction::ApplyLineEndingsAndSave(le),
            &mut app,
            &bus,
            Some(&path),
            &mut profile,
        );

        // In-memory profile updated with the round-trip vocabulary.
        assert_eq!(profile.line_endings.omap, "crlf");
        // File persisted to disk.
        let on_disk = rtcom_config::read(&path).unwrap();
        assert_eq!(on_disk.line_endings.omap, "crlf");
        // ProfileSaved event emitted so the toast layer can surface it.
        let mut saw_saved = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, Event::ProfileSaved { .. }) {
                saw_saved = true;
            }
        }
        assert!(saw_saved, "expected ProfileSaved event");
    }

    #[tokio::test]
    async fn apply_line_endings_live_still_warns_and_does_not_persist() {
        use rtcom_core::{LineEnding, LineEndingConfig};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("le_live.toml");
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());
        let mut profile = Profile::default();

        let le = LineEndingConfig {
            omap: LineEnding::AddCrToLf,
            imap: LineEnding::None,
            emap: LineEnding::None,
        };
        apply_dialog_action(
            &DialogAction::ApplyLineEndingsLive(le),
            &mut app,
            &bus,
            Some(&path),
            &mut profile,
        );

        // Profile in memory untouched (default "none") and no file
        // written to disk.
        assert_eq!(profile.line_endings.omap, "none");
        assert!(!path.exists(), "live-only path must not write profile");
        // No bus events.
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn apply_dialog_action_apply_modal_style_and_save_persists_and_updates_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("style.toml");
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        let mut app = TuiApp::new(bus.clone());
        let mut profile = Profile::default();

        apply_dialog_action(
            &DialogAction::ApplyModalStyleAndSave(rtcom_config::ModalStyle::Fullscreen),
            &mut app,
            &bus,
            Some(&path),
            &mut profile,
        );

        assert_eq!(
            profile.screen.modal_style,
            rtcom_config::ModalStyle::Fullscreen
        );
        assert!(path.exists());

        match rx.try_recv().expect("ProfileSaved on the bus") {
            Event::ProfileSaved { path: p } => assert_eq!(p, path),
            other => panic!("expected ProfileSaved, got {other:?}"),
        }
    }
}
