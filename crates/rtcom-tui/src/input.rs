//! Key-event to byte-stream conversion.
//!
//! Translates `crossterm::event::KeyEvent`s into the byte sequences
//! [`rtcom_core::command::CommandKeyParser`] expects. Matches picocom /
//! minicom semantics: Enter is CR, Backspace is DEL (`0x7f`),
//! Ctrl-char is `0x01..=0x1f`, plain character keys pass through their
//! UTF-8 encoding.
