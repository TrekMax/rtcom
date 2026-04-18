//! Line-endings dialog — RED skeleton (T13).
//!
//! Stub surface so the T13 failing tests compile. Real behaviour
//! arrives in the follow-up GREEN commit.

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use rtcom_core::LineEndingConfig;

use crate::modal::{Dialog, DialogOutcome};

/// Line-endings dialog. RED stub — every method is a placeholder
/// that the GREEN commit replaces with real logic.
pub struct LineEndingsDialog {
    #[allow(dead_code, reason = "GREEN commit wires the pending copy")]
    initial: LineEndingConfig,
    pending: LineEndingConfig,
    cursor: usize,
}

impl LineEndingsDialog {
    /// Construct a dialog seeded with `initial_config`.
    #[must_use]
    pub const fn new(initial_config: LineEndingConfig) -> Self {
        Self {
            initial: initial_config,
            pending: initial_config,
            cursor: 0,
        }
    }

    /// Current cursor position (always 0 in the RED stub).
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Currently pending config.
    #[must_use]
    pub const fn pending(&self) -> &LineEndingConfig {
        &self.pending
    }
}

impl Dialog for LineEndingsDialog {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Line endings"
    }

    fn render(&self, _area: Rect, _buf: &mut Buffer) {
        // RED stub renders nothing.
    }

    fn handle_key(&mut self, _key: KeyEvent) -> DialogOutcome {
        // RED stub consumes everything without side effects.
        DialogOutcome::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modal::DialogAction;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rtcom_core::{LineEnding, LineEndingConfig};

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn default_dialog() -> LineEndingsDialog {
        LineEndingsDialog::new(LineEndingConfig::default())
    }

    #[test]
    fn starts_on_omap() {
        let d = default_dialog();
        assert_eq!(d.cursor(), 0);
    }

    #[test]
    fn cursor_span_is_six() {
        let mut d = default_dialog();
        for _ in 0..5 {
            d.handle_key(key(KeyCode::Down));
        }
        assert_eq!(d.cursor(), 5);
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor(), 0); // wrap
    }

    #[test]
    fn space_cycles_current_field() {
        let mut d = default_dialog();
        let before = d.pending().omap;
        d.handle_key(key(KeyCode::Char(' ')));
        assert_ne!(d.pending().omap, before);
    }

    #[test]
    fn enter_on_field_cycles() {
        let mut d = default_dialog();
        let before = d.pending().omap;
        d.handle_key(key(KeyCode::Enter));
        assert_ne!(d.pending().omap, before);
    }

    #[test]
    fn f2_emits_apply_line_endings_live() {
        let mut d = default_dialog();
        let out = d.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyLineEndingsLive(_))
        ));
    }

    #[test]
    fn f10_emits_apply_line_endings_and_save() {
        let mut d = default_dialog();
        let out = d.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyLineEndingsAndSave(_))
        ));
    }

    #[test]
    fn esc_closes() {
        let mut d = default_dialog();
        let out = d.handle_key(key(KeyCode::Esc));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn enter_on_apply_live_button_emits_action() {
        let mut d = default_dialog();
        for _ in 0..3 {
            d.handle_key(key(KeyCode::Down));
        } // cursor=3 -> Apply live button
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyLineEndingsLive(_))
        ));
    }

    #[test]
    fn enter_on_cancel_closes() {
        let mut d = default_dialog();
        for _ in 0..5 {
            d.handle_key(key(KeyCode::Down));
        } // cursor=5 -> Cancel
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn j_k_nav() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Char('j')));
        assert_eq!(d.cursor(), 1);
        d.handle_key(key(KeyCode::Char('k')));
        assert_eq!(d.cursor(), 0);
    }

    #[test]
    fn preferred_size_40x14() {
        use ratatui::layout::Rect;
        let d = default_dialog();
        let outer = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let pref = d.preferred_size(outer);
        assert_eq!(pref.width, 40);
        assert_eq!(pref.height, 14);
    }

    #[test]
    fn cycling_imap_does_not_touch_omap_or_emap() {
        let mut d = default_dialog();
        // Move cursor to IMAP (idx 1).
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor(), 1);
        d.handle_key(key(KeyCode::Char(' ')));
        assert_ne!(d.pending().imap, LineEnding::None);
        assert_eq!(d.pending().omap, LineEnding::None);
        assert_eq!(d.pending().emap, LineEnding::None);
    }
}
