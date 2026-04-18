//! Command-line argument parsing for the `rtcom` binary.
//!
//! Parsing lives here so `main.rs` stays a thin entry point. The [`Cli`]
//! struct mirrors what `clap` reads from `argv`; [`Cli::to_serial_config`]
//! projects it into [`rtcom_core::SerialConfig`] for the session layer.

use std::path::PathBuf;

use clap::{ArgAction, Parser, ValueEnum};

use rtcom_config::Profile;
use rtcom_core::{
    DataBits, FlowControl, LineEnding, Parity, SerialConfig, StopBits, DEFAULT_READ_TIMEOUT,
};

/// Parsed `rtcom` command-line invocation.
///
/// `struct_excessive_bools` is allowed because each boolean here maps
/// 1-to-1 to a CLI flag the user expects to type (`--no-reset`,
/// `--echo`, `--lower-dtr`, ...). Collapsing into enums or state
/// objects would just make the call site harder to read.
#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Debug, Clone)]
#[command(
    name = "rtcom",
    // `RTCOM_VERSION` is computed by build.rs and looks like
    // "0.1.0 (abc12345)" for git checkouts or just "0.1.0" for
    // crates.io tarball builds.
    version = env!("RTCOM_VERSION"),
    about = "Rust Terminal Communication — modern serial terminal",
    long_about = None,
)]
pub struct Cli {
    /// Serial device path, e.g. `/dev/ttyUSB0` (Linux) or `COM3` (Windows).
    pub device: String,

    /// Baud rate in bits per second. When omitted, rtcom uses the profile's
    /// value (default profile: 115200).
    #[arg(short, long, value_name = "RATE")]
    pub baud: Option<u32>,

    /// Data bits per frame. When omitted, rtcom uses the profile's value
    /// (default profile: 8).
    #[arg(short = 'd', long = "databits", value_enum, value_name = "BITS")]
    pub data_bits: Option<CliDataBits>,

    /// Stop bits per frame. When omitted, rtcom uses the profile's value
    /// (default profile: 1).
    #[arg(short = 's', long = "stopbits", value_enum, value_name = "BITS")]
    pub stop_bits: Option<CliStopBits>,

    /// Parity mode. When omitted, rtcom uses the profile's value (default
    /// profile: none).
    #[arg(short = 'p', long, value_enum, value_name = "MODE")]
    pub parity: Option<CliParity>,

    /// Flow-control mode. When omitted, rtcom uses the profile's value
    /// (default profile: none).
    #[arg(short = 'f', long, value_enum, value_name = "MODE")]
    pub flow: Option<CliFlow>,

    /// Outbound line-ending mapping. See [`CliLineEnding`] for the rules.
    /// When omitted, rtcom uses the profile's value (default profile: none).
    #[arg(long, value_enum, value_name = "RULE")]
    pub omap: Option<CliLineEnding>,

    /// Inbound line-ending mapping. See [`CliLineEnding`] for the rules.
    /// When omitted, rtcom uses the profile's value (default profile: none).
    #[arg(long, value_enum, value_name = "RULE")]
    pub imap: Option<CliLineEnding>,

    /// Echo line-ending mapping. Accepted for parity with picocom; the
    /// echo path itself wires up in a later issue. When omitted, rtcom
    /// uses the profile's value (default profile: none).
    #[arg(long, value_enum, value_name = "RULE")]
    pub emap: Option<CliLineEnding>,

    /// Deassert DTR immediately after opening the device (picocom
    /// `--lower-dtr`). Useful for boards that wire DTR to reset/boot
    /// pins — keeps the MCU from rebooting when rtcom opens the port.
    #[arg(long, conflicts_with = "raise_dtr")]
    pub lower_dtr: bool,

    /// Assert DTR immediately after opening the device (picocom
    /// `--raise-dtr`). Mostly useful when a previous session left
    /// DTR low and you want to put it back.
    #[arg(long, conflicts_with = "lower_dtr")]
    pub raise_dtr: bool,

    /// Deassert RTS immediately after opening the device (picocom
    /// `--lower-rts`). Same MCU-reset rationale as `--lower-dtr`.
    #[arg(long, conflicts_with = "raise_rts")]
    pub lower_rts: bool,

    /// Assert RTS immediately after opening the device (picocom
    /// `--raise-rts`).
    #[arg(long, conflicts_with = "lower_rts")]
    pub raise_rts: bool,

