//! Top-level TUI application object.

use crossterm::event::KeyEvent;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use rtcom_config::ModalStyle;
use rtcom_core::{
    command::{Command, CommandKeyParser, ParseOutput},
    Event, EventBus, LineEndingConfig, ModemLineSnapshot, SerialConfig,
};
use tui_term::widget::PseudoTerminal;

use crate::{
    input::Dispatch,
    layout::main_chrome,
    menu::RootMenu,
    modal::{DialogOutcome, ModalStack},
    serial_pane::SerialPane,
    toast::{render_toasts, ToastLevel, ToastQueue},
};

/// Owns the TUI render state and input dispatcher.
///
/// Tracks the serial data pane, the configuration-menu open/closed
/// state, and a lightweight device summary shown on the top bar.
/// Input handling lives in [`TuiApp::handle_key`], which routes
/// keyboard events through an internal [`CommandKeyParser`] whenever
/// the menu is closed.
pub struct TuiApp {
    bus: EventBus,
    menu_open: bool,
    serial_pane: SerialPane,
    device_path: String,
    config_summary: String,
    parser: CommandKeyParser,
    modal_stack: ModalStack,
    /// Current serial-link configuration; seeded to
    /// [`SerialConfig::default`] at construction and updated by
    /// [`TuiApp::set_serial_config`]. Forwarded into new [`RootMenu`]
    /// instances so sub-dialogs (starting with T12's
    /// [`crate::menu::SerialPortSetupDialog`]) can display live values.
    current_config: SerialConfig,
    /// Current line-ending mapper configuration; seeded to
    /// [`LineEndingConfig::default`] at construction and updated by
    /// [`TuiApp::set_line_endings`]. Forwarded into new [`RootMenu`]
    /// instances so the T13 [`crate::menu::LineEndingsDialog`] opens
    /// with live values.
    current_line_endings: LineEndingConfig,
    /// Current DTR / RTS output-line snapshot as known to rtcom;
    /// seeded to [`ModemLineSnapshot::default`] (both lines
    /// de-asserted) at construction and updated by
    /// [`TuiApp::set_modem_lines`]. Forwarded into new [`RootMenu`]
    /// instances so the T14 [`crate::menu::ModemControlDialog`] opens
    /// with live values.
    current_modem: ModemLineSnapshot,
    /// Current modal render style; seeded to [`ModalStyle::default`]
    /// at construction and updated by
    /// [`TuiApp::set_modal_style`]. Forwarded into new [`RootMenu`]
    /// instances so the T15 [`crate::menu::ScreenOptionsDialog`] opens
    /// with the live value.
    current_modal_style: ModalStyle,
    /// Queue of timed toast notifications. Populated by the runner's
    /// bus-event handler for [`Event::ProfileSaved`] /
    /// [`Event::ProfileLoadFailed`] / [`Event::Error`]. Rendered on
    /// top of the main chrome + modal in [`TuiApp::render`] so
    /// outcome messages are always visible.
    toasts: ToastQueue,
}

impl TuiApp {
    /// Construct a new `TuiApp` bound to the given event bus.
    ///
    /// Starts with a `24x80` serial pane and a default [`SerialConfig`];
    /// the pane is resized to the terminal body on every call to
    /// [`TuiApp::render`], and the config is overwritten by
    /// [`TuiApp::set_serial_config`] once the runner knows the real
    /// link parameters.
    #[must_use]
    pub fn new(bus: EventBus) -> Self {
        Self {
            bus,
            menu_open: false,
            // 24x80 is a safe default; actual size is set on first render.
            serial_pane: SerialPane::new(24, 80),
            device_path: String::new(),
            config_summary: String::new(),
            parser: CommandKeyParser::default(),
            modal_stack: ModalStack::new(),
            current_config: SerialConfig::default(),
            current_line_endings: LineEndingConfig::default(),
            current_modem: ModemLineSnapshot::default(),
            current_modal_style: ModalStyle::default(),
            toasts: ToastQueue::new(),
        }
    }

    /// Push a new toast onto the queue. Consumed by the runner's
    /// bus-event handler for profile IO + error events.
    pub fn push_toast(&mut self, message: impl Into<String>, level: ToastLevel) {
        self.toasts.push(message, level);
    }

    /// Mutable access to the toast queue. Mainly used by tests and
    /// the main-loop tick to advance expiration.
    pub fn toasts_mut(&mut self) -> &mut ToastQueue {
        &mut self.toasts
    }

