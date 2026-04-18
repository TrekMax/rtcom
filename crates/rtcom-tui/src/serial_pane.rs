//! Serial data pane backed by a vt100 terminal emulator.

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
