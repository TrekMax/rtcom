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
    Event, EventBus, SerialConfig,
};
use tui_term::widget::PseudoTerminal;

use crate::{
    input::Dispatch,
    layout::main_chrome,
    menu::RootMenu,
    modal::{DialogOutcome, ModalStack},
    serial_pane::SerialPane,
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
        }
    }

    /// Update the cached [`SerialConfig`] that new [`RootMenu`] pushes
    /// pass down to sub-dialogs.
    ///
    /// Call this whenever the live session's config changes (T17 wires
    /// this into `Event::ConfigChanged`).
    pub const fn set_serial_config(&mut self, cfg: SerialConfig) {
        self.current_config = cfg;
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
                    self.modal_stack
                        .push(Box::new(RootMenu::new(self.current_config)));
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

        // Modal overlay: topmost dialog drawn at its preferred size.
        if self.menu_open {
            if let Some(top) = self.modal_stack.top() {
                let dialog_area = top.preferred_size(area);
                top.render(dialog_area, f.buffer_mut());
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
}