    /// Immutable borrow of the toast queue (read-only introspection).
    #[must_use]
    pub const fn toasts(&self) -> &ToastQueue {
        &self.toasts
    }

    /// Update the cached [`SerialConfig`] that new [`RootMenu`] pushes
    /// pass down to sub-dialogs.
    ///
    /// Call this whenever the live session's config changes (T17 wires
    /// this into `Event::ConfigChanged`).
    pub const fn set_serial_config(&mut self, cfg: SerialConfig) {
        self.current_config = cfg;
    }

    /// Update the cached [`LineEndingConfig`] that new [`RootMenu`]
    /// pushes pass down to the T13
    /// [`crate::menu::LineEndingsDialog`].
    ///
    /// Call this whenever the live session's mapper configuration
    /// changes (T17 wires this into the `ApplyLineEndingsLive` path).
    pub const fn set_line_endings(&mut self, le: LineEndingConfig) {
        self.current_line_endings = le;
    }

    /// Update the cached [`ModemLineSnapshot`] that new [`RootMenu`]
    /// pushes pass down to the T14
    /// [`crate::menu::ModemControlDialog`].
    ///
    /// Call this whenever the live session's modem output lines
    /// change (T17 wires this into the `SetDtr` / `SetRts` paths).
    pub const fn set_modem_lines(&mut self, snapshot: ModemLineSnapshot) {
        self.current_modem = snapshot;
    }

    /// Update the cached [`ModalStyle`] that new [`RootMenu`] pushes
    /// pass down to the T15 [`crate::menu::ScreenOptionsDialog`].
    ///
    /// Call this whenever the live modal-style preference changes
    /// (T17 wires this into the `ApplyModalStyleLive` / `AndSave`
    /// paths).
    pub const fn set_modal_style(&mut self, style: ModalStyle) {
        self.current_modal_style = style;
    }

    /// Whether the configuration menu is currently open.
    #[must_use]
    pub const fn is_menu_open(&self) -> bool {
        self.menu_open
    }

    /// Update the device path + config summary shown on the top bar.
    ///
    /// Accepts any type convertible to `String` so call sites can pass
    /// either borrowed or owned strings.
    pub fn set_device_summary(
        &mut self,
        device_path: impl Into<String>,
        config_summary: impl Into<String>,
    ) {
        self.device_path = device_path.into();
        self.config_summary = config_summary.into();
    }

    /// Update just the config-summary portion of the top bar, leaving
    /// the device path untouched.
    ///
    /// Used by the bus subscriber to refresh the status line after an
    /// [`Event::ConfigChanged`] without having to know the device path.
    pub fn set_config_summary(&mut self, config_summary: impl Into<String>) {
        self.config_summary = config_summary.into();
    }

    /// Mutable access to the serial data pane.
    ///
    /// Primarily used by the serial-reader subscriber to ingest incoming
    /// bytes; tests also use it to seed a known screen state.
    pub fn serial_pane_mut(&mut self) -> &mut SerialPane {
        &mut self.serial_pane
    }

