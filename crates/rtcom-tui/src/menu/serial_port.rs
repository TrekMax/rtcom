//! Serial-port setup dialog — the first real configuration sub-dialog.
//!
//! Lets the user edit the five link parameters of a [`SerialConfig`]
//! (baud / data bits / stop bits / parity / flow control) and either
//! apply them to the live session (`F2`), apply + persist to profile
//! (`F10`), or cancel. Pushed by [`crate::menu::RootMenu`] when the
//! user selects "Serial port setup".
//!
//! T12 focuses on the state machine + outcomes; the visual polish
//! (inline edit cursor, per-field validation toasts) arrives in T22.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

use rtcom_core::{DataBits, FlowControl, Parity, SerialConfig, StopBits};

use crate::modal::{centred_rect, Dialog, DialogAction, DialogOutcome};

/// Index of the first field row (baud rate).
const FIELD_BAUD: usize = 0;
/// Index of the data-bits field row.
const FIELD_DATA_BITS: usize = 1;
/// Index of the stop-bits field row.
const FIELD_STOP_BITS: usize = 2;
/// Index of the parity field row.
const FIELD_PARITY: usize = 3;
/// Index of the flow-control field row.
const FIELD_FLOW: usize = 4;

/// Index of the `[Apply live]` action button.
const ACTION_APPLY_LIVE: usize = 5;
/// Index of the `[Apply + Save]` action button.
const ACTION_APPLY_SAVE: usize = 6;
/// Index of the `[Cancel]` action button.
const ACTION_CANCEL: usize = 7;

/// Total cursor slots (5 fields + 3 actions).
const CURSOR_MAX: usize = 8;

/// Edit mode for the dialog.
///
/// When [`EditState::Idle`] the dialog is navigating fields. When
/// [`EditState::EditingNumeric`] the user is typing digits into a
/// numeric field; `Enter` commits, `Esc` cancels.
#[derive(Debug, Clone)]
enum EditState {
    /// Not editing — arrow / vim keys move the field cursor.
    Idle,
    /// Typing digits into the numeric field identified by the dialog's
    /// current `cursor` position. The buffer holds the raw keystrokes;
    /// parsing happens on commit.
    EditingNumeric(String),
}

/// Serial port setup dialog.
///
/// Holds a snapshot of the initial [`SerialConfig`] and a mutable
/// `pending` copy that tracks the user's edits. Emits
/// [`DialogAction::ApplyLive`] on `F2` / `Enter` on `[Apply live]`,
/// [`DialogAction::ApplyAndSave`] on `F10` / `Enter` on
/// `[Apply + Save]`, and [`DialogOutcome::Close`] on `Esc` / `Enter`
/// on `[Cancel]`.
///
/// After emitting an `Action`, the dialog stays open — T17 wires the
/// outer `TuiApp` to pop the stack once the action has been applied.
pub struct SerialPortSetupDialog {
    #[allow(dead_code, reason = "reserved for T17 revert-on-cancel path")]
    initial: SerialConfig,
    pending: SerialConfig,
    cursor: usize,
    edit_state: EditState,
    /// Flag labels (`-b`, `-d`, `-s`, `-p`, `-f`,
    /// `--omap/--imap/--emap`) for every CLI argument that overrode a
    /// profile value at startup. When non-empty, the dialog renders a
    /// DIM hint line below the action buttons explaining why the
    /// on-screen values may not match the saved profile. Empty
    /// suppresses the hint entirely.
    cli_overrides: Vec<&'static str>,
}

impl SerialPortSetupDialog {
    /// Construct a dialog seeded with `initial_config`. The cursor
    /// starts on the baud-rate row in field-navigation (idle) mode.
    ///
    /// `cli_overrides` carries flag labels for CLI args that
    /// overrode a profile value at startup; when non-empty a hint
    /// line renders below the action buttons.
    #[must_use]
    pub const fn new(initial_config: SerialConfig, cli_overrides: Vec<&'static str>) -> Self {
        Self {
            initial: initial_config,
            pending: initial_config,
            cursor: FIELD_BAUD,
            edit_state: EditState::Idle,
            cli_overrides,
        }
    }

    /// Whether the dialog will render a CLI-override hint line at the
    /// bottom (i.e. its `cli_overrides` list is non-empty).
    #[must_use]
    pub fn has_cli_override_hint(&self) -> bool {
        !self.cli_overrides.is_empty()
    }

    /// Current cursor position. Valid range is `0..8`: indices `0..=4`
    /// select a field (baud / data bits / stop bits / parity / flow
    /// control), and `5..=7` select one of the action buttons
    /// (Apply live / Apply + Save / Cancel).
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// True while the user is typing into a numeric field.
    #[must_use]
    pub const fn is_editing(&self) -> bool {
        matches!(self.edit_state, EditState::EditingNumeric(_))
    }

