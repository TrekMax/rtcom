//! Top-level configuration menu.
//!
//! Seven items, arrow / vim navigation with wrap, Enter drills into
//! child dialogs (placeholders until T12+). Esc or the "Exit menu"
//! item closes the menu.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};
use rtcom_core::{LineEndingConfig, SerialConfig};

use crate::{
    menu::{placeholder::PlaceholderDialog, serial_port::SerialPortSetupDialog},
    modal::{Dialog, DialogOutcome},
};

/// Index of the "Serial port setup" item; selecting it drills into
/// the real [`SerialPortSetupDialog`] (T12).
const SERIAL_PORT_SETUP_INDEX: usize = 0;

/// Top-level configuration menu (the first real [`Dialog`] impl).
///
/// Owns a fixed list of seven entries, an integer cursor, a snapshot
/// of the current [`SerialConfig`] (passed on to sub-dialogs), and a
/// rendering style. Emits [`DialogOutcome::Push`] for every non-exit
/// selection and [`DialogOutcome::Close`] for Esc / "Exit menu".
pub struct RootMenu {
    items: &'static [&'static str],
    selected: usize,
    /// Snapshot of the live [`SerialConfig`]; forwarded to
    /// [`SerialPortSetupDialog::new`] when the user drills in.
    initial_config: SerialConfig,
    /// Snapshot of the live [`LineEndingConfig`]; GREEN commit
    /// forwards it to `LineEndingsDialog::new` when the user drills
    /// into the "Line endings" row.
    #[allow(dead_code, reason = "GREEN commit wires this into activate()")]
    initial_line_endings: LineEndingConfig,
}

const ITEMS: &[&str] = &[
    "Serial port setup", // 0
    "Line endings",      // 1
    "Modem control",     // 2
    // visual separator between config and profile groups
    "Write profile", // 3
    "Read profile",  // 4
    // visual separator between profile and screen groups
    "Screen options", // 5
    "Exit menu",      // 6
];

/// Index of the "Exit menu" sentinel; selecting it closes the menu.
const EXIT_INDEX: usize = 6;

/// Indices after which a visual separator row is drawn.
const SEPARATORS_AFTER: &[usize] = &[2, 4];

impl RootMenu {
    /// Construct a root menu with the cursor on the first item and
    /// snapshotting `initial_config` + `initial_line_endings` for
    /// forwarding to sub-dialogs (currently [`SerialPortSetupDialog`]
    /// and the GREEN-commit `LineEndingsDialog`).
    #[must_use]
    pub const fn new(initial_config: SerialConfig, initial_line_endings: LineEndingConfig) -> Self {
        Self {
            items: ITEMS,
            selected: 0,
            initial_config,
            initial_line_endings,
        }
    }

    /// Current cursor position (0-based).
    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Items in display order.
    #[must_use]
    pub const fn items(&self) -> &'static [&'static str] {
        self.items
    }

    /// Move the cursor up one row, wrapping to the last item when
    /// called on the first.
    fn move_up(&mut self) {
        if self.selected == 0 {
            self.selected = self.items.len() - 1;
        } else {
            self.selected -= 1;
        }
    }

    /// Move the cursor down one row, wrapping to the first item when
    /// called on the last.
    fn move_down(&mut self) {
        if self.selected + 1 >= self.items.len() {
            self.selected = 0;
        } else {
            self.selected += 1;
        }
    }

    /// Handle the Enter key. Exit item closes; the "Serial port setup"
    /// row pushes the real [`SerialPortSetupDialog`] (T12); everything
    /// else still pushes a placeholder until its real dialog lands
    /// (T13+).
    fn activate(&self) -> DialogOutcome {
        match self.selected {
            EXIT_INDEX => DialogOutcome::Close,
            SERIAL_PORT_SETUP_INDEX => {
                DialogOutcome::Push(Box::new(SerialPortSetupDialog::new(self.initial_config)))
            }
            _ => {
                let title = self.items[self.selected];
                DialogOutcome::Push(Box::new(PlaceholderDialog::new(title)))
            }
        }
    }
}

