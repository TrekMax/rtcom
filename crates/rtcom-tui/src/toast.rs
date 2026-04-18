//! Timed toast notifications for the TUI.
//!
//! A [`ToastQueue`] holds up to three [`Toast`] entries with an
//! [`Instant`]-based expiration. The renderer draws visible toasts in
//! a single row each, stacked top-to-bottom with the newest entry at
//! the top. Expiration is driven by calls to [`ToastQueue::tick`] —
//! the TUI runner invokes it on a periodic timer so toasts disappear
//! even when no key / bus events arrive.
//!
//! Toasts overlay every other chrome including the modal dialog, so
//! outcome messages (`Event::ProfileSaved`, `Event::ProfileLoadFailed`,
//! `Event::Error`) are always visible regardless of menu state.
//!
//! Symbol choice: ASCII prefixes (`i  `, `!  `, `x  `) keep snapshot
//! output deterministic across terminals that render unicode
//! differently. Colour distinguishes severity.

use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

/// Severity level for a toast. Drives the foreground colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    /// Informational (e.g., "profile saved"). Rendered in green.
    Info,
    /// Warning. Rendered in yellow.
    Warn,
    /// Error (e.g., "profile IO failed"). Rendered in red.
    Error,
}

/// A single toast message with an expiration time.
#[derive(Debug, Clone)]
pub struct Toast {
    /// User-visible text rendered to the right of the severity prefix.
    pub message: String,
    /// Severity level; drives the colour.
    pub level: ToastLevel,
    /// Absolute wall-clock instant after which the toast is dropped
    /// by [`ToastQueue::tick`].
    pub expires_at: Instant,
}

impl Toast {
    /// Compute the [`Style`] used for both the severity prefix and
    /// the message text.
    #[must_use]
    pub fn style(&self) -> Style {
        let fg = match self.level {
            ToastLevel::Info => Color::Green,
            ToastLevel::Warn => Color::Yellow,
            ToastLevel::Error => Color::Red,
        };
        Style::default().fg(fg).add_modifier(Modifier::BOLD)
    }
}

/// Bounded queue of visible toasts.
///
/// When full, pushing a new toast drops the oldest entry to make
/// room. Expired toasts are dropped by [`ToastQueue::tick`].
#[derive(Debug)]
pub struct ToastQueue {
    toasts: Vec<Toast>,
    max_visible: usize,
}

impl Default for ToastQueue {
    fn default() -> Self {
        Self {
            toasts: Vec::new(),
            max_visible: 3,
        }
    }
}

impl ToastQueue {
    /// Default visible lifetime used by [`ToastQueue::push`].
    pub const DEFAULT_LIFETIME: Duration = Duration::from_secs(3);

    /// Construct an empty queue with the default capacity (3 visible).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new toast with the default 3-second lifetime. Drops the
    /// oldest visible toast if the queue is already at capacity.
    pub fn push(&mut self, message: impl Into<String>, level: ToastLevel) {
        self.push_with_lifetime(message, level, Self::DEFAULT_LIFETIME);
    }

    /// Push a toast with a caller-supplied lifetime. Mainly useful for
    /// tests that need a short-lived toast to exercise [`Self::tick`].
    pub fn push_with_lifetime(
        &mut self,
        message: impl Into<String>,
        level: ToastLevel,
        lifetime: Duration,
    ) {
        let toast = Toast {
            message: message.into(),
            level,
            expires_at: Instant::now() + lifetime,
        };
        if self.toasts.len() >= self.max_visible {
            self.toasts.remove(0);
        }
        self.toasts.push(toast);
    }

    /// Drop expired toasts. Call on each render tick so entries
    /// disappear at the advertised lifetime even when no key / bus
    /// events arrive.
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.toasts.retain(|t| t.expires_at > now);
    }

    /// Borrow the list of currently-visible toasts, oldest first.
    //
    // Not `const fn`: `&self.toasts` (`Vec<Toast>` → `&[Toast]`) goes
    // through deref coercion, which is not yet const as of Rust 1.86.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn visible(&self) -> &[Toast] {
        &self.toasts
    }

    /// Number of currently-visible toasts.
    #[must_use]
    pub const fn visible_count(&self) -> usize {
        self.toasts.len()
    }

    /// `true` when no toasts are currently visible.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }
}