    /// The currently pending [`SerialConfig`]; reflects every committed
    /// edit since construction.
    #[must_use]
    pub const fn pending(&self) -> &SerialConfig {
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

    /// Handle `Enter` while in field-navigation mode.
    fn activate(&mut self) -> DialogOutcome {
        match self.cursor {
            FIELD_BAUD | FIELD_DATA_BITS | FIELD_STOP_BITS => {
                self.edit_state = EditState::EditingNumeric(String::new());
                DialogOutcome::Consumed
            }
            FIELD_PARITY => {
                self.pending.parity = next_parity(self.pending.parity);
                DialogOutcome::Consumed
            }
            FIELD_FLOW => {
                self.pending.flow_control = next_flow(self.pending.flow_control);
                DialogOutcome::Consumed
            }
            ACTION_APPLY_LIVE => DialogOutcome::Action(DialogAction::ApplyLive(self.pending)),
            ACTION_APPLY_SAVE => DialogOutcome::Action(DialogAction::ApplyAndSave(self.pending)),
            ACTION_CANCEL => DialogOutcome::Close,
            _ => DialogOutcome::Consumed,
        }
    }

    /// Attempt to commit the in-progress numeric edit into `pending`.
    /// On parse failure the pending value is left unchanged.
    fn commit_numeric_edit(&mut self) {
        let EditState::EditingNumeric(ref buf) = self.edit_state else {
            return;
        };
        let buf = buf.clone();
        self.edit_state = EditState::Idle;
        if buf.is_empty() {
            return;
        }
        match self.cursor {
            FIELD_BAUD => {
                if let Ok(n) = buf.parse::<u32>() {
                    if n > 0 {
                        self.pending.baud_rate = n;
                    }
                }
            }
            FIELD_DATA_BITS => {
                if let Ok(n) = buf.parse::<u8>() {
                    if let Some(bits) = data_bits_from_u8(n) {
                        self.pending.data_bits = bits;
                    }
                }
            }
            FIELD_STOP_BITS => {
                if let Ok(n) = buf.parse::<u8>() {
                    if let Some(bits) = stop_bits_from_u8(n) {
                        self.pending.stop_bits = bits;
                    }
                }
            }
            _ => {}
        }
    }

    /// Handle a key while in [`EditState::EditingNumeric`].
    fn handle_key_editing(&mut self, key: KeyEvent) -> DialogOutcome {
        match key.code {
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if let EditState::EditingNumeric(ref mut buf) = self.edit_state {
                    buf.push(c);
                }
                DialogOutcome::Consumed
            }
            KeyCode::Backspace => {
                if let EditState::EditingNumeric(ref mut buf) = self.edit_state {
                    buf.pop();
                }
                DialogOutcome::Consumed
            }
            KeyCode::Enter => {
                self.commit_numeric_edit();
                DialogOutcome::Consumed
            }
            KeyCode::Esc => {
                // Discard the buffered keystrokes; pending stays untouched.
                self.edit_state = EditState::Idle;
                DialogOutcome::Consumed
            }
            _ => DialogOutcome::Consumed,
        }
    }

