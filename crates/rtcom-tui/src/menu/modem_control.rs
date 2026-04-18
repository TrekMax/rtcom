//! Modem-control dialog — immediate-action menu for toggling the
//! DTR / RTS output lines and sending a line break.
//!
//! Unlike the [`SerialPortSetupDialog`](crate::menu::SerialPortSetupDialog)
//! (T12) and [`LineEndingsDialog`](crate::menu::LineEndingsDialog) (T13),
//! this dialog does not edit a pending configuration struct: every row
//! fires an action the moment the user presses `Enter`. The read-only
//! "current output lines" display at the top is seeded from whatever
//! [`ModemLineSnapshot`] the outer app passes in at construction time
//! and is not refreshed while the dialog is open (v0.2 scope limit —
//! proper live polling is follow-up work).
//!
//! The dialog stays open after an action: this matches minicom's
//! modem-control menu behaviour and lets the user fire several actions
//! in a row without re-opening. `Esc` / `Enter` on `[Close]` dismisses.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

use rtcom_core::ModemLineSnapshot;

use crate::modal::{centred_rect, Dialog, DialogAction, DialogOutcome};

/// Index of the `Raise DTR` action row.
const ACTION_RAISE_DTR: usize = 0;
/// Index of the `Lower DTR` action row.
const ACTION_LOWER_DTR: usize = 1;
/// Index of the `Raise RTS` action row.
const ACTION_RAISE_RTS: usize = 2;
/// Index of the `Lower RTS` action row.
const ACTION_LOWER_RTS: usize = 3;
/// Index of the `Send break (250 ms)` action row.
const ACTION_SEND_BREAK: usize = 4;
/// Index of the `[Close]` row.
const ACTION_CLOSE: usize = 5;

/// Total cursor slots (5 actions + close).
const CURSOR_MAX: usize = 6;

/// Immediate-action dialog for toggling DTR / RTS and sending a line
/// break.
///
/// Stores a read-only [`ModemLineSnapshot`] for the header display and
/// an integer cursor covering six action rows. Emits
/// [`DialogAction::SetDtr`] / [`DialogAction::SetRts`] /
/// [`DialogAction::SendBreak`] on `Enter` over an action row; emits
/// [`DialogOutcome::Close`] on `Esc` or `Enter` over `[Close]`.
pub struct ModemControlDialog {
    current: ModemLineSnapshot,
    cursor: usize,
}

impl ModemControlDialog {
    /// Construct a dialog displaying `current` as the read-only
    /// "current output lines" snapshot, cursor on the first action
    /// (`Raise DTR`).
    #[must_use]
    pub const fn new(current: ModemLineSnapshot) -> Self {
        Self {
            current,
            cursor: ACTION_RAISE_DTR,
        }
    }

    /// Current cursor position. Valid range is `0..6`: `0..=4` select
    /// an action row, `5` selects the `[Close]` button.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Read-only snapshot of the modem output lines as known to rtcom
    /// at the time this dialog was constructed.
    #[must_use]
    pub const fn current_lines(&self) -> &ModemLineSnapshot {
        &self.current
    }

    /// Move the cursor up one row (wraps).
    const fn move_up(&mut self) {
        self.cursor = if self.cursor == 0 {
            CURSOR_MAX - 1
        } else {
            self.cursor - 1
        };
    }

    /// Move the cursor down one row (wraps).
    const fn move_down(&mut self) {
        self.cursor = (self.cursor + 1) % CURSOR_MAX;
    }

    /// Handle `Enter` by dispatching the action under the cursor.
    const fn activate(&self) -> DialogOutcome {
        match self.cursor {
            ACTION_RAISE_DTR => DialogOutcome::Action(DialogAction::SetDtr(true)),
            ACTION_LOWER_DTR => DialogOutcome::Action(DialogAction::SetDtr(false)),
            ACTION_RAISE_RTS => DialogOutcome::Action(DialogAction::SetRts(true)),
            ACTION_LOWER_RTS => DialogOutcome::Action(DialogAction::SetRts(false)),
            ACTION_SEND_BREAK => DialogOutcome::Action(DialogAction::SendBreak),
            ACTION_CLOSE => DialogOutcome::Close,
            _ => DialogOutcome::Consumed,
        }
    }

    /// Build the rendered row for an action, applying the reversed
    /// style when the cursor is on it.
    fn action_line(&self, idx: usize, label: &'static str) -> Line<'_> {
        let selected = self.cursor == idx;
        let prefix = if selected { "> " } else { "  " };
        let text = format!("{prefix}{label}");
        if selected {
            Line::from(Span::styled(
                text,
                Style::default().add_modifier(Modifier::REVERSED),
            ))
        } else {
            Line::from(Span::raw(text))
        }
    }

    /// Build the rendered row for the `[Close]` button.
    fn close_line(&self) -> Line<'_> {
        let selected = self.cursor == ACTION_CLOSE;
        let prefix = if selected { "> " } else { "  " };
        let text = format!("{prefix}{:<18} {}", "[Close]", "(Esc)");
        if selected {
            Line::from(Span::styled(
                text,
                Style::default().add_modifier(Modifier::REVERSED),
            ))
        } else {
            Line::from(Span::raw(text))
        }
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

    fn preferred_size(&self, outer: Rect) -> Rect {
        centred_rect(outer, 40, 18)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Modem control");
        let inner = block.inner(area);
        block.render(area, buf);

        let sep_width = usize::from(inner.width);
        let sep_line = Line::from(Span::styled(
            "-".repeat(sep_width),
            Style::default().add_modifier(Modifier::DIM),
        ));

        let dtr_mark = if self.current.dtr { "*" } else { "o" };
        let rts_mark = if self.current.rts { "*" } else { "o" };

        let lines = vec![
            Line::from(Span::raw("")),
            Line::from(Span::raw("  Current output lines:")),
            Line::from(Span::raw(format!("    DTR: {dtr_mark}"))),
            Line::from(Span::raw(format!("    RTS: {rts_mark}"))),
            Line::from(Span::raw("")),
            sep_line,
            Line::from(Span::raw("")),
            self.action_line(ACTION_RAISE_DTR, "Raise DTR"),
            self.action_line(ACTION_LOWER_DTR, "Lower DTR"),
            self.action_line(ACTION_RAISE_RTS, "Raise RTS"),
            self.action_line(ACTION_LOWER_RTS, "Lower RTS"),
            self.action_line(ACTION_SEND_BREAK, "Send break (250 ms)"),
            Line::from(Span::raw("")),
            self.close_line(),
        ];

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