    /// Internal accessor for the bus (later tasks wire this in).
    #[allow(dead_code)]
    pub(crate) const fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Route a key event.
    ///
    /// When the menu is closed, the event is converted to bytes via
    /// [`crate::input::key_to_bytes`] and fed one byte at a time to
    /// the internal [`CommandKeyParser`]:
    ///
    /// - [`ParseOutput::Data`] bytes accumulate into a
    ///   [`Dispatch::TxBytes`] payload.
    /// - [`Command::OpenMenu`] flips `menu_open`, pushes a
    ///   [`RootMenu`] onto the modal stack, publishes
    ///   [`Event::MenuOpened`], and returns [`Dispatch::OpenedMenu`].
    /// - [`Command::Quit`] returns [`Dispatch::Quit`].
    /// - Any other [`Command`] is published on the bus as
    ///   [`Event::Command`]; the dispatcher returns [`Dispatch::Noop`]
    ///   (T17 refactors this into direct `Session` handles).
    ///
    /// When the menu is open, the event is handed to the topmost
    /// [`crate::modal::Dialog`] on the [`ModalStack`]. The stack
    /// auto-manages `Close` / `Push` outcomes; this function only
    /// needs to detect the root dialog closing (stack becomes empty)
    /// to publish [`Event::MenuClosed`] and flip `menu_open` back.
    /// `Action` outcomes bubble up as [`Dispatch::Action`] for the
    /// runner to apply.
    pub fn handle_key(&mut self, key: KeyEvent) -> Dispatch {
        if self.menu_open {
            let outcome = self.modal_stack.handle_key(key);
            if self.modal_stack.is_empty() {
                // Root dialog closed; menu is fully dismissed.
                self.menu_open = false;
                let _ = self.bus.publish(Event::MenuClosed);
                return Dispatch::ClosedMenu;
            }
            return match outcome {
                DialogOutcome::Action(action) => Dispatch::Action(action),
                _ => Dispatch::Noop,
            };
        }

        let bytes = crate::input::key_to_bytes(key);
        if bytes.is_empty() {
            return Dispatch::Noop;
        }

        let mut tx = Vec::new();
        for &b in &bytes {
            match self.parser.feed(b) {
                ParseOutput::None => {}
                ParseOutput::Data(data_byte) => tx.push(data_byte),
                ParseOutput::Command(Command::OpenMenu) => {
                    self.menu_open = true;
                    self.modal_stack.push(Box::new(RootMenu::new(
                        self.current_config,
                        self.current_line_endings,
                        self.current_modem,
                        self.current_modal_style,
                    )));
                    let _ = self.bus.publish(Event::MenuOpened);
                    return Dispatch::OpenedMenu;
                }
                ParseOutput::Command(Command::Quit) => {
                    return Dispatch::Quit;
                }
                ParseOutput::Command(cmd) => {
                    // Forward all other commands onto the bus; T17
                    // refactors this into direct Session handles.
                    let _ = self.bus.publish(Event::Command(cmd));
                }
            }
        }

        if tx.is_empty() {
            Dispatch::Noop
        } else {
            Dispatch::TxBytes(tx)
        }
    }

