//! Generic yes/no confirmation dialog (skeleton — T15 GREEN implements
//! real behaviour).

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use crate::modal::{centred_rect, Dialog, DialogAction, DialogOutcome};

/// Default preferred width of a confirmation dialog, in terminal cells.
const PREFERRED_WIDTH: u16 = 50;
/// Default preferred height of a confirmation dialog, in terminal rows.
const PREFERRED_HEIGHT: u16 = 8;

/// Reusable yes/no confirmation dialog.
///
/// Constructed with a title, a prompt message, and the
/// [`DialogAction`] to emit on confirmation. The skeleton consumes
/// every key with no side effects; the T15 GREEN commit adds the real
/// y/N/Esc handling and rendering.
pub struct ConfirmDialog {
    title: String,
    #[allow(dead_code, reason = "used by T15 GREEN rendering")]
    prompt: String,
    #[allow(dead_code, reason = "emitted by T15 GREEN confirm path")]
    on_confirm: DialogAction,
    preferred_width: u16,
    preferred_height: u16,
}

impl ConfirmDialog {
    /// Construct a new confirmation dialog.
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

    fn render(&self, _area: Rect, _buf: &mut Buffer) {
        // T15 GREEN: draw title + prompt + "[Y]es [N]o" hint.
    }

    fn handle_key(&mut self, _key: KeyEvent) -> DialogOutcome {
        DialogOutcome::Consumed
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
