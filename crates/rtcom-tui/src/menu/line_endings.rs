//! Line-endings dialog — edits the three [`LineEnding`]
//! rules that govern a session's byte streams.
//!
//! Structurally the simpler cousin of T12's
//! [`SerialPortSetupDialog`](crate::menu::SerialPortSetupDialog): three
//! enum fields (`omap` / `imap` / `emap`) plus three action buttons
//! (`Apply live` / `Apply + Save` / `Cancel`) — six cursor positions
//! total. Because every field is an enum, cycling is immediate
//! (`Space` or `Enter` on a field advances to the next variant) and
//! there is no numeric-edit state machine.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

use rtcom_core::{LineEnding, LineEndingConfig};

use crate::modal::{centred_rect, Dialog, DialogAction, DialogOutcome};

/// Index of the `omap` (outbound) field row.
const FIELD_OMAP: usize = 0;
/// Index of the `imap` (inbound) field row.
const FIELD_IMAP: usize = 1;
/// Index of the `emap` (echo) field row.
const FIELD_EMAP: usize = 2;

/// Index of the `[Apply live]` action button.
const ACTION_APPLY_LIVE: usize = 3;
/// Index of the `[Apply + Save]` action button.
const ACTION_APPLY_SAVE: usize = 4;
/// Index of the `[Cancel]` action button.
const ACTION_CANCEL: usize = 5;

/// Total cursor slots (3 fields + 3 actions).
const CURSOR_MAX: usize = 6;

/// Line-endings dialog.
///
/// Holds a snapshot of the initial [`LineEndingConfig`] and a mutable
/// `pending` copy that tracks the user's edits. Emits
/// [`DialogAction::ApplyLineEndingsLive`] on `F2` / `Enter` on
/// `[Apply live]`, [`DialogAction::ApplyLineEndingsAndSave`] on `F10`
/// / `Enter` on `[Apply + Save]`, and [`DialogOutcome::Close`] on
/// `Esc` / `Enter` on `[Cancel]`.
///
/// After emitting an `Action`, the dialog stays open — T17 wires the
/// outer `TuiApp` to pop the stack once the action has been applied.
pub struct LineEndingsDialog {
    #[allow(dead_code, reason = "reserved for T17 revert-on-cancel path")]
    initial: LineEndingConfig,
    pending: LineEndingConfig,
    cursor: usize,
}

impl LineEndingsDialog {
    /// Construct a dialog seeded with `initial_config`. The cursor
    /// starts on the `omap` row.
    #[must_use]
    pub const fn new(initial_config: LineEndingConfig) -> Self {
        Self {
            initial: initial_config,
            pending: initial_config,
            cursor: FIELD_OMAP,
        }
    }

    /// Current cursor position. Valid range is `0..6`: indices `0..=2`
    /// select a mapper field (omap / imap / emap), and `3..=5` select
    /// one of the action buttons (Apply live / Apply + Save / Cancel).
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// The currently pending [`LineEndingConfig`]; reflects every
    /// committed edit since construction.
    #[must_use]
    pub const fn pending(&self) -> &LineEndingConfig {
        &self.pending
    }

    /// Move the cursor up one row (wraps).
    fn move_up(&mut self) {
        self.cursor = if self.cursor == 0 {
            CURSOR_MAX - 1
        } else {
            self.cursor - 1
        };
    }

    /// Move the cursor down one row (wraps).
    fn move_down(&mut self) {
        self.cursor = (self.cursor + 1) % CURSOR_MAX;
    }

    /// Cycle the enum value at the current field cursor.
    /// No-op when the cursor is on an action button.
    fn cycle_current_field(&mut self) {
        match self.cursor {
            FIELD_OMAP => self.pending.omap = cycle_line_ending(self.pending.omap),
            FIELD_IMAP => self.pending.imap = cycle_line_ending(self.pending.imap),
            FIELD_EMAP => self.pending.emap = cycle_line_ending(self.pending.emap),
            _ => {}
        }
    }

