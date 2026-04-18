//! Screen-options dialog — edits the TUI's modal-render style.
//!
//! Three radio options (Overlay / Dimmed overlay / Fullscreen) plus
//! three action buttons (Apply live / Apply + Save / Cancel) — six
//! cursor positions total. `scrollback_rows` is shown as a read-only
//! display for v0.2; full editing arrives post-v1.0.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

use rtcom_config::ModalStyle;

use crate::modal::{centred_rect, Dialog, DialogAction, DialogOutcome};

/// Index of the `Overlay` radio row.
const RADIO_OVERLAY: usize = 0;
/// Index of the `Dimmed overlay` radio row.
const RADIO_DIMMED_OVERLAY: usize = 1;
/// Index of the `Fullscreen` radio row.
const RADIO_FULLSCREEN: usize = 2;
/// Index of the `[Apply live]` action button.
const ACTION_APPLY_LIVE: usize = 3;
/// Index of the `[Apply + Save]` action button.
const ACTION_APPLY_SAVE: usize = 4;
/// Index of the `[Cancel]` action button.
const ACTION_CANCEL: usize = 5;

/// Total cursor slots (3 radios + 3 actions).
const CURSOR_MAX: usize = 6;

/// Scrollback rows display value — fixed at 10 000 for v0.2. Made
/// editable post-v1.0 when the TUI backing buffer grows the knob.
const SCROLLBACK_ROWS_DISPLAY: &str = "10000";

/// Screen-options dialog.
///
/// Holds a snapshot of the initial [`ModalStyle`] and a mutable
/// `pending` copy that tracks the user's radio selection. Emits
/// [`DialogAction::ApplyModalStyleLive`] on `F2` / `Enter` on
/// `[Apply live]`, [`DialogAction::ApplyModalStyleAndSave`] on `F10`
/// / `Enter` on `[Apply + Save]`, and [`DialogOutcome::Close`] on
/// `Esc` / `Enter` on `[Cancel]`. Pressing `Enter` on a radio row
/// sets `pending` to that option without moving the cursor.
pub struct ScreenOptionsDialog {
    #[allow(dead_code, reason = "reserved for T17 revert-on-cancel path")]
    initial: ModalStyle,
    pending: ModalStyle,
    cursor: usize,
}

impl ScreenOptionsDialog {
    /// Construct a dialog seeded with the given initial [`ModalStyle`].
    /// The cursor starts on the first radio option.
    #[must_use]
    pub const fn new(initial: ModalStyle) -> Self {
        Self {
            initial,
            pending: initial,
            cursor: RADIO_OVERLAY,
        }
    }

