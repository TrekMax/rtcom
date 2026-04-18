//! Dummy dialog used while the real sub-menu dialogs are unimplemented.
//!
//! Renders a bordered box titled with whatever the parent menu passed
//! in and closes on Esc. Tasks T12–T15 replace every use of this with
//! a real dialog.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, Paragraph, Widget},
};

use crate::modal::{Dialog, DialogOutcome};

/// Minimal placeholder dialog. Shows `TODO: <title>` and closes on Esc.
pub struct PlaceholderDialog {
    title: String,
}

impl PlaceholderDialog {
    /// Create a new placeholder labelled `title`.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }
}

impl Dialog for PlaceholderDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title(self.title.as_str());
        let para = Paragraph::new(format!("TODO: {}", self.title)).block(block);
        para.render(area, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
        if key.code == KeyCode::Esc {
            DialogOutcome::Close
        } else {
            DialogOutcome::Consumed
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

    #[test]
    fn placeholder_title_round_trips() {
        let d = PlaceholderDialog::new("Line endings");
        assert_eq!(d.title(), "Line endings");
    }

    #[test]
    fn placeholder_esc_closes() {
        let mut d = PlaceholderDialog::new("x");
        let out = d.handle_key(key(KeyCode::Esc));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn placeholder_other_keys_consumed() {
        let mut d = PlaceholderDialog::new("x");
        let out = d.handle_key(key(KeyCode::Char('a')));
        assert!(matches!(out, DialogOutcome::Consumed));
    }
}
