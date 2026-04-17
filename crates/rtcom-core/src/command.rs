//! Runtime commands and the keyboard state machine that produces them.
//!
//! Stub: only the public types are defined here. Behaviour is filled in
//! by the next commit in the TDD cycle.

/// One actionable command produced by [`CommandKeyParser`].
///
/// `Copy` because every variant carries only `Copy` data (currently a
/// single `u32` for `SetBaud`). Passing by value is therefore cheap and
/// matches how the dispatcher consumes the value via `match`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Show the help / command-key cheatsheet.
    Help,
    /// Quit the session.
    Quit,
    /// Print the current [`SerialConfig`](crate::SerialConfig).
    ShowConfig,
    /// Toggle the DTR output line.
    ToggleDtr,
    /// Toggle the RTS output line.
    ToggleRts,
    /// Send a line break (~250 ms by default in Issue #7's handler).
    SendBreak,
    /// Apply a new baud rate, parsed from the digits collected after `b`.
    SetBaud(u32),
}

/// What [`CommandKeyParser::feed`] produced for a single input byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseOutput {
    /// Parser is buffering — nothing to emit for this byte.
    None,
    /// Pass this byte through to the device as user data.
    Data(u8),
    /// A command was recognised; dispatch it.
    Command(Command),
}

/// State machine that splits stdin bytes into "data to send" vs.
/// "commands to dispatch" using the configurable escape key.
pub struct CommandKeyParser {
    escape: u8,
    state: State,
}

enum State {
    Default,
    AwaitingCommand,
    AwaitingBaudDigits(String),
}

impl CommandKeyParser {
    /// Builds a parser whose command key is `escape` (commonly `^T` =
    /// `0x14`).
    #[must_use]
    pub const fn new(escape: u8) -> Self {
        Self {
            escape,
            state: State::Default,
        }
    }

    /// Returns the escape byte this parser was configured with.
    #[must_use]
    pub const fn escape_byte(&self) -> u8 {
        self.escape
    }

    /// Feed a single input byte; returns whatever the parser decided to
    /// emit for it.
    ///
    /// State transitions (with `^T` as the escape byte for illustration):
    ///
    /// | from \ byte         | `^T`              | `Esc` (`0x1b`)   | mapped command  | `b`                         | digit (in baud sub-state) | `\r` / `\n` (in baud sub-state) | other                |
    /// |---------------------|-------------------|------------------|-----------------|-----------------------------|---------------------------|---------------------------------|----------------------|
    /// | Default             | → AwaitingCommand | → Data(byte)     | → Data(byte)    | → Data(byte)                | n/a                       | n/a                             | → Data(byte)         |
    /// | AwaitingCommand     | → Data(`^T`)      | → Default        | → Command(...)  | → AwaitingBaudDigits        | n/a                       | n/a                             | → Default (drop)     |
    /// | AwaitingBaudDigits  | → Default (drop)  | → Default        | → Default (drop)| → Default (drop)            | append, stay              | → SetBaud / Default             | → Default (drop)     |
    pub fn feed(&mut self, byte: u8) -> ParseOutput {
        match std::mem::replace(&mut self.state, State::Default) {
            State::Default => {
                if byte == self.escape {
                    self.state = State::AwaitingCommand;
                    ParseOutput::None
                } else {
                    ParseOutput::Data(byte)
                }
            }
            State::AwaitingCommand => self.handle_command_byte(byte),
            State::AwaitingBaudDigits(buf) => self.handle_baud_byte(buf, byte),
        }
    }

    fn handle_command_byte(&mut self, byte: u8) -> ParseOutput {
        if byte == self.escape {
            // Double-escape: pass the escape character through as data.
            return ParseOutput::Data(self.escape);
        }
        match byte {
            ESC_KEY => ParseOutput::None,
            b'?' | b'h' => ParseOutput::Command(Command::Help),
            // Picocom convention: Quit is bound to ^Q (0x11) and ^X
            // (0x18) — control bytes — not the plain letters. That
            // frees the letters to be sent to the wire as data without
            // an extra escape dance.
            CTRL_Q | CTRL_X => ParseOutput::Command(Command::Quit),
            b'c' => ParseOutput::Command(Command::ShowConfig),
            b't' => ParseOutput::Command(Command::ToggleDtr),
            b'g' => ParseOutput::Command(Command::ToggleRts),
            b'\\' => ParseOutput::Command(Command::SendBreak),
            b'b' => {
                self.state = State::AwaitingBaudDigits(String::new());
                ParseOutput::None
            }
            _ => ParseOutput::None,
        }
    }

    fn handle_baud_byte(&mut self, mut buf: String, byte: u8) -> ParseOutput {
        match byte {
            b'\r' | b'\n' => match buf.parse::<u32>() {
                Ok(rate) if rate > 0 => ParseOutput::Command(Command::SetBaud(rate)),
                _ => ParseOutput::None,
            },
            ESC_KEY => ParseOutput::None,
            d if d.is_ascii_digit() => {
                buf.push(d as char);
                self.state = State::AwaitingBaudDigits(buf);
                ParseOutput::None
            }
            _ => ParseOutput::None,
        }
    }
}

const ESC_KEY: u8 = 0x1b;
/// Ctrl-Q. Picocom's "quit" key.
const CTRL_Q: u8 = 0x11;
/// Ctrl-X. Picocom's "terminate" key.
const CTRL_X: u8 = 0x18;

