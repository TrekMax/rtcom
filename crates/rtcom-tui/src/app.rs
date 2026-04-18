//! Top-level TUI application object.

use crossterm::event::KeyEvent;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use rtcom_core::{
    command::{Command, CommandKeyParser, ParseOutput},
    Event, EventBus,
};
use tui_term::widget::PseudoTerminal;

use crate::{input::Dispatch, layout::main_chrome, serial_pane::SerialPane};

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
}

impl TuiApp {
    /// Construct a new `TuiApp` bound to the given event bus.
    ///
    /// Starts with a `24x80` serial pane; the pane is resized to the
    /// terminal body on every call to [`TuiApp::render`].
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
        }
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
    /// - [`Command::OpenMenu`] flips `menu_open` and publishes
    ///   [`Event::MenuOpened`] on the bus, then returns
    ///   [`Dispatch::OpenedMenu`].
    /// - [`Command::Quit`] returns [`Dispatch::Quit`].
    /// - Any other [`Command`] is published on the bus as
    ///   [`Event::Command`]; the dispatcher returns [`Dispatch::Noop`]
    ///   (T17 refactors this into direct `Session` handles).
    ///
    /// When the menu is open, T9 currently swallows the input and
    /// returns [`Dispatch::Noop`]. T10+ will forward the event to the
    /// `ModalStack`.
    pub fn handle_key(&mut self, key: KeyEvent) -> Dispatch {
        if self.menu_open {
            // T10+ will forward this to modal_stack.handle_key(...).
            return Dispatch::Noop;
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

        // Body: serial pane via tui-term's PseudoTerminal widget.
        let term_widget = PseudoTerminal::new(self.serial_pane.screen());
        f.render_widget(term_widget, body);

        // Bottom bar: hint text.
        let bottom_line = Line::from(Span::styled(
            " ^A m menu · ^A ? help · ^A q quit ",
            Style::default().add_modifier(Modifier::DIM),
        ));
        f.render_widget(Paragraph::new(bottom_line), bottom);
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
    fn second_ctrl_a_m_closes_menu() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        // open
        let _ = app.handle_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let _ = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(app.is_menu_open());
        // close — menu-open input handling isn't wired yet (T10+),
        // so T9 only needs to accept the input and return Noop when
        // menu is open. We'll wire actual closing in T11.
        let out = app.handle_key(key(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(matches!(out, Dispatch::Noop));
        // Menu still open until T11+ wires the ModalStack.
        assert!(app.is_menu_open());
    }

    #[test]
    fn enter_emits_cr_byte() {
        let bus = EventBus::new(64);
        let mut app = TuiApp::new(bus);
        let out = app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(out, Dispatch::TxBytes(ref b) if b == b"\r"));
    }
}