impl Dialog for RootMenu {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Configuration"
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Configuration");
        let inner = block.inner(area);
        block.render(area, buf);

        // Build one visual row per item, interleaving separators.
        let mut lines: Vec<Line<'_>> =
            Vec::with_capacity(self.items.len() + SEPARATORS_AFTER.len());
        for (idx, item) in self.items.iter().enumerate() {
            let style = if idx == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let prefix = if idx == self.selected { "> " } else { "  " };
            lines.push(Line::from(vec![Span::styled(
                format!("{prefix}{item}"),
                style,
            )]));
            if SEPARATORS_AFTER.contains(&idx) {
                let width = usize::from(inner.width);
                lines.push(Line::from(Span::styled(
                    "-".repeat(width),
                    Style::default().add_modifier(Modifier::DIM),
                )));
            }
        }

        Paragraph::new(lines).render(inner, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                DialogOutcome::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                DialogOutcome::Consumed
            }
            KeyCode::Esc => DialogOutcome::Close,
            KeyCode::Enter => self.activate(),
            _ => DialogOutcome::Consumed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn menu() -> RootMenu {
        RootMenu::new(SerialConfig::default(), LineEndingConfig::default())
    }

    #[test]
    fn root_menu_starts_on_first_item() {
        let m = menu();
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn root_menu_down_moves_selection() {
        let mut m = menu();
        m.handle_key(key(KeyCode::Down));
        assert_eq!(m.selected(), 1);
    }

    #[test]
    fn root_menu_up_wraps_from_first() {
        let mut m = menu();
        m.handle_key(key(KeyCode::Up));
        assert_eq!(m.selected(), 6);
    }

    #[test]
    fn root_menu_down_wraps_from_last() {
        let mut m = menu();
        for _ in 0..6 {
            m.handle_key(key(KeyCode::Down));
        }
        assert_eq!(m.selected(), 6);
        m.handle_key(key(KeyCode::Down));
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn j_k_vim_bindings_work() {
        let mut m = menu();
        m.handle_key(key(KeyCode::Char('j')));
        assert_eq!(m.selected(), 1);
        m.handle_key(key(KeyCode::Char('k')));
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn enter_on_first_item_pushes_serial_setup_dialog() {
        let mut m = menu();
        let out = m.handle_key(key(KeyCode::Enter));
        match out {
            DialogOutcome::Push(d) => assert_eq!(d.title(), "Serial port setup"),
            _ => panic!("expected Push"),
        }
    }

    #[test]
    fn enter_on_exit_closes_menu() {
        let mut m = menu();
        for _ in 0..6 {
            m.handle_key(key(KeyCode::Down));
        }
        assert_eq!(m.selected(), 6);
        let out = m.handle_key(key(KeyCode::Enter));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn esc_closes() {
        let mut m = menu();
        let out = m.handle_key(key(KeyCode::Esc));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn unknown_key_is_consumed_no_movement() {
        let mut m = menu();
        let out = m.handle_key(key(KeyCode::Char('x')));
        assert!(matches!(out, DialogOutcome::Consumed));
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn new_takes_serial_config() {
        // Compile-time check that RootMenu::new accepts a SerialConfig.
        let cfg = SerialConfig {
            baud_rate: 9600,
            ..SerialConfig::default()
        };
        let m = RootMenu::new(cfg, LineEndingConfig::default());
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn enter_on_line_endings_pushes_line_endings_dialog() {
        let mut m = RootMenu::new(SerialConfig::default(), LineEndingConfig::default());
        // cursor=0 is Serial port. Move to 1 (Line endings).
        m.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let out = m.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match out {
            DialogOutcome::Push(d) => assert_eq!(d.title(), "Line endings"),
            _ => panic!("expected Push"),
        }
    }
}