    /// Handle `Enter` in field-navigation mode.
    fn activate(&mut self) -> DialogOutcome {
        match self.cursor {
            FIELD_OMAP | FIELD_IMAP | FIELD_EMAP => {
                self.cycle_current_field();
                DialogOutcome::Consumed
            }
            ACTION_APPLY_LIVE => {
                DialogOutcome::Action(DialogAction::ApplyLineEndingsLive(self.pending))
            }
            ACTION_APPLY_SAVE => {
                DialogOutcome::Action(DialogAction::ApplyLineEndingsAndSave(self.pending))
            }
            ACTION_CANCEL => DialogOutcome::Close,
            _ => DialogOutcome::Consumed,
        }
    }

    /// Build the rendered field row for `cursor == field_idx`.
    fn field_line(&self, field_idx: usize, label: &'static str, value: LineEnding) -> Line<'_> {
        let selected = self.cursor == field_idx;
        let prefix = if selected { "> " } else { "  " };
        let text = format!("{prefix}{label:<6} {}", line_ending_label(value));
        if selected {
            Line::from(Span::styled(
                text,
                Style::default().add_modifier(Modifier::REVERSED),
            ))
        } else {
            Line::from(Span::raw(text))
        }
    }

    /// Build the rendered action-button row for `cursor == action_idx`.
    fn action_line(
        &self,
        action_idx: usize,
        label: &'static str,
        shortcut: &'static str,
    ) -> Line<'_> {
        let selected = self.cursor == action_idx;
        let prefix = if selected { "> " } else { "  " };
        let text = format!("{prefix}{label:<18} {shortcut}");
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

impl Dialog for LineEndingsDialog {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Line endings"
    }

    fn preferred_size(&self, outer: Rect) -> Rect {
        centred_rect(outer, 40, 14)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Line endings");
        let inner = block.inner(area);
        block.render(area, buf);

        let cfg = &self.pending;
        let sep_width = usize::from(inner.width);
        let sep_line = Line::from(Span::styled(
            "-".repeat(sep_width),
            Style::default().add_modifier(Modifier::DIM),
        ));

        let lines = vec![
            Line::from(Span::raw("")),
            self.field_line(FIELD_OMAP, "OMAP", cfg.omap),
            self.field_line(FIELD_IMAP, "IMAP", cfg.imap),
            self.field_line(FIELD_EMAP, "EMAP", cfg.emap),
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
                return DialogOutcome::Action(DialogAction::ApplyLineEndingsLive(self.pending));
            }
            KeyCode::F(10) => {
                return DialogOutcome::Action(DialogAction::ApplyLineEndingsAndSave(self.pending));
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
            KeyCode::Char(' ') => {
                self.cycle_current_field();
                DialogOutcome::Consumed
            }
            _ => DialogOutcome::Consumed,
        }
    }
}

/// Next [`LineEnding`] in the canonical cycle order (wraps).
///
/// Order chosen to match the declaration order of the enum so the cycle
/// is predictable to read: `None` → `AddCrToLf` → `AddLfToCr` →
/// `DropCr` → `DropLf` → `None`.
const fn cycle_line_ending(le: LineEnding) -> LineEnding {
    match le {
        LineEnding::None => LineEnding::AddCrToLf,
        LineEnding::AddCrToLf => LineEnding::AddLfToCr,
        LineEnding::AddLfToCr => LineEnding::DropCr,
        LineEnding::DropCr => LineEnding::DropLf,
        LineEnding::DropLf => LineEnding::None,
    }
}

/// Human-readable label for a [`LineEnding`].
const fn line_ending_label(le: LineEnding) -> &'static str {
    match le {
        LineEnding::None => "none",
        LineEnding::AddCrToLf => "crlf",
        LineEnding::AddLfToCr => "lfcr",
        LineEnding::DropCr => "igncr",
        LineEnding::DropLf => "ignlf",
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
    fn cycle_order_covers_every_variant() {
        // 5 variants × cycle once each returns to start.
        let mut le = LineEnding::None;
        for _ in 0..5 {
            le = cycle_line_ending(le);
        }
        assert_eq!(le, LineEnding::None);
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
