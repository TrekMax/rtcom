//! Key-event to byte-stream conversion.
//!
//! Translates `crossterm::event::KeyEvent`s into the byte sequences
//! [`rtcom_core::command::CommandKeyParser`] expects. Matches picocom /
//! minicom semantics: Enter is CR, Backspace is DEL (`0x7f`),
//! Ctrl-char is `0x01..=0x1f`, plain character keys pass through their
//! UTF-8 encoding.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::modal::DialogAction;

/// What the dispatcher decided to do with an inbound key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    /// Bytes to write to the serial device.
    TxBytes(Vec<u8>),
    /// Menu opened as a result of this event.
    OpenedMenu,
    /// Menu closed as a result of this event.
    ClosedMenu,
    /// User requested a clean quit.
    Quit,
    /// Dialog emitted a user-level action (apply-live, save-profile,
    /// …). The outer runner interprets this and calls into
    /// `rtcom-core` / `rtcom-config`.
    Action(DialogAction),
    /// No observable side effect (parser buffering, key swallowed
    /// by the menu, etc.).
    Noop,
}

/// Translate a crossterm [`KeyEvent`] into the raw byte sequence the
/// remote device sees. Returns an empty `Vec` for events that do not
/// correspond to a byte on the wire (e.g. modifier-only presses).
///
/// Semantics:
/// - `Ctrl-<letter>` → `0x01..=0x1a` (e.g. Ctrl-A → `0x01`).
/// - Plain [`KeyCode::Char`] → UTF-8 encoding of that character.
/// - [`KeyCode::Enter`] → `CR` (`0x0d`), matching picocom's default
///   "send CR on Enter" behaviour. Line-ending translation (CR → CRLF
///   etc.) is the mapper's job (Issue #8), not ours.
/// - [`KeyCode::Tab`] → `HT` (`0x09`).
/// - [`KeyCode::Backspace`] → `DEL` (`0x7f`), again matching picocom.
/// - [`KeyCode::Esc`] → `ESC` (`0x1b`).
/// - Anything else (arrows, function keys, modifier-only) returns
///   an empty `Vec`. T14+ can grow CSI sequences if a real use
///   case surfaces.
#[must_use]
pub fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    match (key.code, key.modifiers) {
        // Ctrl-<letter>: 0x01..=0x1a
        (KeyCode::Char(c), m) if m.contains(KeyModifiers::CONTROL) && c.is_ascii_alphabetic() => {
            vec![(c.to_ascii_lowercase() as u8) - b'a' + 1]
        }
        // Plain printable char (possibly with Shift): UTF-8 encode
        (KeyCode::Char(c), _) => {
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        (KeyCode::Enter, _) => vec![b'\r'],
        (KeyCode::Tab, _) => vec![b'\t'],
        (KeyCode::Backspace, _) => vec![0x7f],
        (KeyCode::Esc, _) => vec![0x1b],
        // Arrow keys, F-keys, etc. — emit nothing for T9 baseline.
        // T12/T13 dialog navigation handles these inline (menu-open
        // branch); the serial passthrough needs them only with
        // CSI encoding, which T14+ can add if needed.
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_a_is_0x01() {
        let ev = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(key_to_bytes(ev), vec![0x01]);
    }

    #[test]
    fn plain_letter_is_ascii() {
        let ev = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(key_to_bytes(ev), vec![b'h']);
    }

    #[test]
    fn enter_is_cr() {
        let ev = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(ev), vec![b'\r']);
    }

    #[test]
    fn esc_is_0x1b() {
        let ev = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(key_to_bytes(ev), vec![0x1b]);
    }
}
