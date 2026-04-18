//! Top-level chrome layout for the TUI main screen.
//!
//! The main screen has three horizontal bands:
//!
//! ```text
//! ┌────────────────────────────────────┐
//! │ top bar (1 row, status / device)   │
//! ├────────────────────────────────────┤
//! │ body (serial pane)                 │
//! │                                    │
//! │                                    │
//! ├────────────────────────────────────┤
//! │ bottom bar (1 row, hint text)      │
//! └────────────────────────────────────┘
//! ```
//!
//! [`main_chrome`] splits a [`Rect`] into exactly those three bands so
//! the top-level renderer can hand each sub-rect to the right widget.

use ratatui::layout::{Constraint, Layout, Rect};

/// Splits the terminal area into top bar (1 row), body (min 1 row),
/// and bottom bar (1 row).
///
/// Returned tuple is `(top, body, bottom)`. When `area.height < 3`
/// the ratatui [`Layout`] engine still yields three rects but some
/// may be zero-sized; the caller is responsible for skipping them.
///
/// # Examples
///
/// ```
/// use ratatui::layout::Rect;
/// use rtcom_tui::layout::main_chrome;
///
/// let (top, body, bottom) = main_chrome(Rect::new(0, 0, 80, 24));
/// assert_eq!(top.height, 1);
/// assert_eq!(body.height, 22);
/// assert_eq!(bottom.height, 1);
/// ```
#[must_use]
pub fn main_chrome(area: Rect) -> (Rect, Rect, Rect) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    (rows[0], rows[1], rows[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_chrome_splits_80x24() {
        let (top, body, bottom) = main_chrome(Rect::new(0, 0, 80, 24));
        assert_eq!(top.height, 1);
        assert_eq!(bottom.height, 1);
        assert_eq!(body.height, 22);
        assert_eq!(top.width, 80);
        assert_eq!(body.width, 80);
        assert_eq!(bottom.width, 80);
    }

    #[test]
    fn main_chrome_preserves_origin() {
        let (top, body, bottom) = main_chrome(Rect::new(5, 2, 40, 10));
        assert_eq!(top.y, 2);
        assert_eq!(body.y, 3);
        // bottom.y = origin.y + height - 1 = 2 + 10 - 1 = 11
        assert_eq!(bottom.y, 11);
        assert_eq!(top.x, 5);
        assert_eq!(body.x, 5);
        assert_eq!(bottom.x, 5);
    }
}
