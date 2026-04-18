//! Conversion helpers between [`rtcom_config`]'s string-based
//! [`Profile`] and [`rtcom_core`]'s typed [`SerialConfig`] /
//! [`LineEndingConfig`].
//!
//! The profile layer persists each field as a stable TOML string
//! (`"none"`, `"even"`, `"crlf"`, ...) so hand-edited files survive
//! refactors in `rtcom-core`'s enums. This module bridges the two
//! representations in both directions:
//!
//! - [`serial_section_to_config`] / [`serial_config_to_section`] —
//!   serial framing round-trip.
//! - [`line_endings_from_profile`] — pulls the three line-ending
//!   mappers out of a profile's `[line_endings]` section.
//!
//! Unknown strings fall through to sensible defaults rather than
//! erroring: a hand-edited `parity = "quantum"` must never crash rtcom.
//!
//! Both `rtcom-cli` (at startup) and `rtcom-tui::run` (when applying
//! `DialogAction::ReadProfile` or writing back on `ApplyAndSave`) lean
//! on these helpers, which is why they live in the shared `rtcom-tui`
//! crate.

use rtcom_config::{profile::SerialSection, Profile};
use rtcom_core::{
    DataBits, FlowControl, LineEnding, LineEndingConfig, Parity, SerialConfig, StopBits,
    DEFAULT_READ_TIMEOUT,
};

/// Project a profile `[serial]` section into a runtime [`SerialConfig`].
///
/// Unknown parity / flow strings and out-of-range data / stop bit counts
/// fall through to their type-level defaults rather than erroring.
#[must_use]
pub fn serial_section_to_config(s: &SerialSection) -> SerialConfig {
    SerialConfig {
        baud_rate: s.baud,
        data_bits: parse_data_bits(s.data_bits),
        stop_bits: parse_stop_bits(s.stop_bits),
        parity: parse_parity(&s.parity),
        flow_control: parse_flow(&s.flow),
        read_timeout: DEFAULT_READ_TIMEOUT,
    }
}

/// Project a runtime [`SerialConfig`] back into its TOML-facing
/// [`SerialSection`] representation (used when persisting on `--save`
/// or `DialogAction::ApplyAndSave`).
#[must_use]
pub fn serial_config_to_section(c: &SerialConfig) -> SerialSection {
    SerialSection {
        baud: c.baud_rate,
        data_bits: c.data_bits.bits(),
        stop_bits: stop_bits_number(c.stop_bits),
        parity: parity_word(c.parity).into(),
        flow: flow_word(c.flow_control).into(),
    }
}

/// Pull the three line-ending mappers out of a profile's
/// `[line_endings]` section. Unknown strings fall through to
/// [`LineEnding::None`].
#[must_use]
pub fn line_endings_from_profile(p: &Profile) -> LineEndingConfig {
    LineEndingConfig {
        omap: parse_line_ending(&p.line_endings.omap),
        imap: parse_line_ending(&p.line_endings.imap),
        emap: parse_line_ending(&p.line_endings.emap),
    }
}

fn parse_parity(s: &str) -> Parity {
    match s.to_ascii_lowercase().as_str() {
        "even" => Parity::Even,
        "odd" => Parity::Odd,
        "mark" => Parity::Mark,
        "space" => Parity::Space,
        _ => Parity::None,
    }
}

fn parse_flow(s: &str) -> FlowControl {
    match s.to_ascii_lowercase().as_str() {
        "hw" | "hardware" | "rtscts" => FlowControl::Hardware,
        "sw" | "software" | "xonxoff" => FlowControl::Software,
        _ => FlowControl::None,
    }
}

const fn parse_data_bits(n: u8) -> DataBits {
    match n {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

const fn parse_stop_bits(n: u8) -> StopBits {
    match n {
        2 => StopBits::Two,
        _ => StopBits::One,
    }
}

/// Parse a profile-string line-ending rule into [`LineEnding`].
/// Unknown strings fall through to [`LineEnding::None`].
#[must_use]
pub fn parse_line_ending(s: &str) -> LineEnding {
    match s.to_ascii_lowercase().as_str() {
        "crlf" => LineEnding::AddCrToLf,
        "lfcr" => LineEnding::AddLfToCr,
        "igncr" => LineEnding::DropCr,
        "ignlf" => LineEnding::DropLf,
        _ => LineEnding::None,
    }
}

const fn parity_word(p: Parity) -> &'static str {
    match p {
        Parity::None => "none",
        Parity::Even => "even",
        Parity::Odd => "odd",
        Parity::Mark => "mark",
        Parity::Space => "space",
    }
}

const fn flow_word(f: FlowControl) -> &'static str {
    match f {
        FlowControl::None => "none",
        FlowControl::Hardware => "hw",
        FlowControl::Software => "sw",
    }
}

