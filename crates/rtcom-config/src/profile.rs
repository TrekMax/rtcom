//! `Profile` struct and TOML-persisted sub-sections for rtcom settings.

use serde::{Deserialize, Serialize};

/// Top-level rtcom profile persisted to TOML.
///
/// Unknown TOML keys are silently ignored (serde default), and missing leaf
/// fields within a declared section fall back to the section's [`Default`]
/// impl — so hand-edited profiles with partial overrides keep working across
/// rtcom versions that add fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Profile {
    /// Serial-port settings (baud, framing, flow control).
    #[serde(default)]
    pub serial: SerialSection,
    /// Line-ending translation (CR/LF) on input, output, and echo paths.
    #[serde(default)]
    pub line_endings: LineEndingsSection,
    /// Modem control line startup policy.
    #[serde(default)]
    pub modem: ModemSection,
    /// Screen / TUI rendering preferences.
    #[serde(default)]
    pub screen: ScreenSection,
}

/// Serial-port settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SerialSection {
    /// Baud rate in bits per second (e.g. `115_200`).
    pub baud: u32,
    /// Number of data bits per frame (5..=8).
    pub data_bits: u8,
    /// Number of stop bits (1 or 2).
    pub stop_bits: u8,
    /// Parity: `none`, `even`, `odd`, `mark`, or `space`.
    pub parity: String,
    /// Flow control: `none`, `hw` (RTS/CTS), or `sw` (XON/XOFF).
    pub flow: String,
}

impl Default for SerialSection {
    fn default() -> Self {
        Self {
            baud: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".into(),
            flow: "none".into(),
        }
    }
}

/// Line-ending mappers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineEndingsSection {
    /// Output map applied to bytes sent to the device.
    pub omap: String,
    /// Input map applied to bytes received from the device.
    pub imap: String,
    /// Echo map applied to locally-echoed bytes.
    pub emap: String,
}

impl Default for LineEndingsSection {
    fn default() -> Self {
        Self {
            omap: "none".into(),
            imap: "none".into(),
            emap: "none".into(),
        }
    }
}

/// Modem control line startup policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModemSection {
    /// Initial DTR state: `unchanged`, `raise`, or `lower`.
    pub initial_dtr: String,
    /// Initial RTS state: `unchanged`, `raise`, or `lower`.
    pub initial_rts: String,
}

impl Default for ModemSection {
    fn default() -> Self {
        Self {
            initial_dtr: "unchanged".into(),
            initial_rts: "unchanged".into(),
        }
    }
}

/// Screen / TUI rendering preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenSection {
    /// How modal dialogs render over the terminal stream.
    pub modal_style: ModalStyle,
    /// Number of scrollback rows retained in the TUI buffer.
    pub scrollback_rows: usize,
    /// Lines scrolled per mouse-wheel notch in the serial pane.
    ///
    /// Values less than 1 are treated as 1 at runtime so the wheel
    /// always has a visible effect. v0.2 has no menu control for this
    /// — hand-edit the TOML to change it; a menu-editable control is
    /// deferred to v0.2.1.
    pub wheel_scroll_lines: u16,
}

impl Default for ScreenSection {
    fn default() -> Self {
        Self {
            modal_style: ModalStyle::Overlay,
            scrollback_rows: 10_000,
            wheel_scroll_lines: 3,
        }
    }
}

/// How modal dialogs (menus, prompts) render over the terminal stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModalStyle {
    /// Draw on top of the live stream without altering its contents.
    #[default]
    Overlay,
    /// Overlay with the background stream dimmed.
    DimmedOverlay,
    /// Take over the full terminal while active.
    Fullscreen,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_default_values() {
        let p = Profile::default();
        assert_eq!(p.serial.baud, 115_200);
        assert_eq!(p.serial.data_bits, 8);
        assert_eq!(p.screen.modal_style, ModalStyle::Overlay);
        assert_eq!(p.screen.scrollback_rows, 10_000);
        assert_eq!(p.screen.wheel_scroll_lines, 3);
    }

    #[test]
    fn profile_partial_screen_section_keeps_wheel_default() {
        // Simulate an existing profile written before T24 — the file
        // has modal_style but no wheel_scroll_lines. Serde's
        // `#[serde(default)]` must fall back to the section default
        // without failing to parse.
        let partial = r#"
            [screen]
            modal_style = "fullscreen"
        "#;
        let parsed: Profile = toml::from_str(partial).expect("parse");
        assert_eq!(parsed.screen.modal_style, ModalStyle::Fullscreen);
        assert_eq!(parsed.screen.wheel_scroll_lines, 3);
    }

    #[test]
    fn profile_roundtrip_toml() {
        let original = Profile::default();
        let serialized = toml::to_string(&original).expect("serialize");
        let parsed: Profile = toml::from_str(&serialized).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn profile_unknown_keys_are_dropped() {
        let with_unknown = r#"
            [serial]
            baud = 9600
            unknown_field = "ignored"
            data_bits = 8
            stop_bits = 1
            parity = "none"
            flow = "none"
        "#;
        let parsed: Profile = toml::from_str(with_unknown).expect("parse");
        assert_eq!(parsed.serial.baud, 9600);
    }

    #[test]
    fn profile_partial_section_uses_defaults_for_missing_leaf_fields() {
        let partial = r"
            [serial]
            baud = 9600
        ";
        let parsed: Profile = toml::from_str(partial).expect("parse");
        assert_eq!(parsed.serial.baud, 9600);
        assert_eq!(parsed.serial.data_bits, 8); // default preserved
        assert_eq!(parsed.serial.parity, "none"); // default preserved
    }
}