    /// Do not toggle DTR on startup (suppress the MCU-reset pulse).
    #[arg(long = "no-reset")]
    pub no_reset: bool,

    /// Locally echo characters typed at the keyboard.
    #[arg(long)]
    pub echo: bool,

    /// Command-escape key. Accepts a single char (e.g. `a`) or caret
    /// notation (`^A`, `^T`, ...). Defaults to `^A` (Ctrl-A) — picocom's
    /// historical default; `^T` is intercepted by some terminals
    /// (tmux's default prefix in some configs, "new tab" in others).
    #[arg(
        long,
        default_value = "^A",
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

    /// Path to the profile TOML file. When omitted, rtcom uses the platform
    /// default (`$XDG_CONFIG_HOME/rtcom/default.toml` on Linux, equivalent
    /// on macOS / Windows).
    #[arg(short = 'c', long = "config", value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Write the effective configuration back to the profile on startup.
    /// Merges CLI-provided fields over the loaded profile, then persists
    /// the result before starting the session. Fails hard when no profile
    /// path is available (no `-c PATH` and no discoverable home).
    ///
    /// **v0.2 scope:** only the `[serial]` section is updated on save.
    /// `--omap/--imap/--emap`, modem lines and screen options pass through
    /// from the loaded profile unchanged. Menu-driven line-ending and modem
    /// persistence lands with the Line endings / Modem control dialogs in
    /// later v0.2 tasks.
    #[arg(long = "save")]
    pub save: bool,
}

impl Cli {
    /// Projects the parsed arguments into the [`SerialConfig`] consumed by
    /// `rtcom-core`, resolving each field with the merge rule
    /// `defaults < profile < CLI`.
    ///
    /// For every profile-backed field, a `Some(_)` on the CLI wins;
    /// otherwise the corresponding `profile.serial` value is used.
    /// Unknown strings in the profile (e.g. hand-edited garbage) fall
    /// through to the CLI/core default rather than panicking.
    #[must_use]
    pub fn to_serial_config(&self, profile: &Profile) -> SerialConfig {
        SerialConfig {
            baud_rate: self.baud.unwrap_or(profile.serial.baud),
            data_bits: self.data_bits.map_or_else(
                || data_bits_from_profile(profile.serial.data_bits),
                Into::into,
            ),
            stop_bits: self.stop_bits.map_or_else(
                || stop_bits_from_profile(profile.serial.stop_bits),
                Into::into,
            ),
            parity: self
                .parity
                .map_or_else(|| parity_from_profile(&profile.serial.parity), Into::into),
            flow_control: self
                .flow
                .map_or_else(|| flow_from_profile(&profile.serial.flow), Into::into),
            read_timeout: DEFAULT_READ_TIMEOUT,
        }
    }

    /// Resolves the outbound line-ending rule with CLI > profile > none.
    #[must_use]
    pub fn resolved_omap(&self, profile: &Profile) -> LineEnding {
        self.omap.map_or_else(
            || line_ending_from_profile(&profile.line_endings.omap),
            Into::into,
        )
    }

    /// Resolves the inbound line-ending rule with CLI > profile > none.
    #[must_use]
    pub fn resolved_imap(&self, profile: &Profile) -> LineEnding {
        self.imap.map_or_else(
            || line_ending_from_profile(&profile.line_endings.imap),
            Into::into,
        )
    }

    // Note: `emap` has no runtime wire-up yet (the echo path lands in a
    // later issue), so there's no `resolved_emap` here. The field is
    // parsed for picocom parity and round-tripped through `--save` via
    // the profile layer once the menu-editable line-endings task
    // (Task 13) wires it through.
}

/// Translates a profile-string parity (`"none"`, `"even"`, ...) into the core
/// enum. Unknown strings fall through to [`Parity::None`] rather than
/// erroring — hand-edited TOML must never crash rtcom.
fn parity_from_profile(s: &str) -> Parity {
    match s.to_ascii_lowercase().as_str() {
        "even" => Parity::Even,
        "odd" => Parity::Odd,
        "mark" => Parity::Mark,
        "space" => Parity::Space,
        _ => Parity::None,
    }
}

/// Translates a profile-string flow-control mode (`"none"`, `"hw"`, `"sw"`)
/// into the core enum. Unknown strings fall through to [`FlowControl::None`].
fn flow_from_profile(s: &str) -> FlowControl {
    match s.to_ascii_lowercase().as_str() {
        "hw" | "hardware" | "rtscts" => FlowControl::Hardware,
        "sw" | "software" | "xonxoff" => FlowControl::Software,
        _ => FlowControl::None,
    }
}