    /// Current cursor position. Valid range is `0..6`: `0..=2` select
    /// a radio option, `3..=5` select an action button.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// The currently pending [`ModalStyle`] — what will be emitted by
    /// the next `Apply live` / `Apply + Save` action.
    #[must_use]
    pub const fn pending(&self) -> ModalStyle {
        self.pending
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

    /// Handle `Enter` over the current cursor position.
    const fn activate(&mut self) -> DialogOutcome {
        match self.cursor {
            RADIO_OVERLAY => {
                self.pending = ModalStyle::Overlay;
                DialogOutcome::Consumed
            }
            RADIO_DIMMED_OVERLAY => {
                self.pending = ModalStyle::DimmedOverlay;
                DialogOutcome::Consumed
            }
            RADIO_FULLSCREEN => {
                self.pending = ModalStyle::Fullscreen;
                DialogOutcome::Consumed
            }
            ACTION_APPLY_LIVE => {
                DialogOutcome::Action(DialogAction::ApplyModalStyleLive(self.pending))
            }
            ACTION_APPLY_SAVE => {
                DialogOutcome::Action(DialogAction::ApplyModalStyleAndSave(self.pending))
            }
            ACTION_CANCEL => DialogOutcome::Close,
            _ => DialogOutcome::Consumed,
        }
    }

    /// Build a radio row for the given cursor slot.
    fn radio_line(&self, slot: usize, label: &'static str, style_for_slot: ModalStyle) -> Line<'_> {
        let selected = self.cursor == slot;
        let marker = if self.pending == style_for_slot {
            "(*)"
        } else {
            "( )"
        };
        let prefix = if selected { "> " } else { "  " };
        let text = format!("  {prefix}{marker} {label}");
        if selected {
            Line::from(Span::styled(
                text,
                Style::default().add_modifier(Modifier::REVERSED),
            ))
        } else {
            Line::from(Span::raw(text))
        }
    }

    /// Build an action-button row for the given cursor slot.
    fn action_line(&self, slot: usize, label: &'static str, shortcut: &'static str) -> Line<'_> {
        let selected = self.cursor == slot;
        let prefix = if selected { "> " } else { "  " };
        let text = format!("  {prefix}{label:<18} {shortcut}");
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

impl Dialog for ScreenOptionsDialog {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Screen options"
    }

    fn preferred_size(&self, outer: Rect) -> Rect {
        centred_rect(outer, 40, 16)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Screen options");
        let inner = block.inner(area);
        block.render(area, buf);

        let sep_width = usize::from(inner.width);
        let sep_line = Line::from(Span::styled(
            "-".repeat(sep_width),
            Style::default().add_modifier(Modifier::DIM),
        ));

        let lines = vec![
            Line::from(Span::raw("")),
            Line::from(Span::raw("  Modal style:")),
            self.radio_line(RADIO_OVERLAY, "Overlay", ModalStyle::Overlay),
            self.radio_line(
                RADIO_DIMMED_OVERLAY,
                "Dimmed overlay",
                ModalStyle::DimmedOverlay,
            ),
            self.radio_line(RADIO_FULLSCREEN, "Fullscreen", ModalStyle::Fullscreen),
            Line::from(Span::raw("")),
            Line::from(Span::raw(format!(
                "  Scrollback rows:  {SCROLLBACK_ROWS_DISPLAY}"
            ))),
            Line::from(Span::raw("")),
            sep_line,
            Line::from(Span::raw("")),
            self.action_line(ACTION_APPLY_LIVE, "[Apply live]", "(F2)"),
            self.action_line(ACTION_APPLY_SAVE, "[Apply + Save]", "(F10)"),
            self.action_line(ACTION_CANCEL, "[Cancel]", "(Esc)"),
        ];

        Paragraph::new(lines).render(inner, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
        // F2 / F10 act as global "apply now" shortcuts regardless of
        // cursor position.
        match key.code {
            KeyCode::F(2) => {
                return DialogOutcome::Action(DialogAction::ApplyModalStyleLive(self.pending));
            }
            KeyCode::F(10) => {
                return DialogOutcome::Action(DialogAction::ApplyModalStyleAndSave(self.pending));
            }
            _ => {}
        }

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

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    const fn default_dialog() -> ScreenOptionsDialog {
        ScreenOptionsDialog::new(ModalStyle::Overlay)
    }

    #[test]
    fn starts_at_overlay_radio() {
        let d = default_dialog();
        assert_eq!(d.cursor(), RADIO_OVERLAY);
        assert_eq!(d.pending(), ModalStyle::Overlay);
    }

    #[test]
    fn down_moves_through_six_slots() {
        let mut d = default_dialog();
        for _ in 0..5 {
            d.handle_key(key(KeyCode::Down));
        }
        assert_eq!(d.cursor(), 5);
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor(), 0); // wrap
    }

    #[test]
    fn enter_on_dimmed_radio_sets_pending() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Down)); // cursor=1 dimmed
        d.handle_key(key(KeyCode::Enter));
        assert_eq!(d.pending(), ModalStyle::DimmedOverlay);
        // cursor does not move
        assert_eq!(d.cursor(), RADIO_DIMMED_OVERLAY);
    }

    #[test]
    fn enter_on_fullscreen_radio_sets_pending() {
        let mut d = default_dialog();
        d.handle_key(key(KeyCode::Down));
        d.handle_key(key(KeyCode::Down)); // cursor=2 fullscreen
        d.handle_key(key(KeyCode::Enter));
        assert_eq!(d.pending(), ModalStyle::Fullscreen);
        assert_eq!(d.cursor(), RADIO_FULLSCREEN);
    }

    #[test]
    fn f2_emits_apply_modal_style_live() {
        let mut d = default_dialog();
        // Change pending to DimmedOverlay first.
        d.handle_key(key(KeyCode::Down));
        d.handle_key(key(KeyCode::Enter));
        let out = d.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyModalStyleLive(ModalStyle::DimmedOverlay))
        ));
    }

    #[test]
    fn f10_emits_apply_modal_style_and_save() {
        let mut d = default_dialog();
        let out = d.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyModalStyleAndSave(_))
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
        for _ in 0..ACTION_APPLY_LIVE {
            d.handle_key(key(KeyCode::Down));
        }
        assert_eq!(d.cursor(), ACTION_APPLY_LIVE);
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyModalStyleLive(_))
        ));
    }

    #[test]
    fn enter_on_apply_save_button_emits_action() {
        let mut d = default_dialog();
        for _ in 0..ACTION_APPLY_SAVE {
            d.handle_key(key(KeyCode::Down));
        }
        assert_eq!(d.cursor(), ACTION_APPLY_SAVE);
        let out = d.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            out,
            DialogOutcome::Action(DialogAction::ApplyModalStyleAndSave(_))
        ));
    }

    #[test]
    fn enter_on_cancel_closes() {
        let mut d = default_dialog();
        for _ in 0..ACTION_CANCEL {
            d.handle_key(key(KeyCode::Down));
        }
        assert_eq!(d.cursor(), ACTION_CANCEL);
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
    fn preferred_size_40x16() {
        let d = default_dialog();
        let outer = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let pref = d.preferred_size(outer);
        assert_eq!(pref.width, 40);
        assert_eq!(pref.height, 16);
    }

    #[test]
    fn cursor_max_is_six() {
        assert_eq!(CURSOR_MAX, 6);
    }
}
