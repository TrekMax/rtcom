//! Runtime commands and the keyboard state machine that produces them.
//!
//! Stub: only the public types are defined here. Behaviour is filled in
//! by the next commit in the TDD cycle.

/// One actionable command produced by [`CommandKeyParser`].
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[allow(dead_code, reason = "fields are written by the green commit")]
pub struct CommandKeyParser {
    escape: u8,
    state: State,
}

#[allow(dead_code, reason = "variants are constructed by the green commit")]
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
    pub fn feed(&mut self, _byte: u8) -> ParseOutput {
        todo!("CommandKeyParser::feed — implementation lands in the green commit")
    }
}

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

    #[test]
    fn escape_then_q_or_x_emits_quit() {
        for key in [b'q', b'x'] {
            let mut p = parser();
            assert_eq!(p.feed(ESC), ParseOutput::None);
            assert_eq!(p.feed(key), ParseOutput::Command(Command::Quit));
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
        assert_eq!(p.feed(b'q'), ParseOutput::Command(Command::Quit));
        assert_eq!(p.feed(b'a'), ParseOutput::Data(b'a'));
    }

    #[test]
    fn escape_byte_is_observable() {
        assert_eq!(parser().escape_byte(), ESC);
    }
}