    /// Render the main screen into `f`.
    ///
    /// Layout: 1-row top bar ("rtcom {version} | {device} | {config}"),
    /// body (serial pane rendered via [`tui_term`]), 1-row bottom bar
    /// with command-key hints. The serial pane is resized to the body
    /// size every frame so it follows terminal resizes.
    ///
    /// When the configuration menu is open, the body is drawn according
    /// to the current [`ModalStyle`] (set via
    /// [`TuiApp::set_modal_style`]):
    ///
    /// - [`ModalStyle::Overlay`]: serial pane drawn normally; the
    ///   modal dialog is painted over it at its preferred size.
    /// - [`ModalStyle::DimmedOverlay`]: serial pane drawn normally,
    ///   then every body cell has [`Modifier::DIM`] OR-ed into its
    ///   style so the stream fades behind the modal. The modal is then
    ///   painted on top at full brightness.
    /// - [`ModalStyle::Fullscreen`]: the serial pane is **not** drawn
    ///   at all; the modal fills the entire body area. The top/bottom
    ///   chrome bars remain visible.
    pub fn render(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        let (top, body, bottom) = main_chrome(area);

        // Keep the serial pane's internal grid aligned with the body.
        if body.height > 0 && body.width > 0 {
            self.serial_pane.resize(body.height, body.width);
        }

        // Top bar.
        let version = env!("CARGO_PKG_VERSION");
        let top_line = Line::from(vec![
            Span::styled(
                format!(" rtcom {version} "),
                Style::default().add_modifier(Modifier::REVERSED),
            ),
            Span::raw("  "),
            Span::raw(self.device_path.clone()),
            Span::raw("  "),
            Span::raw(self.config_summary.clone()),
        ]);
        f.render_widget(Paragraph::new(top_line), top);

        // Body: whether to draw the live serial pane, and whether to
        // dim it, depends on the current modal style and menu state.
        let in_fullscreen_menu =
            self.menu_open && self.current_modal_style == ModalStyle::Fullscreen;

        if !in_fullscreen_menu {
            // Serial pane via tui-term's PseudoTerminal widget.
            let term_widget = PseudoTerminal::new(self.serial_pane.screen());
            f.render_widget(term_widget, body);

            // DimmedOverlay: OR DIM into every cell in the body area so
            // the background stream fades behind the upcoming modal.
            // `Buffer::set_style` composes by OR-ing `add_modifier` into
            // each cell, preserving existing fg/bg/modifiers.
            if self.menu_open && self.current_modal_style == ModalStyle::DimmedOverlay {
                f.buffer_mut()
                    .set_style(body, Style::default().add_modifier(Modifier::DIM));
            }
        }

        // Bottom bar: hint text.
        let bottom_line = Line::from(Span::styled(
            " ^A m menu · ^A ? help · ^A ^Q quit ",
            Style::default().add_modifier(Modifier::DIM),
        ));
        f.render_widget(Paragraph::new(bottom_line), bottom);

        // Modal overlay: topmost dialog drawn over the (possibly
        // dimmed, possibly skipped) body. In Fullscreen mode the
        // modal fills the body area; otherwise it uses its preferred
        // size (typically a centered box).
        if self.menu_open {
            if let Some(top_dialog) = self.modal_stack.top() {
                let dialog_area = if in_fullscreen_menu {
                    body
                } else {
                    top_dialog.preferred_size(area)
                };
                top_dialog.render(dialog_area, f.buffer_mut());
            }
        }

        // Toast overlay: tick to drop expired entries, then draw the
        // remainder on top of the main chrome *and* the modal so
        // outcome messages stay visible regardless of menu state.
        self.toasts.tick();
        if !self.toasts.is_empty() {
            // Reserve up to max_visible rows immediately below the top
            // bar; clamp so we never overflow the terminal height.
            let height = u16::try_from(self.toasts.visible_count())
                .unwrap_or(u16::MAX)
                .min(area.height.saturating_sub(top.height));
            if height > 0 {
                let toast_area = Rect {
                    x: area.x,
                    y: area.y + top.height,
                    width: area.width,
                    height,
                };
                render_toasts(&self.toasts, toast_area, f.buffer_mut());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dispatch;
    use ratatui::{backend::TestBackend, Terminal};
    use rtcom_core::EventBus;

    fn render_app(app: &mut TuiApp, width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        terminal
    }

    #[test]
    fn tui_app_builds_without_running() {
        let bus = EventBus::new(64);
        let app = TuiApp::new(bus);
        assert!(!app.is_menu_open());
    }

    #[test]
    fn main_screen_80x24_empty_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        let terminal = render_app(&mut app, 80, 24);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn main_screen_80x24_with_serial_data_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        app.serial_pane_mut().ingest(b"boot: starting...\r\nok\r\n");
        let terminal = render_app(&mut app, 80, 24);
        insta::assert_snapshot!(terminal.backend());
    }

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    const fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn key_passthrough_when_menu_closed() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        let out = app.handle_key(key(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(matches!(out, Dispatch::TxBytes(ref b) if b == b"h"));
    }

    #[test]
    fn ctrl_a_then_m_opens_menu() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        let step1 = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert!(matches!(step1, Dispatch::Noop));
        let step2 = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(matches!(step2, Dispatch::OpenedMenu));
        assert!(app.is_menu_open());
    }

    #[test]
    fn ctrl_q_requests_quit() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        // Bytes: ^A then ^Q
        let _ = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let out = app.handle_key(key(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(matches!(out, Dispatch::Quit));
    }

    #[test]
    fn ctrl_a_m_second_press_is_swallowed_by_menu() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        // open
        let _ = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let _ = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(app.is_menu_open());
        // With the modal stack wired in T11, menu-open keys go to the
        // root dialog. `^A` reaches the dialog as `0x01` (a plain
        // unprintable Ctrl char), which the root menu simply consumes.
        // The menu stays open.
        let out = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert!(matches!(out, Dispatch::Noop));
        let out = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(matches!(out, Dispatch::Noop));
        assert!(app.is_menu_open());
    }

    #[test]
    fn esc_in_root_menu_closes_it() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        let _ = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let _ = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(app.is_menu_open());
        let out = app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(out, Dispatch::ClosedMenu));
        assert!(!app.is_menu_open());
    }

