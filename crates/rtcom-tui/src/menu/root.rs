//! Top-level configuration menu.
//!
//! Seven items, arrow / vim navigation with wrap, Enter drills into
//! child dialogs (placeholders until T12+). Esc or the "Exit menu"
//! item closes the menu.

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

#[allow(
    unused_imports,
    reason = "PlaceholderDialog is consumed by the T11 GREEN impl"
)]
use crate::{
    menu::placeholder::PlaceholderDialog,
    modal::{Dialog, DialogOutcome},
};

/// Top-level configuration menu (the first real [`Dialog`] impl).
pub struct RootMenu {
    items: &'static [&'static str],
    selected: usize,
}

const ITEMS: &[&str] = &[
    "Serial port setup", // 0
    "Line endings",      // 1
    "Modem control",     // 2
    // separator after index 2
    "Write profile", // 3
    "Read profile",  // 4
    // separator after index 4
    "Screen options", // 5
    "Exit menu",      // 6
];

impl Default for RootMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl RootMenu {
    /// Construct a root menu with the cursor on the first item.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: ITEMS,
            selected: 0,
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
}

impl Dialog for RootMenu {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Configuration"
    }

    fn render(&self, _area: Rect, _buf: &mut Buffer) {
        // stub: T11 impl phase will render
    }

    fn handle_key(&mut self, _key: KeyEvent) -> DialogOutcome {
        // stub: tests must fail
        DialogOutcome::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn root_menu_starts_on_first_item() {
        let m = RootMenu::new();
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn root_menu_down_moves_selection() {
        let mut m = RootMenu::new();
        m.handle_key(key(KeyCode::Down));
        assert_eq!(m.selected(), 1);
    }

    #[test]
    fn root_menu_up_wraps_from_first() {
        let mut m = RootMenu::new();
        m.handle_key(key(KeyCode::Up));
        assert_eq!(m.selected(), 6);
    }

    #[test]
    fn root_menu_down_wraps_from_last() {
        let mut m = RootMenu::new();
        for _ in 0..6 {
            m.handle_key(key(KeyCode::Down));
        }
        assert_eq!(m.selected(), 6);
        m.handle_key(key(KeyCode::Down));
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn j_k_vim_bindings_work() {
        let mut m = RootMenu::new();
        m.handle_key(key(KeyCode::Char('j')));
        assert_eq!(m.selected(), 1);
        m.handle_key(key(KeyCode::Char('k')));
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn enter_on_first_item_pushes_serial_setup_placeholder() {
        let mut m = RootMenu::new();
        let out = m.handle_key(key(KeyCode::Enter));
        match out {
            DialogOutcome::Push(d) => assert_eq!(d.title(), "Serial port setup"),
            _ => panic!("expected Push"),
        }
    }

    #[test]
    fn enter_on_exit_closes_menu() {
        let mut m = RootMenu::new();
        for _ in 0..6 {
            m.handle_key(key(KeyCode::Down));
        }
        assert_eq!(m.selected(), 6);
        let out = m.handle_key(key(KeyCode::Enter));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn esc_closes() {
        let mut m = RootMenu::new();
        let out = m.handle_key(key(KeyCode::Esc));
        assert!(matches!(out, DialogOutcome::Close));
    }
}
