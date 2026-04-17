//! Command-line argument parsing for the `rtcom` binary.
//!
//! Parsing lives here so `main.rs` stays a thin entry point. The [`Cli`]
//! struct mirrors what `clap` reads from `argv`; [`Cli::to_serial_config`]
//! projects it into [`rtcom_core::SerialConfig`] for the session layer.

use clap::{ArgAction, Parser, ValueEnum};

use rtcom_core::{
    DataBits, FlowControl, LineEnding, Parity, SerialConfig, StopBits, DEFAULT_READ_TIMEOUT,
};

/// Parsed `rtcom` command-line invocation.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "rtcom",
    version,
    about = "Rust Terminal Communication — modern serial terminal",
    long_about = None,
)]
pub struct Cli {
    /// Serial device path, e.g. `/dev/ttyUSB0` (Linux) or `COM3` (Windows).
    pub device: String,

    /// Baud rate in bits per second.
    #[arg(short, long, default_value_t = 115_200, value_name = "RATE")]
    pub baud: u32,

    /// Data bits per frame.
    #[arg(
        short = 'd',
        long = "databits",
        value_enum,
        default_value_t = CliDataBits::Eight,
        value_name = "BITS",
    )]
    pub data_bits: CliDataBits,

    /// Stop bits per frame.
    #[arg(
        short = 's',
        long = "stopbits",
        value_enum,
        default_value_t = CliStopBits::One,
        value_name = "BITS",
    )]
    pub stop_bits: CliStopBits,

    /// Parity mode.
    #[arg(
        short = 'p',
        long,
        value_enum,
        default_value_t = CliParity::None,
        value_name = "MODE",
    )]
    pub parity: CliParity,

    /// Flow-control mode.
    #[arg(
        short = 'f',
        long,
        value_enum,
        default_value_t = CliFlow::None,
        value_name = "MODE",
    )]
    pub flow: CliFlow,

    /// Outbound line-ending mapping. See [`CliLineEnding`] for the rules.
    #[arg(
        long,
        value_enum,
        default_value_t = CliLineEnding::None,
        value_name = "RULE",
    )]
    pub omap: CliLineEnding,

    /// Inbound line-ending mapping. See [`CliLineEnding`] for the rules.
    #[arg(
        long,
        value_enum,
        default_value_t = CliLineEnding::None,
        value_name = "RULE",
    )]
    pub imap: CliLineEnding,

    /// Echo line-ending mapping. Accepted for parity with picocom; the
    /// echo path itself wires up in a later issue.
    #[arg(
        long,
        value_enum,
        default_value_t = CliLineEnding::None,
        value_name = "RULE",
    )]
    pub emap: CliLineEnding,

    /// Do not toggle DTR on startup (suppress the MCU-reset pulse).
    #[arg(long = "no-reset")]
    pub no_reset: bool,

    /// Locally echo characters typed at the keyboard.
    #[arg(long)]
    pub echo: bool,

    /// Command-escape key. Accepts a single char (e.g. `a`) or caret
    /// notation (`^T`, `^A`, ...). Defaults to `^T`.
    #[arg(
        long,
        default_value = "^T",
        value_parser = parse_escape,
        value_name = "CHAR",
    )]
    pub escape: u8,

    /// Suppress non-essential stderr output.
    #[arg(short, long)]
    pub quiet: bool,

    /// Increase diagnostic verbosity (repeatable: `-v`, `-vv`, `-vvv`).
    #[arg(short, long, action = ArgAction::Count)]
    pub verbose: u8,
}

impl Cli {
    /// Projects the parsed arguments into the [`SerialConfig`] consumed by
    /// `rtcom-core`.
    #[must_use]
    pub fn to_serial_config(&self) -> SerialConfig {
        SerialConfig {
            baud_rate: self.baud,
            data_bits: self.data_bits.into(),
            stop_bits: self.stop_bits.into(),
            parity: self.parity.into(),
            flow_control: self.flow.into(),
            read_timeout: DEFAULT_READ_TIMEOUT,
        }
    }
}

/// CLI-facing data-bit enum (keeps `clap` shape concerns out of `rtcom-core`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CliDataBits {
    /// 5 data bits.
    #[value(name = "5")]
    Five,
    /// 6 data bits.
    #[value(name = "6")]
    Six,
    /// 7 data bits.
    #[value(name = "7")]
    Seven,
    /// 8 data bits (default).
    #[value(name = "8")]
    Eight,
}

impl From<CliDataBits> for DataBits {
    fn from(v: CliDataBits) -> Self {
        match v {
            CliDataBits::Five => Self::Five,
            CliDataBits::Six => Self::Six,
            CliDataBits::Seven => Self::Seven,
            CliDataBits::Eight => Self::Eight,
        }
    }
}