const fn stop_bits_number(s: StopBits) -> u8 {
    match s {
        StopBits::One => 1,
        StopBits::Two => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_section_round_trip() {
        let original = SerialConfig {
            baud_rate: 9600,
            data_bits: DataBits::Seven,
            stop_bits: StopBits::Two,
            parity: Parity::Even,
            flow_control: FlowControl::Hardware,
            read_timeout: DEFAULT_READ_TIMEOUT,
        };
        let section = serial_config_to_section(&original);
        let back = serial_section_to_config(&section);
        assert_eq!(back.baud_rate, original.baud_rate);
        assert_eq!(back.data_bits, original.data_bits);
        assert_eq!(back.stop_bits, original.stop_bits);
        assert_eq!(back.parity, original.parity);
        assert_eq!(back.flow_control, original.flow_control);
    }

    #[test]
    fn unknown_parity_string_falls_back_to_none() {
        let section = SerialSection {
            parity: "quantum".into(),
            ..SerialSection::default()
        };
        let cfg = serial_section_to_config(&section);
        assert_eq!(cfg.parity, Parity::None);
    }

    #[test]
    fn unknown_flow_string_falls_back_to_none() {
        let section = SerialSection {
            flow: "teleport".into(),
            ..SerialSection::default()
        };
        let cfg = serial_section_to_config(&section);
        assert_eq!(cfg.flow_control, FlowControl::None);
    }

    #[test]
    fn out_of_range_data_bits_fall_back_to_eight() {
        let section = SerialSection {
            data_bits: 42,
            ..SerialSection::default()
        };
        let cfg = serial_section_to_config(&section);
        assert_eq!(cfg.data_bits, DataBits::Eight);
    }

    #[test]
    fn out_of_range_stop_bits_fall_back_to_one() {
        let section = SerialSection {
            stop_bits: 9,
            ..SerialSection::default()
        };
        let cfg = serial_section_to_config(&section);
        assert_eq!(cfg.stop_bits, StopBits::One);
    }

    #[test]
    fn line_endings_from_profile_reads_all_three_slots() {
        let mut profile = Profile::default();
        profile.line_endings.omap = "crlf".into();
        profile.line_endings.imap = "igncr".into();
        profile.line_endings.emap = "lfcr".into();
        let le = line_endings_from_profile(&profile);
        assert_eq!(le.omap, LineEnding::AddCrToLf);
        assert_eq!(le.imap, LineEnding::DropCr);
        assert_eq!(le.emap, LineEnding::AddLfToCr);
    }

    #[test]
    fn line_endings_from_profile_default_is_all_none() {
        let profile = Profile::default();
        let le = line_endings_from_profile(&profile);
        assert_eq!(le.omap, LineEnding::None);
        assert_eq!(le.imap, LineEnding::None);
        assert_eq!(le.emap, LineEnding::None);
    }

    #[test]
    fn parse_line_ending_covers_all_known_forms() {
        assert_eq!(parse_line_ending("crlf"), LineEnding::AddCrToLf);
        assert_eq!(parse_line_ending("lfcr"), LineEnding::AddLfToCr);
        assert_eq!(parse_line_ending("igncr"), LineEnding::DropCr);
        assert_eq!(parse_line_ending("ignlf"), LineEnding::DropLf);
        assert_eq!(parse_line_ending("none"), LineEnding::None);
        assert_eq!(parse_line_ending("bogus"), LineEnding::None);
    }

    #[test]
    fn serial_config_to_section_emits_stable_strings() {
        let cfg = SerialConfig {
            baud_rate: 9600,
            data_bits: DataBits::Seven,
            stop_bits: StopBits::Two,
            parity: Parity::Even,
            flow_control: FlowControl::Hardware,
            read_timeout: DEFAULT_READ_TIMEOUT,
        };
        let section = serial_config_to_section(&cfg);
        assert_eq!(section.baud, 9600);
        assert_eq!(section.data_bits, 7);
        assert_eq!(section.stop_bits, 2);
        assert_eq!(section.parity, "even");
        assert_eq!(section.flow, "hw");
    }
}