/// Render the queue's visible toasts into `area`.
///
/// Each toast occupies one row; the newest entry is drawn at the top
/// of the area, older entries below. Rows past `area.height` are
/// skipped silently.
pub fn render_toasts(queue: &ToastQueue, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Newest at top: iterate in reverse insertion order.
    for (i, toast) in queue.visible().iter().rev().enumerate() {
        let offset = u16::try_from(i).unwrap_or(u16::MAX);
        if offset >= area.height {
            break;
        }
        let row = Rect {
            x: area.x,
            y: area.y + offset,
            width: area.width,
            height: 1,
        };
        let prefix = match toast.level {
            ToastLevel::Info => "i  ",
            ToastLevel::Warn => "!  ",
            ToastLevel::Error => "x  ",
        };
        let style = toast.style();
        let line = Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(toast.message.as_str(), style),
        ]);
        Paragraph::new(line).render(row, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn queue_starts_empty() {
        let q = ToastQueue::new();
        assert_eq!(q.visible_count(), 0);
        assert!(q.is_empty());
    }

    #[test]
    fn push_adds_toast() {
        let mut q = ToastQueue::new();
        q.push("hello", ToastLevel::Info);
        assert_eq!(q.visible_count(), 1);
        assert_eq!(q.visible()[0].message, "hello");
        assert_eq!(q.visible()[0].level, ToastLevel::Info);
    }

    #[test]
    fn tick_removes_expired_toasts() {
        let mut q = ToastQueue::new();
        q.push_with_lifetime("short", ToastLevel::Info, Duration::from_millis(10));
        assert_eq!(q.visible_count(), 1);
        thread::sleep(Duration::from_millis(25));
        q.tick();
        assert_eq!(q.visible_count(), 0);
        assert!(q.is_empty());
    }

    #[test]
    fn tick_keeps_live_toasts() {
        let mut q = ToastQueue::new();
        q.push_with_lifetime("live", ToastLevel::Info, Duration::from_secs(60));
        q.tick();
        assert_eq!(q.visible_count(), 1);
    }

    #[test]
    fn queue_drops_oldest_when_full() {
        let mut q = ToastQueue::new();
        // Default max_visible = 3.
        for i in 0..4 {
            q.push(format!("t{i}"), ToastLevel::Info);
        }
        assert_eq!(q.visible_count(), 3);
        let msgs: Vec<&str> = q.visible().iter().map(|t| t.message.as_str()).collect();
        assert_eq!(msgs, vec!["t1", "t2", "t3"]);
    }

    #[test]
    fn default_lifetime_is_3_seconds() {
        assert_eq!(ToastQueue::DEFAULT_LIFETIME, Duration::from_secs(3));
    }

    #[test]
    fn level_colours_are_distinct() {
        let now = Instant::now();
        let info = Toast {
            message: String::new(),
            level: ToastLevel::Info,
            expires_at: now,
        };
        let warn = Toast {
            message: String::new(),
            level: ToastLevel::Warn,
            expires_at: now,
        };
        let err = Toast {
            message: String::new(),
            level: ToastLevel::Error,
            expires_at: now,
        };
        assert_ne!(info.style().fg, warn.style().fg);
        assert_ne!(warn.style().fg, err.style().fg);
        assert_ne!(info.style().fg, err.style().fg);
    }

    #[test]
    fn level_styles_are_bold() {
        let t = Toast {
            message: String::new(),
            level: ToastLevel::Info,
            expires_at: Instant::now(),
        };
        assert!(t.style().add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_noop_on_zero_area() {
        let mut q = ToastQueue::new();
        q.push("hello", ToastLevel::Info);
        let mut buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        render_toasts(&q, Rect::new(0, 0, 0, 0), &mut buf);
        // Nothing to assert beyond "does not panic".
    }

    #[test]
    fn render_writes_newest_toast_at_top() {
        let mut q = ToastQueue::new();
        q.push("first", ToastLevel::Info);
        q.push("second", ToastLevel::Warn);
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        render_toasts(&q, area, &mut buf);
        // Row 0 should start with the Warn prefix and "second".
        let top = buf_row_string(&buf, 0);
        assert!(top.starts_with("!  second"), "got: {top:?}");
        // Row 1 should start with the Info prefix and "first".
        let row1 = buf_row_string(&buf, 1);
        assert!(row1.starts_with("i  first"), "got: {row1:?}");
    }

    fn buf_row_string(buf: &Buffer, y: u16) -> String {
        let area = buf.area;
        (0..area.width)
            .map(|x| buf[(area.x + x, area.y + y)].symbol().to_string())
            .collect::<String>()
    }
}