/// Translates a profile-numeric data-bits value into the core enum. Values
/// outside 5..=8 fall through to 8.
const fn data_bits_from_profile(n: u8) -> DataBits {
    match n {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

/// Translates a profile-numeric stop-bits value into the core enum. Values
/// outside {1, 2} fall through to 1.
const fn stop_bits_from_profile(n: u8) -> StopBits {
    match n {
        2 => StopBits::Two,
        _ => StopBits::One,
    }
}

/// Translates a profile-string line-ending rule into the core enum.
/// Unknown strings fall through to [`LineEnding::None`].
fn line_ending_from_profile(s: &str) -> LineEnding {
    match s.to_ascii_lowercase().as_str() {
        "crlf" => LineEnding::AddCrToLf,
        "lfcr" => LineEnding::AddLfToCr,
        "igncr" => LineEnding::DropCr,
        "ignlf" => LineEnding::DropLf,
        _ => LineEnding::None,
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
    fn from(v: CliLineEnding) -> Self {
        match v {
            CliLineEnding::None => Self::None,
            CliLineEnding::Crlf => Self::AddCrToLf,
            CliLineEnding::Lfcr => Self::AddLfToCr,
            CliLineEnding::Igncr => Self::DropCr,
            CliLineEnding::Ignlf => Self::DropLf,
        }
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
        // Profile-backed fields default to None when unspecified so the
        // profile layer (or its built-in defaults) can fill them in.
        assert_eq!(cli.baud, None);
        assert_eq!(cli.data_bits, None);
        assert_eq!(cli.stop_bits, None);
        assert_eq!(cli.parity, None);
        assert_eq!(cli.flow, None);
        assert!(!cli.no_reset);
        assert!(!cli.echo);
        assert!(!cli.quiet);
        assert_eq!(cli.verbose, 0);
        assert_eq!(cli.escape, 0x01); // ^A
    }

    #[test]
    fn parses_baud_and_parity() {
        let cli = Cli::parse_from(["rtcom", "/dev/ttyUSB0", "-b", "9600", "-p", "even"]);
        assert_eq!(cli.baud, Some(9600));
        assert_eq!(cli.parity, Some(CliParity::Even));
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
        assert_eq!(cli.baud, Some(921_600));
        assert_eq!(cli.data_bits, Some(CliDataBits::Seven));
        assert_eq!(cli.stop_bits, Some(CliStopBits::Two));
        assert_eq!(cli.parity, Some(CliParity::Odd));
        assert_eq!(cli.flow, Some(CliFlow::Hardware));
    }

    #[test]
    fn baud_is_none_when_not_specified() {
        let cli = Cli::parse_from(["rtcom", "/dev/ttyUSB0"]);
        assert_eq!(cli.baud, None);
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
    fn line_control_flags_default_to_false() {
        let cli = Cli::parse_from(["rtcom", "/dev/x"]);
        assert!(!cli.lower_dtr);
        assert!(!cli.raise_dtr);
        assert!(!cli.lower_rts);
        assert!(!cli.raise_rts);
    }

    #[test]
    fn lower_dtr_parses_alone() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--lower-dtr"]);
        assert!(cli.lower_dtr);
        assert!(!cli.raise_dtr);
    }

    #[test]
    fn raise_dtr_parses_alone() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--raise-dtr"]);
        assert!(cli.raise_dtr);
        assert!(!cli.lower_dtr);
    }

    #[test]
    fn lower_rts_parses_alone() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--lower-rts"]);
        assert!(cli.lower_rts);
        assert!(!cli.raise_rts);
    }

    #[test]
    fn raise_rts_parses_alone() {
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--raise-rts"]);
        assert!(cli.raise_rts);
        assert!(!cli.lower_rts);
    }

    #[test]
    fn lower_dtr_and_raise_dtr_are_mutually_exclusive() {
        let res = Cli::try_parse_from(["rtcom", "/dev/x", "--lower-dtr", "--raise-dtr"]);
        assert!(res.is_err(), "expected clap to reject the conflict");
    }

    #[test]
    fn lower_rts_and_raise_rts_are_mutually_exclusive() {
        let res = Cli::try_parse_from(["rtcom", "/dev/x", "--lower-rts", "--raise-rts"]);
        assert!(res.is_err(), "expected clap to reject the conflict");
    }

    #[test]
    fn lower_dtr_does_not_conflict_with_lower_rts() {
        // Crossing the DTR/RTS axes is the canonical
        // `--lower-dtr --lower-rts` invocation. Must remain valid.
        let cli = Cli::parse_from(["rtcom", "/dev/x", "--lower-dtr", "--lower-rts"]);
        assert!(cli.lower_dtr);
        assert!(cli.lower_rts);
    }

    #[test]
    fn line_ending_options_default_to_none() {
        let cli = Cli::parse_from(["rtcom", "/dev/x"]);
        // With profile merging, an unspecified map on the CLI now means
        // "take from profile, else none" — represented as `None` here.
        assert_eq!(cli.omap, None);
        assert_eq!(cli.imap, None);
        assert_eq!(cli.emap, None);
    }

    #[test]
    fn omap_imap_emap_parse_each_value() {
        let cli = Cli::parse_from([
            "rtcom", "/dev/x", "--omap", "crlf", "--imap", "igncr", "--emap", "lfcr",
        ]);
        assert_eq!(cli.omap, Some(CliLineEnding::Crlf));
        assert_eq!(cli.imap, Some(CliLineEnding::Igncr));
        assert_eq!(cli.emap, Some(CliLineEnding::Lfcr));
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
        let profile = rtcom_config::Profile::default();
        let cfg = cli.to_serial_config(&profile);
        assert_eq!(cfg.baud_rate, 57_600);
        assert_eq!(cfg.data_bits, DataBits::Seven);
        assert_eq!(cfg.stop_bits, StopBits::Two);
        assert_eq!(cfg.parity, Parity::Even);
        assert_eq!(cfg.flow_control, FlowControl::Software);
    }

    #[test]
    fn to_serial_config_falls_back_to_profile_when_cli_unspecified() {
        // CLI omits everything — profile values win (defaults < profile).
        let cli = Cli::parse_from(["rtcom", "/dev/x"]);
        let mut profile = rtcom_config::Profile::default();
        profile.serial.baud = 9600;
        profile.serial.parity = "even".into();
        profile.serial.flow = "hw".into();
        let cfg = cli.to_serial_config(&profile);
        assert_eq!(cfg.baud_rate, 9600);
        assert_eq!(cfg.parity, Parity::Even);
        assert_eq!(cfg.flow_control, FlowControl::Hardware);
    }

    #[test]
    fn to_serial_config_cli_overrides_profile() {
        // CLI fields win over profile (profile < CLI).
        let cli = Cli::parse_from(["rtcom", "/dev/x", "-b", "460800"]);
        let mut profile = rtcom_config::Profile::default();
        profile.serial.baud = 9600;
        let cfg = cli.to_serial_config(&profile);
        assert_eq!(cfg.baud_rate, 460_800);
    }

    #[test]
    fn to_serial_config_tolerates_unknown_profile_strings() {
        // Malformed profile strings fall through to defaults rather
        // than panicking — user-edited TOML shouldn't ever crash rtcom.
        let cli = Cli::parse_from(["rtcom", "/dev/x"]);
        let mut profile = rtcom_config::Profile::default();
        profile.serial.parity = "bogus".into();
        profile.serial.flow = "also-bogus".into();
        let cfg = cli.to_serial_config(&profile);
        assert_eq!(cfg.parity, Parity::None);
        assert_eq!(cfg.flow_control, FlowControl::None);
    }

    // -------- New flags (Task 3) --------

    #[test]
    fn cli_accepts_config_path() {
        let args = Cli::try_parse_from(["rtcom", "/dev/ttyUSB0", "-c", "/tmp/alt.toml"]).unwrap();
        assert_eq!(
            args.config.as_deref(),
            Some(std::path::Path::new("/tmp/alt.toml"))
        );
    }

    #[test]
    fn cli_save_flag_requires_device() {
        let err = Cli::try_parse_from(["rtcom", "--save"]).unwrap_err();
        // clap rejects because positional `device` is required.
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("device") || msg.contains("required"),
            "unexpected clap error: {err}"
        );
    }

    #[test]
    fn cli_save_with_device_parses() {
        let args = Cli::try_parse_from(["rtcom", "/dev/ttyUSB0", "-b", "9600", "--save"]).unwrap();
        assert!(args.save);
        assert_eq!(args.baud, Some(9600));
    }
}