/// CLI-facing stop-bit enum.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CliStopBits {
    /// One stop bit (default).
    #[value(name = "1")]
    One,
    /// Two stop bits.
    #[value(name = "2")]
    Two,
}

impl From<CliStopBits> for StopBits {
    fn from(v: CliStopBits) -> Self {
        match v {
            CliStopBits::One => Self::One,
            CliStopBits::Two => Self::Two,
        }
    }
}

/// CLI-facing parity enum.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CliParity {
    /// No parity (default).
    None,
    /// Even parity.
    Even,
    /// Odd parity.
    Odd,
    /// Mark parity (parity bit always 1).
    Mark,
    /// Space parity (parity bit always 0).
    Space,
}

impl From<CliParity> for Parity {
    fn from(v: CliParity) -> Self {
        match v {
            CliParity::None => Self::None,
            CliParity::Even => Self::Even,
            CliParity::Odd => Self::Odd,
            CliParity::Mark => Self::Mark,
            CliParity::Space => Self::Space,
        }
    }
}

/// CLI-facing line-ending mapping enum (used by `--omap`, `--imap`,
/// `--emap`). Names follow the picocom convention.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CliLineEnding {
    /// No transformation (default).
    None,
    /// LF → CRLF (picocom `crlf`).
    Crlf,
    /// CR → CRLF (picocom `lfcr`).
    Lfcr,
    /// Drop CR (picocom `igncr`).
    Igncr,
    /// Drop LF (picocom `ignlf`).
    Ignlf,
}

impl From<CliLineEnding> for LineEnding {
    fn from(_v: CliLineEnding) -> Self {
        todo!("CliLineEnding -> LineEnding mapping lands in the green commit")
    }
}

/// CLI-facing flow-control enum.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CliFlow {
    /// No flow control (default).
    None,
    /// Hardware RTS/CTS flow control.
    #[value(name = "hw")]
    Hardware,
    /// Software XON/XOFF flow control.
    #[value(name = "sw")]
    Software,
}

impl From<CliFlow> for FlowControl {
    fn from(v: CliFlow) -> Self {
        match v {
            CliFlow::None => Self::None,
            CliFlow::Hardware => Self::Hardware,
            CliFlow::Software => Self::Software,
        }
    }
}

