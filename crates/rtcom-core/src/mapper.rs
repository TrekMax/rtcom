//! Byte-stream mappers (CR/LF normalisation, future telnet/escape
//! decoders, ...).
//!
//! A [`Mapper`] transforms a chunk of bytes into another chunk. It is
//! deliberately direction-agnostic — the caller decides whether the
//! mapper applies to inbound (`imap`), outbound (`omap`), or echoed
//! (`emap`) traffic. v0.1 ships a single concrete mapper,
//! [`LineEndingMapper`], that covers the picocom-equivalent
//! `crlf`/`lfcr`/`igncr`/`ignlf` rules.
//!
use bytes::Bytes;

/// Line-ending transformation rule.
///
/// Names match the picocom convention:
///
/// | rule          | semantics                                            |
/// |---------------|------------------------------------------------------|
/// | `None`        | Pass bytes through unchanged (default).              |
/// | `AddCrToLf`   | Insert `\r` before every `\n` (LF → CRLF).           |
/// | `AddLfToCr`   | Insert `\n` after every `\r` (CR → CRLF).            |
/// | `DropCr`      | Discard every `\r` byte.                             |
/// | `DropLf`      | Discard every `\n` byte.                             |
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// No transformation (default).
    #[default]
    None,
    /// LF → CRLF.
    AddCrToLf,
    /// CR → CRLF.
    AddLfToCr,
    /// Drop CR.
    DropCr,
    /// Drop LF.
    DropLf,
}

/// The full set of line-ending mappers for a session's byte streams.
///
/// Holds one [`LineEnding`] rule per direction:
///
/// - `omap` — outbound (applied to bytes sent to the device)
/// - `imap` — inbound (applied to bytes received from the device)
/// - `emap` — echo map (applied to local echo display)
///
/// `Default` returns all three set to [`LineEnding::None`] — i.e. the
/// transparent configuration that passes every byte through unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineEndingConfig {
    /// Outbound mapper — bytes typed by the user before they reach the device.
    pub omap: LineEnding,
    /// Inbound mapper — bytes received from the device before they reach the screen.
    pub imap: LineEnding,
    /// Echo mapper — applied to the local echo of outbound bytes when echo is on.
    pub emap: LineEnding,
}

/// Generic byte-stream transformation.
///
/// `&mut self` because some future mappers (e.g. one that normalises
/// `\r\n` straddling a chunk boundary) will keep state across calls.
/// The line-ending mapper is stateless but pays the same signature cost.
pub trait Mapper: Send {
    /// Transforms the input chunk and returns the result.
    fn map(&mut self, bytes: &[u8]) -> Bytes;
}

/// Stateless byte mapper that applies a single [`LineEnding`] rule.
#[derive(Clone, Copy, Debug, Default)]
pub struct LineEndingMapper {
    rule: LineEnding,
}

impl LineEndingMapper {
    /// Builds a mapper that applies `rule` on every call to
    /// [`Mapper::map`].
    #[must_use]
    pub const fn new(rule: LineEnding) -> Self {
        Self { rule }
    }

    /// Returns the rule this mapper was configured with.
    #[must_use]
    pub const fn rule(&self) -> LineEnding {
        self.rule
    }
}

impl Mapper for LineEndingMapper {
    fn map(&mut self, bytes: &[u8]) -> Bytes {
        // Fast path: identity mapping copies the slice once.
        if matches!(self.rule, LineEnding::None) {
            return Bytes::copy_from_slice(bytes);
        }
        // Worst case (Add* rules) doubles every LF/CR. Reserve a hair
        // more than the input length to avoid the first realloc on the
        // common case of a few line endings per chunk.
        let mut out = Vec::with_capacity(bytes.len() + 4);
        for &byte in bytes {
            match (self.rule, byte) {
                // Both Add* rules expand the matched byte to CRLF.
                (LineEnding::AddCrToLf, b'\n') | (LineEnding::AddLfToCr, b'\r') => {
                    out.push(b'\r');
                    out.push(b'\n');
                }
                (LineEnding::DropCr, b'\r') | (LineEnding::DropLf, b'\n') => {
                    // skip
                }
                _ => out.push(byte),
            }
        }
        Bytes::from(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(rule: LineEnding, input: &[u8]) -> Vec<u8> {
        let mut m = LineEndingMapper::new(rule);
        m.map(input).to_vec()
    }

    #[test]
    fn none_passes_bytes_through_verbatim() {
        assert_eq!(run(LineEnding::None, b""), b"");
        assert_eq!(
            run(LineEnding::None, b"hello\r\nworld\n"),
            b"hello\r\nworld\n"
        );
    }

    #[test]
    fn default_rule_is_none() {
        let mut m = LineEndingMapper::default();
        assert_eq!(m.rule(), LineEnding::None);
        assert_eq!(m.map(b"abc").to_vec(), b"abc");
    }

    #[test]
    fn add_cr_to_lf_converts_lf_to_crlf() {
        assert_eq!(run(LineEnding::AddCrToLf, b"hi\nyo\n"), b"hi\r\nyo\r\n");
    }

    #[test]
    fn add_cr_to_lf_does_not_touch_existing_crlf() {
        // The rule is "before every LF, insert CR" — so an existing CR
        // before an LF means we get CRCRLF. That matches picocom's
        // behaviour and keeps the rule trivially per-byte.
        assert_eq!(run(LineEnding::AddCrToLf, b"a\r\nb"), b"a\r\r\nb");
    }

    #[test]
    fn add_cr_to_lf_handles_consecutive_lfs() {
        assert_eq!(run(LineEnding::AddCrToLf, b"\n\n"), b"\r\n\r\n");
    }

    #[test]
    fn add_lf_to_cr_converts_cr_to_crlf() {
        assert_eq!(run(LineEnding::AddLfToCr, b"hi\ryo\r"), b"hi\r\nyo\r\n");
    }

    #[test]
    fn add_lf_to_cr_does_not_touch_existing_crlf() {
        // Same rationale: per-byte rule, "after every CR, insert LF" — a
        // CR already followed by LF gains a second LF.
        assert_eq!(run(LineEnding::AddLfToCr, b"a\r\nb"), b"a\r\n\nb");
    }

    #[test]
    fn drop_cr_removes_carriage_returns_and_keeps_other_bytes() {
        assert_eq!(run(LineEnding::DropCr, b"a\r\nb\rc"), b"a\nbc");
    }

    #[test]
    fn drop_lf_removes_line_feeds_and_keeps_other_bytes() {
        assert_eq!(run(LineEnding::DropLf, b"a\r\nb\nc"), b"a\rbc");
    }

    #[test]
    fn empty_input_yields_empty_output_for_every_rule() {
        for rule in [
            LineEnding::None,
            LineEnding::AddCrToLf,
            LineEnding::AddLfToCr,
            LineEnding::DropCr,
            LineEnding::DropLf,
        ] {
            assert!(run(rule, b"").is_empty(), "{rule:?} on empty input");
        }
    }

    #[test]
    fn add_cr_to_lf_leaves_non_lf_bytes_alone() {
        // The mapper must not touch CR or arbitrary bytes when the
        // active rule targets only LF.
        assert_eq!(run(LineEnding::AddCrToLf, b"\rabc\x1bxyz"), b"\rabc\x1bxyz");
    }

    #[test]
    fn line_ending_config_default_all_none() {
        let c = LineEndingConfig::default();
        assert_eq!(c.omap, LineEnding::None);
        assert_eq!(c.imap, LineEnding::None);
        assert_eq!(c.emap, LineEnding::None);
    }
}
