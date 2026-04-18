//! Modem-control dialog — RED skeleton (T14).
//!
//! Stub surface so the T14 failing tests compile. Real behaviour
//! arrives in the follow-up GREEN commit.

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use rtcom_core::ModemLineSnapshot;

use crate::modal::{Dialog, DialogOutcome};

/// Immediate-action dialog for toggling DTR / RTS and sending a line
/// break. RED stub — every method is a placeholder that the GREEN
/// commit replaces with real logic.
pub struct ModemControlDialog {
    current: ModemLineSnapshot,
    cursor: usize,
}

impl ModemControlDialog {
    /// Construct a dialog displaying `current` as the read-only
    /// "current output lines" snapshot, cursor on the first action.
    #[must_use]
    pub const fn new(current: ModemLineSnapshot) -> Self {
        Self { current, cursor: 0 }
    }

    /// Current cursor position (always 0 in the RED stub).
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Read-only snapshot of the modem output lines as known to rtcom.
    #[must_use]
    pub const fn current_lines(&self) -> &ModemLineSnapshot {
        &self.current
    }
}

impl Dialog for ModemControlDialog {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Modem control"
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
    use rtcom_core::ModemLineSnapshot;

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn default_dialog() -> ModemControlDialog {
        ModemControlDialog::new(ModemLineSnapshot::default())
    }

    #[test]
    fn starts_with_raise_dtr_selected() {
        assert_eq!(default_dialog().cursor(), 0);
    }

    #[test]
    fn enter_raise_dtr_emits_set_dtr_true() {
        let mut d = default_dialog();
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::SetDtr(true))
        ));
    }

    #[test]
    fn enter_lower_dtr_emits_set_dtr_false() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Down));
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::SetDtr(false))
        ));
    }

    #[test]
    fn enter_raise_rts_emits_set_rts_true() {
        let mut d = default_dialog();
        for _ in 0..2 {
            d.handle_key(key(KeyCode::Down));
        }
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::SetRts(true))
        ));
    }

    #[test]
    fn enter_lower_rts_emits_set_rts_false() {
        let mut d = default_dialog();
        for _ in 0..3 {
            d.handle_key(key(KeyCode::Down));
        }
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::SetRts(false))
        ));
    }

    #[test]
    fn enter_send_break_emits_send_break() {
        let mut d = default_dialog();
        for _ in 0..4 {
            d.handle_key(key(KeyCode::Down));
        }
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::SendBreak)
        ));
    }

    #[test]
    fn enter_on_close_closes() {
        let mut d = default_dialog();
        for _ in 0..5 {
            d.handle_key(key(KeyCode::Down));
        }
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn esc_closes() {
        let mut d = default_dialog();
        let out = d.handle_key(key(KeyCode::Esc));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn cursor_wraps() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Up));
        assert_eq!(d.cursor(), 5);
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor(), 0);
    }

    #[test]
    fn preferred_size_40x18() {
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
        assert_eq!(pref.height, 18);
    }

    #[test]
    fn dialog_shows_current_dtr_rts_in_title_area() {
        // Just verify constructor stores the passed snapshot.
        let snap = ModemLineSnapshot {
            dtr: true,
            rts: false,
        };
        let d = ModemControlDialog::new(snap);
        assert!(d.current_lines().dtr);
        assert!(!d.current_lines().rts);
    }
}
