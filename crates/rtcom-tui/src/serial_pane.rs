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
