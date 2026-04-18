//! Serial line configuration and modem-status types.
//!
//! These are the framing and flow parameters every [`SerialDevice`] needs to
//! expose. They intentionally mirror the classic `termios` model (data bits /
//! stop bits / parity / flow control) so behaviour lines up with user
//! expectations inherited from `picocom` and `tio`.
//!
//! [`SerialDevice`]: crate::SerialDevice

use std::time::Duration;

/// Default read poll timeout used by blocking backends.
///
/// The async backend does not gate reads on this value, but it is stored so
/// [`SerialConfig`] can be printed verbatim and so future blocking fallbacks
/// behave consistently.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(100);

/// Number of data bits per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataBits {
    /// 5 data bits per frame.
    Five,
    /// 6 data bits per frame.
    Six,
    /// 7 data bits per frame.
    Seven,
    /// 8 data bits per frame (the default and the only mode most USB-serial
    /// bridges support).
    Eight,
}

impl DataBits {
    /// Returns the numeric width in bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::Five => 5,
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
        }
    }
}

/// Number of stop bits appended after the data / parity bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopBits {
    /// One stop bit (default).
    One,
    /// Two stop bits.
    Two,
}

/// Parity mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parity {
    /// No parity bit (default).
    None,
    /// Even parity.
    Even,
    /// Odd parity.
    Odd,
    /// Mark parity (parity bit always 1). Rare; not supported on all
    /// platforms.
    Mark,
    /// Space parity (parity bit always 0). Rare; not supported on all
    /// platforms.
    Space,
}

/// Flow-control mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowControl {
    /// No flow control (default).
    None,
    /// Hardware flow control using the RTS/CTS lines.
    Hardware,
    /// Software flow control using XON/XOFF bytes (0x11 / 0x13).
    Software,
}

/// Snapshot of the input-side modem control lines.
///
/// Returned by [`SerialDevice::modem_status`](crate::SerialDevice::modem_status).
/// Each field is `true` when the corresponding line is asserted. The struct
/// is deliberately a flat record of four booleans — it mirrors the hardware
/// register one-to-one — so the `struct_excessive_bools` lint does not apply.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModemStatus {
    /// Clear to Send.
    pub cts: bool,
    /// Data Set Ready.
    pub dsr: bool,
    /// Ring Indicator.
    pub ri: bool,
    /// Carrier Detect.
    pub cd: bool,
}

/// Snapshot of the modem output lines as rtcom knows them.
///
/// Unlike [`ModemStatus`] (which reflects the input-side lines CTS / DSR /
/// RI / CD and requires polling the device), the output lines DTR and RTS
/// are driven by rtcom itself — so the current state is simply whatever
/// the `Session` last wrote. The TUI modem-control dialog (v0.2 task 14)
/// consumes this snapshot for its read-only "Current output lines"
/// display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModemLineSnapshot {
    /// Data Terminal Ready output line: `true` when asserted.
    pub dtr: bool,
    /// Request To Send output line: `true` when asserted.
    pub rts: bool,
}

/// Full serial-link configuration.
///
/// `SerialConfig` is what the CLI builds from command-line flags (see
/// `rtcom-cli` Issue #3) and what the session orchestrator hands to a
/// [`SerialDevice`](crate::SerialDevice) at open time. It is also what
/// [`SerialDevice::config`](crate::SerialDevice::config) returns so runtime
/// code can display or serialize the current link parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerialConfig {
    /// Baud rate in bits per second.
    pub baud_rate: u32,
    /// Data bits per frame.
    pub data_bits: DataBits,
    /// Stop bits per frame.
    pub stop_bits: StopBits,
    /// Parity mode.
    pub parity: Parity,
    /// Flow-control mode.
    pub flow_control: FlowControl,
    /// Timeout used by blocking reads (unused on the async path, but kept so
    /// `config()` remains a faithful record of the requested settings).
    pub read_timeout: Duration,
}

impl Default for SerialConfig {
    /// Returns the tio/picocom-compatible default: `115200 8N1`, no flow control.
    fn default() -> Self {
        Self {
            baud_rate: 115_200,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            read_timeout: DEFAULT_READ_TIMEOUT,
        }
    }
}

impl SerialConfig {
    /// Validates that the configuration is internally consistent.
    ///
    /// Currently only rejects a zero baud rate; more checks (e.g. disallowing
    /// `Mark`/`Space` on platforms that don't implement them) may be added in
    /// the future.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidConfig`](crate::Error::InvalidConfig) if the
    /// configuration cannot be used to open a device.
    pub fn validate(&self) -> crate::Result<()> {
        if self.baud_rate == 0 {
            return Err(crate::Error::InvalidConfig(
                "baud_rate must be non-zero".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_115200_8n1_no_flow() {
        let cfg = SerialConfig::default();
        assert_eq!(cfg.baud_rate, 115_200);
        assert_eq!(cfg.data_bits, DataBits::Eight);
        assert_eq!(cfg.stop_bits, StopBits::One);
        assert_eq!(cfg.parity, Parity::None);
        assert_eq!(cfg.flow_control, FlowControl::None);
    }

    #[test]
    fn data_bits_width_matches_enum() {
        assert_eq!(DataBits::Five.bits(), 5);
        assert_eq!(DataBits::Six.bits(), 6);
        assert_eq!(DataBits::Seven.bits(), 7);
        assert_eq!(DataBits::Eight.bits(), 8);
    }

    #[test]
    fn validate_rejects_zero_baud() {
        let cfg = SerialConfig {
            baud_rate: 0,
            ..SerialConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_accepts_default() {
        assert!(SerialConfig::default().validate().is_ok());
    }

    #[test]
    fn modem_line_snapshot_default_both_false() {
        let s = ModemLineSnapshot::default();
        assert!(!s.dtr);
        assert!(!s.rts);
    }
}
