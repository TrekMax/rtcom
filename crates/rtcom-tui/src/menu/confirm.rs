//! Generic yes/no confirmation dialog.
//!
//! [`ConfirmDialog`] is a reusable two-button dialog that emits a
//! caller-supplied [`DialogAction`] when the user confirms (`y` / `Y`
//! / `Enter`) and closes without action on `n` / `N` / `Esc`. It is
//! used by the root menu for the "Write profile" and "Read profile"
//! rows (T15) and can be reused by any future flow that needs a
//! single-shot confirmation prompt.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

use crate::modal::{centred_rect, Dialog, DialogAction, DialogOutcome};

/// Default preferred width of a confirmation dialog, in terminal cells.
const PREFERRED_WIDTH: u16 = 50;
/// Default preferred height of a confirmation dialog, in terminal rows.
const PREFERRED_HEIGHT: u16 = 8;

/// Reusable yes/no confirmation dialog.
///
/// Constructed with a title, a prompt message, and the
/// [`DialogAction`] to emit on confirmation. Emits
/// [`DialogOutcome::Action`] on `y` / `Y` / `Enter` and
/// [`DialogOutcome::Close`] on `n` / `N` / `Esc`; every other key is
/// swallowed with [`DialogOutcome::Consumed`].
pub struct ConfirmDialog {
    title: String,
    prompt: String,
    on_confirm: DialogAction,
    preferred_width: u16,
    preferred_height: u16,
}

impl ConfirmDialog {
    /// Construct a new confirmation dialog.
    ///
    /// `title` is used both as the window title and as the widget
    /// title; `prompt` is rendered as the body text; `on_confirm` is
    /// the [`DialogAction`] emitted when the user confirms.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        prompt: impl Into<String>,
        on_confirm: DialogAction,
    ) -> Self {
        Self {
            title: title.into(),
            prompt: prompt.into(),
            on_confirm,
            preferred_width: PREFERRED_WIDTH,
            preferred_height: PREFERRED_HEIGHT,
        }
    }
}

impl Dialog for ConfirmDialog {
    fn title(&self) -> &str {
        self.title.as_str()
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title(self.title.as_str());
        let inner = block.inner(area);
        block.render(area, buf);
        let lines = vec![
            Line::from(self.prompt.as_str()),
            Line::from(""),
            Line::from(Span::styled(
                "  [Y]es   [N]o / Esc to cancel  ",
                Style::default().add_modifier(Modifier::DIM),
            )),
        ];
        Paragraph::new(lines).render(inner, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                DialogOutcome::Action(self.on_confirm.clone())
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => DialogOutcome::Close,
            _ => DialogOutcome::Consumed,
        }
    }

    fn preferred_size(&self, outer: Rect) -> Rect {
        centred_rect(outer, self.preferred_width, self.preferred_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn dialog() -> ConfirmDialog {
        ConfirmDialog::new("Title", "Are you sure?", DialogAction::WriteProfile)
    }

    #[test]
    fn lowercase_y_confirms() {
        let mut d = dialog();
        let out = d.handle_key(key(KeyCode::Char('y')));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::WriteProfile)
        ));
    }

    #[test]
    fn uppercase_y_confirms() {
        let mut d = dialog();
        let out = d.handle_key(key(KeyCode::Char('Y')));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::WriteProfile)
        ));
    }

    #[test]
    fn enter_confirms() {
        let mut d = dialog();
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::WriteProfile)
        ));
    }

    #[test]
    fn lowercase_n_cancels() {
        let mut d = dialog();
        let out = d.handle_key(key(KeyCode::Char('n')));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn uppercase_n_cancels() {
        let mut d = dialog();
        let out = d.handle_key(key(KeyCode::Char('N')));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn esc_cancels() {
        let mut d = dialog();
        let out = d.handle_key(key(KeyCode::Esc));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn other_key_consumed() {
        let mut d = dialog();
        let out = d.handle_key(key(KeyCode::Char('x')));
        assert!(matches!(out, DialogOutcome::Consumed));
    }

    #[test]
    fn title_round_trips() {
        let d = ConfirmDialog::new("Write profile", "prompt", DialogAction::WriteProfile);
        assert_eq!(d.title(), "Write profile");
    }

    #[test]
    fn preferred_size_50x8_centred() {
        let d = dialog();
        let outer = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let pref = d.preferred_size(outer);
        assert_eq!(pref.width, 50);
        assert_eq!(pref.height, 8);
    }
}