    /// Build the rendered field row text for `cursor == field_idx`.
    fn field_line(&self, field_idx: usize, label: &'static str, value: String) -> Line<'_> {
        let selected = self.cursor == field_idx;
        let prefix = if selected { "> " } else { "  " };
        let value_display = if selected && self.is_editing() {
            if let EditState::EditingNumeric(ref buf) = self.edit_state {
                format!("[{buf}_]")
            } else {
                value
            }
        } else {
            value
        };
        let text = format!("{prefix}{label:<12} {value_display}");
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

impl Dialog for SerialPortSetupDialog {
    #[allow(
        clippy::unnecessary_literal_bound,
        reason = "trait signature must remain &str"
    )]
    fn title(&self) -> &str {
        "Serial port setup"
    }

    fn preferred_size(&self, outer: Rect) -> Rect {
        // Reserve one extra row when the CLI-override hint is active
        // so the bottom line doesn't collide with the dialog border.
        let height = if self.has_cli_override_hint() { 19 } else { 18 };
        centred_rect(outer, 44, height)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Serial port setup");
        let inner = block.inner(area);
        block.render(area, buf);

        let cfg = &self.pending;
        let sep_width = usize::from(inner.width);
        let sep_line = Line::from(Span::styled(
            "-".repeat(sep_width),
            Style::default().add_modifier(Modifier::DIM),
        ));

        let mut lines = vec![
            Line::from(Span::raw("")),
            self.field_line(FIELD_BAUD, "Baud rate", cfg.baud_rate.to_string()),
            self.field_line(
                FIELD_DATA_BITS,
                "Data bits",
                cfg.data_bits.bits().to_string(),
            ),
            self.field_line(
                FIELD_STOP_BITS,
                "Stop bits",
                stop_bits_label(cfg.stop_bits).to_string(),
            ),
            self.field_line(FIELD_PARITY, "Parity", parity_label(cfg.parity).to_string()),
            self.field_line(
                FIELD_FLOW,
                "Flow ctrl",
                flow_label(cfg.flow_control).to_string(),
            ),
            Line::from(Span::raw("")),
            sep_line,
            Line::from(Span::raw("")),
            self.action_line(ACTION_APPLY_LIVE, "[Apply live]", "(F2)"),
            self.action_line(ACTION_APPLY_SAVE, "[Apply + Save]", "(F10)"),
            self.action_line(ACTION_CANCEL, "[Cancel]", "(Esc)"),
        ];

        if self.has_cli_override_hint() {
            let flags = self.cli_overrides.join("/");
            let hint = format!(
                " * {} field(s) overridden by CLI; relaunch without {} to use saved value *",
                self.cli_overrides.len(),
                flags,
            );
            lines.push(Line::from(Span::styled(
                hint,
                Style::default().add_modifier(Modifier::DIM),
            )));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
        // F2 / F10 fire from anywhere, including edit mode — treat them
        // as explicit "commit and apply" shortcuts.
        match key.code {
            KeyCode::F(2) => {
                self.commit_numeric_edit();
                return DialogOutcome::Action(DialogAction::ApplyLive(self.pending));
            }
            KeyCode::F(10) => {
                self.commit_numeric_edit();
                return DialogOutcome::Action(DialogAction::ApplyAndSave(self.pending));
            }
            _ => {}
        }

        if self.is_editing() {
            return self.handle_key_editing(key);
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
            // Space cycles enum fields; on numeric fields it's a no-op.
            KeyCode::Char(' ') => match self.cursor {
                FIELD_PARITY => {
                    self.pending.parity = next_parity(self.pending.parity);
                    DialogOutcome::Consumed
                }
                FIELD_FLOW => {
                    self.pending.flow_control = next_flow(self.pending.flow_control);
                    DialogOutcome::Consumed
                }
                _ => DialogOutcome::Consumed,
            },
            _ => DialogOutcome::Consumed,
        }
    }
}

/// Next parity value in the canonical cycle order (wraps).
const fn next_parity(p: Parity) -> Parity {
    match p {
        Parity::None => Parity::Even,
        Parity::Even => Parity::Odd,
        Parity::Odd => Parity::Mark,
        Parity::Mark => Parity::Space,
        Parity::Space => Parity::None,
    }
}

/// Next flow-control value (wraps).
const fn next_flow(f: FlowControl) -> FlowControl {
    match f {
        FlowControl::None => FlowControl::Hardware,
        FlowControl::Hardware => FlowControl::Software,
        FlowControl::Software => FlowControl::None,
    }
}

/// Human-readable label for a [`Parity`].
const fn parity_label(p: Parity) -> &'static str {
    match p {
        Parity::None => "none",
        Parity::Even => "even",
        Parity::Odd => "odd",
        Parity::Mark => "mark",
        Parity::Space => "space",
    }
}

/// Human-readable label for a [`FlowControl`].
const fn flow_label(f: FlowControl) -> &'static str {
    match f {
        FlowControl::None => "none",
        FlowControl::Hardware => "hw",
        FlowControl::Software => "sw",
    }
}

/// Human-readable label for a [`StopBits`].
const fn stop_bits_label(s: StopBits) -> &'static str {
    match s {
        StopBits::One => "1",
        StopBits::Two => "2",
    }
}

/// Convert `5|6|7|8` into the matching [`DataBits`] variant.
const fn data_bits_from_u8(n: u8) -> Option<DataBits> {
    match n {
        5 => Some(DataBits::Five),
        6 => Some(DataBits::Six),
        7 => Some(DataBits::Seven),
        8 => Some(DataBits::Eight),
        _ => None,
    }
}

/// Convert `1|2` into the matching [`StopBits`] variant.
const fn stop_bits_from_u8(n: u8) -> Option<StopBits> {
    match n {
        1 => Some(StopBits::One),
        2 => Some(StopBits::Two),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rtcom_core::SerialConfig;

    const fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn default_dialog() -> SerialPortSetupDialog {
        SerialPortSetupDialog::new(SerialConfig::default(), Vec::new())
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
        assert_eq!(d.cursor(), CURSOR_MAX - 1);
    }

    #[test]
    fn down_wraps_from_last_to_first() {
        let mut d = default_dialog();
        for _ in 0..CURSOR_MAX {
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
    fn dialog_without_cli_overrides_has_no_hint_row() {
        let d = SerialPortSetupDialog::new(SerialConfig::default(), Vec::new());
        assert!(!d.has_cli_override_hint());
    }

    #[test]
    fn dialog_with_cli_overrides_renders_hint() {
        use ratatui::{backend::TestBackend, layout::Rect, Terminal};
        let d = SerialPortSetupDialog::new(SerialConfig::default(), vec!["-b", "-d"]);
        assert!(d.has_cli_override_hint());

        // Render into a sizable test backend and confirm the hint text
        // reaches the on-screen buffer.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height: 24,
                };
                d.render(area, f.buffer_mut());
            })
            .unwrap();
        let rendered = format!("{}", terminal.backend());
        assert!(
            rendered.contains("2 field(s) overridden by CLI"),
            "expected hint in rendered buffer, got:\n{rendered}"
        );
        assert!(
            rendered.contains("-b/-d"),
            "expected flag list in rendered buffer, got:\n{rendered}"
        );
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