#[cfg(test)]
mod tests {
    use super::*;

    const ESC: u8 = 0x14; // ^T

    const fn parser() -> CommandKeyParser {
        CommandKeyParser::new(ESC)
    }

    fn drive(p: &mut CommandKeyParser, bytes: &[u8]) -> Vec<ParseOutput> {
        bytes.iter().map(|&b| p.feed(b)).collect()
    }

    #[test]
    fn default_state_passes_bytes_through() {
        let mut p = parser();
        assert_eq!(
            drive(&mut p, b"abc"),
            vec![
                ParseOutput::Data(b'a'),
                ParseOutput::Data(b'b'),
                ParseOutput::Data(b'c'),
            ]
        );
    }

    #[test]
    fn escape_alone_produces_no_output() {
        let mut p = parser();
        assert_eq!(p.feed(ESC), ParseOutput::None);
    }

    /// `^Q` (0x11) and `^X` (0x18) are the picocom-style quit keys.
    /// Lowercase `q`/`x` plain-letters fall through to "unknown" and
    /// must NOT quit — that mirrors picocom and frees the letters to
    /// be sent to the wire as data without an extra escape dance.
    #[test]
    fn escape_then_ctrl_q_or_ctrl_x_emits_quit() {
        for key in [0x11_u8, 0x18_u8] {
            let mut p = parser();
            assert_eq!(p.feed(ESC), ParseOutput::None);
            assert_eq!(p.feed(key), ParseOutput::Command(Command::Quit));
        }
    }

    #[test]
    fn escape_then_lowercase_q_or_x_does_not_quit() {
        for key in [b'q', b'x'] {
            let mut p = parser();
            assert_eq!(p.feed(ESC), ParseOutput::None);
            // Unmapped after escape -> drop and return to default.
            assert_eq!(p.feed(key), ParseOutput::None);
            // Default state: next byte passes through verbatim.
            assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
        }
    }

    #[test]
    fn escape_then_help_keys_emit_help() {
        for key in [b'?', b'h'] {
            let mut p = parser();
            p.feed(ESC);
            assert_eq!(p.feed(key), ParseOutput::Command(Command::Help));
        }
    }

    #[test]
    fn escape_then_c_emits_show_config() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(b'c'), ParseOutput::Command(Command::ShowConfig));
    }

    #[test]
    fn escape_then_t_emits_toggle_dtr() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(b't'), ParseOutput::Command(Command::ToggleDtr));
    }

    #[test]
    fn escape_then_g_emits_toggle_rts() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(b'g'), ParseOutput::Command(Command::ToggleRts));
    }

    #[test]
    fn escape_then_backslash_emits_send_break() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(b'\\'), ParseOutput::Command(Command::SendBreak));
    }

    #[test]
    fn baud_change_collects_digits_and_emits_set_baud_on_cr() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(b'b'), ParseOutput::None);
        for &d in b"9600" {
            assert_eq!(p.feed(d), ParseOutput::None);
        }
        assert_eq!(p.feed(b'\r'), ParseOutput::Command(Command::SetBaud(9600)));
    }

    #[test]
    fn baud_change_lf_terminator_works_too() {
        let mut p = parser();
        p.feed(ESC);
        p.feed(b'b');
        for &d in b"115200" {
            p.feed(d);
        }
        assert_eq!(
            p.feed(b'\n'),
            ParseOutput::Command(Command::SetBaud(115_200))
        );
    }

    #[test]
    fn baud_change_cancelled_by_esc_returns_to_default() {
        let mut p = parser();
        p.feed(ESC);
        p.feed(b'b');
        p.feed(b'9');
        assert_eq!(p.feed(0x1b), ParseOutput::None);
        // Default state again.
        assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
    }

    #[test]
    fn baud_change_cancelled_by_non_digit() {
        let mut p = parser();
        p.feed(ESC);
        p.feed(b'b');
        p.feed(b'9');
        assert_eq!(p.feed(b'x'), ParseOutput::None);
        assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
    }

    #[test]
    fn baud_change_with_empty_digits_is_dropped() {
        let mut p = parser();
        p.feed(ESC);
        p.feed(b'b');
        // Immediate Enter with no digits — nothing to apply, return to default.
        assert_eq!(p.feed(b'\r'), ParseOutput::None);
        assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
    }

    #[test]
    fn double_escape_passes_escape_byte_through() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(ESC), ParseOutput::Data(ESC));
    }

    #[test]
    fn esc_in_command_state_cancels_quietly() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(0x1b), ParseOutput::None);
        assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
    }

    #[test]
    fn unknown_command_byte_silently_drops_and_resets() {
        let mut p = parser();
        p.feed(ESC);
        assert_eq!(p.feed(b'z'), ParseOutput::None);
        assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
    }

    #[test]
    fn pass_through_resumes_after_command() {
        let mut p = parser();
        p.feed(ESC);
        // ^X (0x18) is one of the picocom-style quit keys.
        assert_eq!(p.feed(0x18), ParseOutput::Command(Command::Quit));
        assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
    }

    #[test]
    fn escape_byte_is_observable() {
        assert_eq!(parser().escape_byte(), ESC);
    }
}
