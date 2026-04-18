//! Serial-port setup dialog — the first real configuration sub-dialog.
//!
//! T12 red stage: scaffold only. Methods return defaults; the tests
//! below encode the target behaviour and intentionally fail.

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use rtcom_core::SerialConfig;

use crate::modal::{Dialog, DialogOutcome};

/// Serial port setup dialog (stub).
pub struct SerialPortSetupDialog {
    pending: SerialConfig,
}

impl SerialPortSetupDialog {
    /// Construct a dialog seeded with `initial_config`.
    #[must_use]
    pub const fn new(initial_config: SerialConfig) -> Self {
        Self {
            pending: initial_config,
        }
    }

    /// Current cursor position (stub always returns 0).
    #[must_use]
    pub const fn cursor(&self) -> usize {
        0
    }

    /// True while editing a numeric field (stub always false).
    #[must_use]
    pub const fn is_editing(&self) -> bool {
        false
    }

    /// The currently pending [`SerialConfig`].
    #[must_use]
    pub const fn pending(&self) -> &SerialConfig {
        &self.pending
    }
}

impl Dialog for SerialPortSetupDialog {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Serial port setup"
    }

    fn render(&self, _area: Rect, _buf: &mut Buffer) {}

    fn handle_key(&mut self, _key: KeyEvent) -> DialogOutcome {
        DialogOutcome::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rtcom_core::SerialConfig;

    use crate::modal::DialogAction;

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn default_dialog() -> SerialPortSetupDialog {
        SerialPortSetupDialog::new(SerialConfig::default())
    }

    #[test]
    fn dialog_starts_with_baud_field_selected() {
        let d = default_dialog();
        assert_eq!(d.cursor(), 0);
    }

    #[test]
    fn down_moves_field_cursor() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor(), 1);
    }

    #[test]
    fn cursor_reaches_apply_live_at_index_5() {
        let mut d = default_dialog();
        for _ in 0..5 {
            d.handle_key(key(KeyCode::Down));
        }
        assert_eq!(d.cursor(), 5);
    }

    #[test]
    fn esc_from_field_view_closes() {
        let mut d = default_dialog();
        let out = d.handle_key(key(KeyCode::Esc));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn enter_on_cancel_closes() {
        let mut d = default_dialog();
        for _ in 0..7 {
            d.handle_key(key(KeyCode::Down));
        }
        // cursor on Cancel (idx 7)
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(out, DialogOutcome::Close));
    }

    #[test]
    fn f2_emits_apply_live_with_current_pending() {
        let mut d = default_dialog();
        let out = d.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        match out {
            DialogOutcome::Action(DialogAction::ApplyLive(cfg)) => {
                assert_eq!(cfg, SerialConfig::default());
            }
            _ => panic!("expected Action(ApplyLive)"),
        }
    }

    #[test]
    fn f10_emits_apply_and_save() {
        let mut d = default_dialog();
        let out = d.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyAndSave(_))
        ));
    }

    #[test]
    fn enter_on_baud_enters_edit_mode() {
        let mut d = default_dialog();
        // cursor is on Baud (idx 0) by default
        d.handle_key(key(KeyCode::Enter));
        assert!(d.is_editing());
    }

    #[test]
    fn typing_digits_updates_pending_baud_on_commit() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Enter)); // enter edit mode
        d.handle_key(key(KeyCode::Char('9')));
        d.handle_key(key(KeyCode::Char('6')));
        d.handle_key(key(KeyCode::Char('0')));
        d.handle_key(key(KeyCode::Char('0')));
        d.handle_key(key(KeyCode::Enter)); // commit
        assert!(!d.is_editing());
        assert_eq!(d.pending().baud_rate, 9600);
    }

    #[test]
    fn esc_during_edit_cancels_and_preserves_pending() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Enter)); // enter edit mode on baud
        d.handle_key(key(KeyCode::Char('4'))); // typing '4'
        let before = d.pending().baud_rate;
        d.handle_key(key(KeyCode::Esc)); // cancel edit, return to field view
        assert!(!d.is_editing());
        assert_eq!(d.pending().baud_rate, before); // unchanged
    }

    #[test]
    fn enum_field_cycles_with_space() {
        let mut d = default_dialog();
        // move cursor to parity (idx 3)
        for _ in 0..3 {
            d.handle_key(key(KeyCode::Down));
        }
        let initial_parity = d.pending().parity;
        d.handle_key(key(KeyCode::Char(' '))); // cycle
        assert_ne!(d.pending().parity, initial_parity);
    }

    #[test]
    fn preferred_size_is_wider_than_default() {
        use ratatui::layout::Rect;
        let d = default_dialog();
        let outer = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let pref = d.preferred_size(outer);
        // Expect wider than the default 30x12
        assert!(pref.width >= 40, "expected >=40 cols, got {}", pref.width);
        assert!(pref.height >= 14, "expected >=14 rows, got {}", pref.height);
    }

    #[test]
    fn enter_on_parity_cycles_without_edit_mode() {
        let mut d = default_dialog();
        for _ in 0..3 {
            d.handle_key(key(KeyCode::Down));
        }
        let initial = d.pending().parity;
        d.handle_key(key(KeyCode::Enter));
        assert_ne!(d.pending().parity, initial);
        assert!(!d.is_editing());
    }

    #[test]
    fn up_wraps_to_last_action() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Up));
        assert_eq!(d.cursor(), 7);
    }

    #[test]
    fn down_wraps_from_last_to_first() {
        let mut d = default_dialog();
        for _ in 0..8 {
            d.handle_key(key(KeyCode::Down));
        }
        assert_eq!(d.cursor(), 0);
    }

    #[test]
    fn invalid_baud_commit_leaves_pending_unchanged() {
        let mut d = default_dialog();
        let before = d.pending().baud_rate;
        d.handle_key(key(KeyCode::Enter)); // edit
        d.handle_key(key(KeyCode::Enter)); // commit empty buffer
        assert_eq!(d.pending().baud_rate, before);
    }

    #[test]
    fn enter_on_apply_live_emits_action() {
        let mut d = default_dialog();
        for _ in 0..5 {
            d.handle_key(key(KeyCode::Down));
        }
        // cursor now on [Apply live]
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyLive(_))
        ));
    }

    #[test]
    fn pending_carries_edits_through_f2() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Enter)); // edit baud
        d.handle_key(key(KeyCode::Char('1')));
        d.handle_key(key(KeyCode::Char('9')));
        d.handle_key(key(KeyCode::Char('2')));
        d.handle_key(key(KeyCode::Char('0')));
        d.handle_key(key(KeyCode::Char('0')));
        // F2 commits the in-progress edit and emits Action.
        let out = d.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        match out {
            DialogOutcome::Action(DialogAction::ApplyLive(cfg)) => {
                assert_eq!(cfg.baud_rate, 19_200);
            }
            _ => panic!("expected ApplyLive"),
        }
    }
}
