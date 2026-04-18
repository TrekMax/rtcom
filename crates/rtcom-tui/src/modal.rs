//! Modal dialog trait + a stack that routes input to the topmost
//! dialog. T10 defines the abstraction; T11+ wire actual dialogs
//! (root menu, serial port setup, ...) on top of it.

use crossterm::event::KeyEvent;
use ratatui::{buffer::Buffer, layout::Rect};

use rtcom_core::SerialConfig;

/// What a [`Dialog`] wants the surrounding [`ModalStack`] to do after
/// it has processed an input event.
#[derive(Debug)]
pub enum DialogOutcome {
    /// Dialog handled the key; stack stays as-is.
    Consumed,
    /// Dialog wants to close itself (Esc, Cancel, action complete).
    Close,
    /// Dialog produced a user-level action for the outer app to
    /// apply (e.g. save the profile, push a config change).
    Action(DialogAction),
}

/// User-level actions emitted by dialogs. The `TuiApp` orchestrator
/// consumes these and calls into `rtcom-core` / `rtcom-config` to
/// apply them.
#[derive(Debug, Clone)]
pub enum DialogAction {
    /// Apply `SerialConfig` to the live session immediately (F2 path).
    ApplyLive(SerialConfig),
    /// Apply `SerialConfig` to the live session *and* persist to
    /// profile (F10 path).
    ApplyAndSave(SerialConfig),
    /// Persist the current profile as-is.
    WriteProfile,
    /// Reload profile from disk (discards unsaved live changes).
    ReadProfile,
}

/// A full-screen or modal dialog rendered over the main TUI chrome.
///
/// Implementors typically hold their own local state (cursor, field
/// values, ...), draw themselves inside the provided area, and emit
/// a [`DialogOutcome`] per key event to tell the surrounding
/// [`ModalStack`] how to react.
pub trait Dialog {
    /// Human-readable title, used for decoration.
    fn title(&self) -> &str;
    /// Render the dialog into the given area.
    fn render(&self, area: Rect, buf: &mut Buffer);
    /// Handle a key event and report back how the stack should react.
    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome;
}

/// Stack of [`Dialog`]s. The topmost dialog receives keys first;
/// [`DialogOutcome::Close`] pops it.
///
/// The `Send` bound on the contained trait objects keeps
/// [`ModalStack`] usable inside an async task that may be moved
/// between tokio worker threads.
pub struct ModalStack {
    stack: Vec<Box<dyn Dialog + Send>>,
}

impl Default for ModalStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalStack {
    /// Empty stack.
    #[must_use]
    pub const fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// True if no dialog is on the stack.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Number of dialogs on the stack.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Reference to the topmost dialog, if any.
    #[must_use]
    pub fn top(&self) -> Option<&(dyn Dialog + Send)> {
        self.stack.last().map(AsRef::as_ref)
    }

    /// Push a dialog onto the stack. It becomes the new top.
    pub fn push(&mut self, dialog: Box<dyn Dialog + Send>) {
        self.stack.push(dialog);
    }

    /// Pop the topmost dialog off the stack.
    pub fn pop(&mut self) -> Option<Box<dyn Dialog + Send>> {
        self.stack.pop()
    }

    /// Clear the entire stack — used on forced-quit /
    /// device-disconnect.
    pub fn clear(&mut self) {
        self.stack.clear();
    }

    /// Route a key event to the topmost dialog. Empty stack returns
    /// [`DialogOutcome::Consumed`] (nothing to do).
    ///
    /// Automatically handles [`DialogOutcome::Close`] by popping the
    /// top dialog.
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
        let Some(top) = self.stack.last_mut() else {
            return DialogOutcome::Consumed;
        };
        let outcome = top.handle_key(key);
        if matches!(outcome, DialogOutcome::Close) {
            self.stack.pop();
        }
        outcome
    }
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::unnecessary_literal_bound,
    reason = "test code mirrors the T10 spec verbatim"
)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{buffer::Buffer, layout::Rect};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Counts calls to handle_key.
    struct CountingDialog {
        count: Arc<AtomicUsize>,
    }

    impl Dialog for CountingDialog {
        fn title(&self) -> &str {
            "counting"
        }
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}
        fn handle_key(&mut self, _key: KeyEvent) -> DialogOutcome {
            self.count.fetch_add(1, Ordering::SeqCst);
            DialogOutcome::Consumed
        }
    }

    /// Closes on Esc, consumes everything else.
    struct ClosingDialog;

    impl Dialog for ClosingDialog {
        fn title(&self) -> &str {
            "closing"
        }
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}
        fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome {
            if key.code == KeyCode::Esc {
                DialogOutcome::Close
            } else {
                DialogOutcome::Consumed
            }
        }
    }

    #[test]
    fn modal_stack_starts_empty() {
        let stack = ModalStack::new();
        assert!(stack.is_empty());
        assert!(stack.top().is_none());
    }

    #[test]
    fn modal_stack_push_pop() {
        let mut stack = ModalStack::new();
        stack.push(Box::new(ClosingDialog));
        assert!(!stack.is_empty());
        assert_eq!(stack.top().map(Dialog::title), Some("closing"));
        let popped = stack.pop();
        assert!(popped.is_some());
        assert!(stack.is_empty());
    }

    #[test]
    fn modal_stack_routes_keys_to_top() {
        let count = Arc::new(AtomicUsize::new(0));
        let mut stack = ModalStack::new();
        stack.push(Box::new(CountingDialog {
            count: count.clone(),
        }));
        let _ = stack.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn modal_stack_close_outcome_pops_top() {
        let mut stack = ModalStack::new();
        stack.push(Box::new(ClosingDialog));
        stack.push(Box::new(ClosingDialog));
        assert_eq!(stack.depth(), 2);
        let _ = stack.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(stack.depth(), 1);
    }

    #[test]
    fn modal_stack_handle_key_on_empty_is_noop() {
        let mut stack = ModalStack::new();
        let outcome = stack.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(outcome, DialogOutcome::Consumed));
    }

    #[test]
    fn dialog_action_apply_live_carries_config() {
        use rtcom_core::SerialConfig;
        let cfg = SerialConfig::default();
        let action = DialogAction::ApplyLive(cfg);
        match action {
            DialogAction::ApplyLive(_) => {}
            _ => panic!("wrong variant"),
        }
    }
}
