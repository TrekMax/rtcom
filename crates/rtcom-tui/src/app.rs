//! Top-level TUI application object.

use rtcom_core::EventBus;

/// Owns the TUI render state and input dispatcher.
///
/// T6 intentionally only tracks menu open/closed state; the serial pane,
/// modal stack, and input dispatcher land in later tasks.
pub struct TuiApp {
    // Kept for later tasks (T7+) which will subscribe / publish events.
    #[allow(dead_code)]
    bus: EventBus,
    menu_open: bool,
}

impl TuiApp {
    /// Construct a new `TuiApp` bound to the given event bus.
    #[must_use]
    pub const fn new(bus: EventBus) -> Self {
        Self {
            bus,
            menu_open: false,
        }
    }

    /// Whether the configuration menu is currently open.
    #[must_use]
    pub const fn is_menu_open(&self) -> bool {
        self.menu_open
    }

    /// Internal accessor for the bus (later tasks wire this in).
    #[allow(dead_code)]
    pub(crate) const fn bus(&self) -> &EventBus {
        &self.bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtcom_core::EventBus;

    #[test]
    fn tui_app_builds_without_running() {
        let bus = EventBus::new(64);
        let app = TuiApp::new(bus);
        assert!(!app.is_menu_open());
    }
}