/// Parses an escape-key specification.
///
/// Accepts either a single ASCII character (interpreted literally) or caret
/// notation `^X` where `X` maps to the matching ASCII control byte
/// (`'A'` → `0x01`, `'T'` → `0x14`, `'@'` → `0x00`, `'_'` → `0x1f`).
///
/// # Errors
///
/// Returns an error string suitable for `clap` to print if the spec is
/// neither a single char nor a valid caret form.
fn parse_escape(s: &str) -> Result<u8, String> {
    let bytes = s.as_bytes();
    match bytes.len() {
        1 => Ok(bytes[0]),
        2 if bytes[0] == b'^' => {
            let c = bytes[1];
            // Control characters map to 0x00..=0x1f, i.e. ASCII '@'..='_' XOR 0x40.
            if (b'@'..=b'_').contains(&c) || c.is_ascii_lowercase() {
                Ok(c.to_ascii_uppercase() ^ 0x40)
            } else {
                Err(format!(
                    "caret escape '{s}' must be ^@..^_ or ^a..^z, got '{}'",
                    c as char
                ))
            }
        }
        _ => Err(format!(
            "escape must be a single char or ^X caret form (got '{s}')"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_are_115200_8n1() {
        let cli = Cli::parse_from(["rtcom", "/dev/ttyUSB0"]);
        assert_eq!(cli.device, "/dev/ttyUSB0");
        assert_eq!(cli.baud, 115_200);
        assert_eq!(cli.data_bits, CliDataBits::Eight);
        assert_eq!(cli.stop_bits, CliStopBits::One);
        assert_eq!(cli.parity, CliParity::None);
        assert_eq!(cli.flow, CliFlow::None);
        assert!(!cli.no_reset);
        assert!(!cli.echo);
        assert!(!cli.quiet);
        assert_eq!(cli.verbose, 0);
        assert_eq!(cli.escape, 0x14); // ^T
    }

    #[test]
    fn parses_baud_and_parity() {
        let cli = Cli::parse_from(["rtcom", "/dev/ttyUSB0", "-b", "9600", "-p", "even"]);
        assert_eq!(cli.baud, 9600);
        assert_eq!(cli.parity, CliParity::Even);
    }

    #[test]
    fn parses_all_framing_options() {
        let cli = Cli::parse_from([
            "rtcom",
            "/dev/ttyUSB0",
            "-b",
            "921600",
            "-d",
            "7",
            "-s",
            "2",
            "-p",
            "odd",
            "-f",
            "hw",
        ]);
        assert_eq!(cli.baud, 921_600);
        assert_eq!(cli.data_bits, CliDataBits::Seven);
        assert_eq!(cli.stop_bits, CliStopBits::Two);
        assert_eq!(cli.parity, CliParity::Odd);
        assert_eq!(cli.flow, CliFlow::Hardware);
    }

    #[test]
    fn boolean_flags_toggle() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--no-reset", "--echo", "-q"]);
        assert!(cli.no_reset);
        assert!(cli.echo);
        assert!(cli.quiet);
    }

    #[test]
    fn verbose_counts_occurrences() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "-vvv"]);
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn missing_device_is_an_error() {
        let res = Cli::try_parse_from(["rtcom"]);
        assert!(res.is_err());
    }

    #[test]
    fn rejects_invalid_parity_value() {
        let res = Cli::try_parse_from(["rtcom", "/dev/x", "-p", "bogus"]);
        assert!(res.is_err());
    }

    #[test]
    fn rejects_invalid_flow_value() {
        let res = Cli::try_parse_from(["rtcom", "/dev/x", "-f", "xon"]);
        assert!(res.is_err());
    }

    #[test]
    fn escape_caret_notation_maps_to_control_char() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--escape", "^T"]);
        assert_eq!(cli.escape, 0x14);
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--escape", "^A"]);
        assert_eq!(cli.escape, 0x01);
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--escape", "^@"]);
        assert_eq!(cli.escape, 0x00);
    }

    #[test]
    fn escape_lowercase_caret_also_valid() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--escape", "^t"]);
        assert_eq!(cli.escape, 0x14);
    }

    #[test]
    fn escape_single_char_taken_verbatim() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--escape", "a"]);
        assert_eq!(cli.escape, b'a');
    }

    #[test]
    fn escape_empty_or_oversized_rejected() {
        assert!(Cli::try_parse_from(["rtcom", "/dev/x", "--escape", ""]).is_err());
        assert!(Cli::try_parse_from(["rtcom", "/dev/x", "--escape", "abc"]).is_err());
        assert!(Cli::try_parse_from(["rtcom", "/dev/x", "--escape", "^!"]).is_err());
    }

    #[test]
    fn line_ending_options_default_to_none() {
        let cli = Cli::parse_from(["rtcom", "/dev/x"]);
        assert_eq!(cli.omap, CliLineEnding::None);
        assert_eq!(cli.imap, CliLineEnding::None);
        assert_eq!(cli.emap, CliLineEnding::None);
    }

    #[test]
    fn omap_imap_emap_parse_each_value() {
        let cli = Cli::parse_from([
            "rtcom", "/dev/x", "--omap", "crlf", "--imap", "igncr", "--emap", "lfcr",
        ]);
        assert_eq!(cli.omap, CliLineEnding::Crlf);
        assert_eq!(cli.imap, CliLineEnding::Igncr);
        assert_eq!(cli.emap, CliLineEnding::Lfcr);
    }

    #[test]
    fn rejects_invalid_line_ending_value() {
        assert!(Cli::try_parse_from(["rtcom", "/dev/x", "--omap", "weird"]).is_err());
    }

    #[test]
    fn cli_line_ending_projects_to_core_line_ending() {
        assert_eq!(LineEnding::from(CliLineEnding::None), LineEnding::None);
        assert_eq!(LineEnding::from(CliLineEnding::Crlf), LineEnding::AddCrToLf);
        assert_eq!(LineEnding::from(CliLineEnding::Lfcr), LineEnding::AddLfToCr);
        assert_eq!(LineEnding::from(CliLineEnding::Igncr), LineEnding::DropCr);
        assert_eq!(LineEnding::from(CliLineEnding::Ignlf), LineEnding::DropLf);
    }

    #[test]
    fn projects_into_serial_config() {
        let cli = Cli::parse_from([
            "rtcom", "/dev/x", "-b", "57600", "-d", "7", "-s", "2", "-p", "even", "-f", "sw",
        ]);
        let cfg = cli.to_serial_config();
        assert_eq!(cfg.baud_rate, 57_600);
        assert_eq!(cfg.data_bits, DataBits::Seven);
        assert_eq!(cfg.stop_bits, StopBits::Two);
        assert_eq!(cfg.parity, Parity::Even);
        assert_eq!(cfg.flow_control, FlowControl::Software);
    }
}
