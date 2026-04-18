//! Serial data pane backed by a vt100 terminal emulator.
//!
//! Bytes from the serial port are fed into [`vt100::Parser`] which
//! maintains a 2D cell grid with full ANSI support (colour, cursor
//! movement, scroll regions, and so on). Later tasks render that
//! grid into the ratatui main view; T7 stops at "state is well-kept".

/// Terminal-emulator-backed pane for serial data.
///
/// Wraps a [`vt100::Parser`] with a fixed scrollback budget.
/// Use [`SerialPane::ingest`] to feed newly received bytes;
/// [`SerialPane::screen`] exposes the current grid for rendering.
pub struct SerialPane {
    parser: vt100::Parser,
    scrollback_rows: usize,
}

impl SerialPane {
    /// Default scrollback capacity applied by [`SerialPane::new`].
    ///
    /// 10,000 rows is enough for ~minutes of typical embedded debug
    /// output and keeps peak memory bounded (cell grid at 80 cols
    /// ≈ 80 bytes × `10_000` ≈ 800 KiB).
    pub const DEFAULT_SCROLLBACK_ROWS: usize = 10_000;

    /// Build a pane with the default scrollback capacity.
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self::with_scrollback(rows, cols, Self::DEFAULT_SCROLLBACK_ROWS)
    }

    /// Build a pane with an explicit scrollback capacity.
    #[must_use]
    pub fn with_scrollback(rows: u16, cols: u16, scrollback_rows: usize) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, scrollback_rows),
            scrollback_rows,
        }
    }

    /// Feed bytes into the terminal emulator.
    ///
    /// Safe to call with any arbitrary byte stream — invalid escape
    /// sequences are dropped by vt100.
    pub fn ingest(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Reference to the current vt100 [`Screen`](vt100::Screen).
    ///
    /// The screen is mutable internally but exposed as `&` so callers
    /// can only read it; ingestion must go through [`SerialPane::ingest`].
    #[must_use]
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Resize the emulator grid.
    ///
    /// vt100 reflows accordingly — lines longer than the new width
    /// wrap; scrollback is preserved as-is.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(rows, cols);
    }

    /// Scrollback row count configured for this pane.
    #[must_use]
    pub const fn scrollback_rows(&self) -> usize {
        self.scrollback_rows
    }

    /// Current scrollback offset from the live tail (0 = live).
    ///
    /// A non-zero value means the rendered view is above the live
    /// tail by that many rows; new bytes continue to accumulate in
    /// the buffer but do not scroll the view until the user scrolls
    /// back down via [`SerialPane::scroll_down`] /
    /// [`SerialPane::scroll_to_bottom`].
    #[must_use]
    pub fn scrollback_offset(&self) -> usize {
        self.parser.screen().scrollback()
    }

    /// True when the view is above the live tail.
    ///
    /// Consumers (e.g. the top-bar renderer) use this as the "should
    /// I show the `[SCROLL ↑N]` indicator?" predicate.
    #[must_use]
    pub fn is_scrolled(&self) -> bool {
        self.scrollback_offset() > 0
    }

    /// Scroll up by `lines` (toward older content).
    ///
    /// Clamped to the configured scrollback capacity so extreme input
    /// values (e.g. `usize::MAX`) do not overflow. vt100 also clamps
    /// internally to the *actual* amount of scrollback accumulated so
    /// far, so requesting more than exists simply lands at "top of
    /// history".
    pub fn scroll_up(&mut self, lines: usize) {
        let target = self
            .scrollback_offset()
            .saturating_add(lines)
            .min(self.scrollback_rows);
        self.parser.set_scrollback(target);
    }

    /// Scroll down by `lines` (toward newer content / the live tail).
    ///
    /// Saturates at 0 (live tail); calling with a huge value is
    /// equivalent to [`SerialPane::scroll_to_bottom`].
    pub fn scroll_down(&mut self, lines: usize) {
        let target = self.scrollback_offset().saturating_sub(lines);
        self.parser.set_scrollback(target);
    }

    /// Jump to the oldest row retained in the scrollback buffer.
    ///
    /// Requests the configured scrollback capacity; vt100 internally
    /// clamps to however much history actually exists.
    pub fn scroll_to_top(&mut self) {
        self.parser.set_scrollback(self.scrollback_rows);
    }

    /// Jump back to the live tail (`scrollback_offset == 0`).
    pub fn scroll_to_bottom(&mut self) {
        self.parser.set_scrollback(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_pane_ingests_bytes_into_vt100() {
        let mut pane = SerialPane::new(24, 80);
        pane.ingest(b"hello\r\nworld");
        let screen = pane.screen();
        assert_eq!(screen.cell(0, 0).unwrap().contents(), "h");
        assert_eq!(screen.cell(1, 0).unwrap().contents(), "w");
    }

    #[test]
    fn serial_pane_resize_updates_size() {
        let mut pane = SerialPane::new(24, 80);
        for _ in 0..30 {
            pane.ingest(b"line\r\n");
        }
        pane.resize(40, 80);
        assert_eq!(pane.screen().size(), (40, 80));
    }

    #[test]
    fn serial_pane_default_scrollback_is_ten_thousand() {
        let _pane = SerialPane::new(24, 80);
        // Implementation detail — make it a public const we can verify.
        assert_eq!(SerialPane::DEFAULT_SCROLLBACK_ROWS, 10_000);
    }

    #[test]
    fn serial_pane_custom_scrollback() {
        let pane = SerialPane::with_scrollback(24, 80, 500);
        // Just ensure construction with custom scrollback succeeds.
        // Actual scrollback semantics are exercised by vt100 itself.
        assert_eq!(pane.scrollback_rows(), 500);
    }

    #[test]
    fn scroll_up_increments_offset() {
        let mut pane = SerialPane::new(24, 80);
        // Need enough content to build scrollback — ingest > 24 rows.
        for i in 0..40 {
            pane.ingest(format!("row {i}\r\n").as_bytes());
        }
        assert_eq!(pane.scrollback_offset(), 0);
        assert!(!pane.is_scrolled());
        pane.scroll_up(5);
        assert_eq!(pane.scrollback_offset(), 5);
        assert!(pane.is_scrolled());
    }

    #[test]
    fn scroll_down_decrements_offset() {
        let mut pane = SerialPane::new(24, 80);
        for i in 0..40 {
            pane.ingest(format!("row {i}\r\n").as_bytes());
        }
        pane.scroll_up(10);
        pane.scroll_down(4);
        assert_eq!(pane.scrollback_offset(), 6);
    }

    #[test]
    fn scroll_to_bottom_resets_offset() {
        let mut pane = SerialPane::new(24, 80);
        for i in 0..40 {
            pane.ingest(format!("row {i}\r\n").as_bytes());
        }
        pane.scroll_up(15);
        assert!(pane.is_scrolled());
        pane.scroll_to_bottom();
        assert_eq!(pane.scrollback_offset(), 0);
        assert!(!pane.is_scrolled());
    }

    #[test]
    fn scroll_up_clamps_to_scrollback_capacity() {
        let mut pane = SerialPane::new(24, 80);
        for _ in 0..5 {
            pane.ingest(b"x\r\n");
        }
        // Request a massive scroll — the API must not overflow and
        // must not exceed the configured scrollback capacity.
        pane.scroll_up(usize::MAX / 2);
        assert!(pane.scrollback_offset() <= SerialPane::DEFAULT_SCROLLBACK_ROWS);
    }

    #[test]
    fn scroll_down_saturates_at_zero() {
        let mut pane = SerialPane::new(24, 80);
        for i in 0..40 {
            pane.ingest(format!("row {i}\r\n").as_bytes());
        }
        // Not scrolled: scroll_down from 0 must stay at 0.
        pane.scroll_down(100);
        assert_eq!(pane.scrollback_offset(), 0);
    }

    #[test]
    fn scroll_to_top_jumps_to_oldest() {
        let mut pane = SerialPane::new(24, 80);
        for i in 0..40 {
            pane.ingest(format!("row {i}\r\n").as_bytes());
        }
        pane.scroll_to_top();
        // vt100 clamps to the actual scrollback length, which is at
        // most (total_rows - visible_rows) = 40 - 24 = 16. We only
        // assert that we moved up and that we haven't exceeded the
        // configured capacity.
        assert!(pane.is_scrolled());
        assert!(pane.scrollback_offset() <= SerialPane::DEFAULT_SCROLLBACK_ROWS);
    }

    #[test]
    fn serial_pane_ingest_handles_ansi_escape_sequences() {
        let mut pane = SerialPane::new(24, 80);
        // Red foreground, then 'X', then reset
        pane.ingest(b"\x1b[31mX\x1b[0m");
        let cell = pane.screen().cell(0, 0).unwrap();
        assert_eq!(cell.contents(), "X");
        // vt100 should have captured the red fg
        let fgcolor = cell.fgcolor();
        // vt100::Color is an enum; red maps to Color::Idx(1) in the
        // ANSI palette. Check "not default".
        assert!(
            !matches!(fgcolor, vt100::Color::Default),
            "expected coloured fg, got {fgcolor:?}",
        );
    }
}
