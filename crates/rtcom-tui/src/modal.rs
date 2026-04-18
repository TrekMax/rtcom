//! Modal dialog trait + a stack that routes input to the topmost
//! dialog. T10 defines the abstraction; T11+ wire actual dialogs
//! (root menu, serial port setup, ...) on top of it.

#[cfg(test)]
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
