//! Top-level TUI application object.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use rtcom_core::EventBus;
use tui_term::widget::PseudoTerminal;

use crate::{layout::main_chrome, serial_pane::SerialPane};

/// Owns the TUI render state and input dispatcher.
///
/// Tracks the serial data pane, the configuration-menu open/closed
/// state, and a lightweight device summary shown on the top bar.
/// Input handling and the menu overlay are added in follow-up tasks.
pub struct TuiApp {
    // Kept for later tasks (T9+) which will subscribe / publish events.
    #[allow(dead_code)]
    bus: EventBus,
    menu_open: bool,
    serial_pane: SerialPane,
    device_path: String,
    config_summary: String,
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
}
