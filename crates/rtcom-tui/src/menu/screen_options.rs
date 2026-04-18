//! Screen-options dialog — edits the TUI's modal-render style.
//!
//! Three radio options (Overlay / Dimmed overlay / Fullscreen) plus
//! three action buttons (Apply live / Apply + Save / Cancel) — six
//! cursor positions total. `scrollback_rows` is shown as a read-only
//! display for v0.2; full editing arrives post-v1.0.
//!
//! Skeleton — T15 GREEN implements real behaviour.

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use rtcom_config::ModalStyle;

use crate::modal::{centred_rect, Dialog, DialogOutcome};

/// Index of the `Overlay` radio row.
const RADIO_OVERLAY: usize = 0;
/// Index of the `Dimmed overlay` radio row.
#[allow(dead_code, reason = "used by T15 GREEN")]
const RADIO_DIMMED_OVERLAY: usize = 1;
/// Index of the `Fullscreen` radio row.
#[allow(dead_code, reason = "used by T15 GREEN")]
const RADIO_FULLSCREEN: usize = 2;
/// Index of the `[Apply live]` action button.
#[allow(dead_code, reason = "used by T15 GREEN")]
const ACTION_APPLY_LIVE: usize = 3;
/// Index of the `[Apply + Save]` action button.
#[allow(dead_code, reason = "used by T15 GREEN")]
const ACTION_APPLY_SAVE: usize = 4;
/// Index of the `[Cancel]` action button.
#[allow(dead_code, reason = "used by T15 GREEN")]
const ACTION_CANCEL: usize = 5;

/// Total cursor slots (3 radios + 3 actions).
#[allow(dead_code, reason = "used by T15 GREEN")]
const CURSOR_MAX: usize = 6;

/// Screen-options dialog.
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

    fn render(&self, _area: Rect, _buf: &mut Buffer) {
        // T15 GREEN: radios + action buttons + scrollback display.
    }

    fn handle_key(&mut self, _key: KeyEvent) -> DialogOutcome {
        DialogOutcome::Consumed
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