    #[test]
    fn main_screen_80x24_menu_open_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        let _ = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let _ = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(app.is_menu_open());
        let terminal = render_app(&mut app, 80, 24);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn main_screen_80x24_serial_port_setup_open_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        // Open menu (^A m), then Enter on "Serial port setup" (idx 0).
        let _ = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let _ = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        let _ = app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.is_menu_open());
        let terminal = render_app(&mut app, 80, 24);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn enter_emits_cr_byte() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        let out = app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(out, Dispatch::TxBytes(ref b) if b == b"\r"));
    }

    #[test]
    fn push_toast_appears_in_queue() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        assert_eq!(app.toasts().visible_count(), 0);
        app.push_toast("saved", crate::toast::ToastLevel::Info);
        assert_eq!(app.toasts().visible_count(), 1);
        assert_eq!(app.toasts().visible()[0].message, "saved");
    }

    #[test]
    fn main_screen_80x24_with_toast_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        app.push_toast(
            "profile saved: ~/.config/rtcom/default.toml",
            crate::toast::ToastLevel::Info,
        );
        let terminal = render_app(&mut app, 80, 24);
        insta::assert_snapshot!(terminal.backend());
    }

    // ----- T20: ModalStyle render matrix -----

    /// Helper: open the configuration menu via `^A m`. Leaves the root
    /// dialog on top of the modal stack.
    fn open_menu(app: &mut TuiApp) {
        let _ = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let _ = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(app.is_menu_open());
    }

    #[test]
    fn main_screen_120x40_empty_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        let terminal = render_app(&mut app, 120, 40);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn main_screen_120x40_menu_open_overlay_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        open_menu(&mut app);
        let terminal = render_app(&mut app, 120, 40);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn main_screen_80x24_menu_open_dimmed_overlay_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        app.set_modal_style(ModalStyle::DimmedOverlay);
        // Seed the serial pane so dimming is applied over visible
        // content (the DIM modifier is invisible in TestBackend's
        // text output, but the content itself should still appear).
        app.serial_pane_mut()
            .ingest(b"background line one\r\nbackground line two\r\n");
        open_menu(&mut app);
        let terminal = render_app(&mut app, 80, 24);
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn main_screen_80x24_menu_open_fullscreen_snapshot() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        app.set_modal_style(ModalStyle::Fullscreen);
        // Content ingested but should NOT appear: fullscreen hides the
        // serial pane entirely while the menu is open.
        app.serial_pane_mut().ingest(b"hidden background\r\n");
        open_menu(&mut app);
        let terminal = render_app(&mut app, 80, 24);
        insta::assert_snapshot!(terminal.backend());
    }

    // ----- T20: direct buffer inspection tests -----
    //
    // TestBackend's `Display` impl only emits cell symbols, so a
    // snapshot alone cannot distinguish DimmedOverlay from Overlay.
    // These tests inspect the rendered buffer to verify each
    // ModalStyle actually has the intended effect on cell styles.

    fn dim_probe_at(app: &mut TuiApp, width: u16, height: u16) -> ratatui::style::Style {
        use ratatui::layout::Position;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        // (0, 1) = first column of the first body row (below the top
        // bar). Well outside the centered modal on an 80x24 screen.
        let buf = terminal.backend().buffer();
        buf.cell(Position::new(0, 1)).unwrap().style()
    }

    #[test]
    fn dimmed_overlay_actually_dims_body_cells() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        app.set_modal_style(ModalStyle::DimmedOverlay);
        app.serial_pane_mut().ingest(b"hello\r\n");
        open_menu(&mut app);
        let style = dim_probe_at(&mut app, 80, 24);
        assert!(
            style.add_modifier.contains(Modifier::DIM),
            "expected DIM on body cell outside modal, got {style:?}"
        );
    }

    #[test]
    fn overlay_does_not_dim_body_cells() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        // Default is Overlay.
        assert_eq!(app.current_modal_style, ModalStyle::Overlay);
        app.serial_pane_mut().ingest(b"hello\r\n");
        open_menu(&mut app);
        let style = dim_probe_at(&mut app, 80, 24);
        assert!(
            !style.add_modifier.contains(Modifier::DIM),
            "expected no DIM on body cell with Overlay style, got {style:?}"
        );
    }

    #[test]
    fn fullscreen_menu_hides_serial_pane_content() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
        app.set_modal_style(ModalStyle::Fullscreen);
        // Distinctive marker that would appear in the top-left of the
        // body if the serial pane were drawn.
        app.serial_pane_mut().ingest(b"ZZZZZ-secret-marker\r\n");
        open_menu(&mut app);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let rendered = format!("{}", terminal.backend());
        assert!(
            !rendered.contains("ZZZZZ-secret-marker"),
            "Fullscreen menu should hide serial pane content, \
             but marker leaked into render:\n{rendered}"
        );
    }
}
