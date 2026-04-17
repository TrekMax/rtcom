//! Asynchronous serial device abstraction and default `serialport` backend.
//!
//! The [`SerialDevice`] trait is the narrow runtime contract every backend —
//! real hardware, in-memory mock, pseudo-terminal loopback — must satisfy.
//! The rest of `rtcom-core` (event bus, session orchestrator, mappers) never
//! references a concrete serial implementation, which keeps testing and
//! future backends (e.g. a TCP passthrough, a simulated device) cheap.
//!
//! [`SerialPortDevice`] is the stock implementation, layered on top of
//! [`tokio_serial`] so reads and writes are driven by the tokio reactor
//! rather than a per-port blocking thread.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialStream};

use crate::config::{DataBits, FlowControl, ModemStatus, Parity, SerialConfig, StopBits};
use crate::error::Result;

/// Runtime contract for every serial backend used by `rtcom-core`.
///
/// Implementors supply full-duplex async I/O (via [`AsyncRead`] +
/// [`AsyncWrite`]) plus the control-plane operations needed for interactive
/// sessions: baud / framing changes, DTR/RTS toggling, line-break injection,
/// and a modem-status snapshot.
///
/// # Examples
///
/// ```no_run
/// use rtcom_core::{SerialConfig, SerialDevice, SerialPortDevice};
/// use tokio::io::AsyncWriteExt;
///
/// # async fn example() -> rtcom_core::Result<()> {
/// let mut port = SerialPortDevice::open("/dev/ttyUSB0", SerialConfig::default())?;
/// port.write_all(b"AT\r\n").await?;
/// # Ok(()) }
/// ```
pub trait SerialDevice: AsyncRead + AsyncWrite + Send + Unpin {
    /// Changes the baud rate at runtime.
    ///
    /// Successful calls also update the cached [`SerialConfig`] returned by
    /// [`config`](Self::config).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying driver rejects the rate (e.g. the
    /// hardware cannot produce the requested divisor).
    fn set_baud_rate(&mut self, baud: u32) -> Result<()>;

    /// Changes the data-bit width.
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the new setting cannot be applied.
    fn set_data_bits(&mut self, bits: DataBits) -> Result<()>;

    /// Changes the stop-bit count.
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the new setting cannot be applied.
    fn set_stop_bits(&mut self, bits: StopBits) -> Result<()>;

    /// Changes the parity mode.
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the new setting cannot be applied. Some
    /// platforms reject [`Parity::Mark`] / [`Parity::Space`].
    fn set_parity(&mut self, parity: Parity) -> Result<()>;

    /// Changes the flow-control mode.
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the new setting cannot be applied.
    fn set_flow_control(&mut self, flow: FlowControl) -> Result<()>;

    /// Drives the DTR output line to `level` (`true` = asserted).
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the line cannot be toggled.
    fn set_dtr(&mut self, level: bool) -> Result<()>;

    /// Drives the RTS output line to `level` (`true` = asserted).
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the line cannot be toggled.
    fn set_rts(&mut self, level: bool) -> Result<()>;

    /// Asserts a line break for `duration`.
    ///
    /// The call blocks the current thread for the duration of the break. In
    /// async contexts, schedule it via [`tokio::task::spawn_blocking`] if the
    /// duration is long enough to matter.
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the break cannot be asserted or cleared.
    fn send_break(&mut self, duration: Duration) -> Result<()>;

    /// Reads the current input-side modem-status lines.
    ///
    /// Takes `&mut self` because the underlying [`serialport`] API does: the
    /// OS read may update internal driver state.
    ///
    /// # Errors
    ///
    /// Propagates backend errors if the modem status register cannot be read.
    fn modem_status(&mut self) -> Result<ModemStatus>;

    /// Returns the most recently applied [`SerialConfig`].
    ///
    /// This is always in sync with successful calls to the `set_*` methods;
    /// it may diverge from the hardware if an external process reconfigures
    /// the port behind our back.
    fn config(&self) -> &SerialConfig;
}

/// Default [`SerialDevice`] backed by [`tokio_serial::SerialStream`].
///
/// Use [`SerialPortDevice::open`] to create one from a device path. On Unix,
/// [`SerialPortDevice::pair`] creates a connected pseudo-terminal pair that is
/// convenient for integration tests without real hardware.
pub struct SerialPortDevice {
    stream: SerialStream,
    config: SerialConfig,
}

impl SerialPortDevice {
    /// Opens the device at `path` with the supplied `config`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidConfig`](crate::Error::InvalidConfig) if
    /// `config` fails [`SerialConfig::validate`](SerialConfig::validate), and
    /// [`Error::Backend`](crate::Error::Backend) if the port cannot be opened
    /// or configured.
    pub fn open(path: &str, config: SerialConfig) -> Result<Self> {
        config.validate()?;
        let stream = tokio_serial::new(path, config.baud_rate)
            .data_bits(to_sp_data_bits(config.data_bits))
            .stop_bits(to_sp_stop_bits(config.stop_bits))
            .parity(to_sp_parity(config.parity))
            .flow_control(to_sp_flow(config.flow_control))
            .timeout(config.read_timeout)
            .open_native_async()?;
        Ok(Self { stream, config })
    }

    /// Creates a connected pseudo-terminal pair for testing. **Unix only.**
    ///
    /// Both ends are returned with [`SerialConfig::default`] cached; the
    /// baud-rate setting does not meaningfully affect a PTY but round-trip
    /// writes/reads work.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`](crate::Error::Backend) if the kernel cannot
    /// allocate a PTY pair.
    #[cfg(unix)]
    pub fn pair() -> Result<(Self, Self)> {
        let (a, b) = SerialStream::pair()?;
        let config = SerialConfig::default();
        Ok((Self { stream: a, config }, Self { stream: b, config }))
    }
}

impl AsyncRead for SerialPortDevice {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for SerialPortDevice {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl SerialDevice for SerialPortDevice {
    fn set_baud_rate(&mut self, baud: u32) -> Result<()> {
        if baud == 0 {
            return Err(crate::Error::InvalidConfig(
                "baud_rate must be non-zero".into(),
            ));
        }
        self.stream.set_baud_rate(baud)?;
        self.config.baud_rate = baud;
        Ok(())
    }

    fn set_data_bits(&mut self, bits: DataBits) -> Result<()> {
        self.stream.set_data_bits(to_sp_data_bits(bits))?;
        self.config.data_bits = bits;
        Ok(())
    }

    fn set_stop_bits(&mut self, bits: StopBits) -> Result<()> {
        self.stream.set_stop_bits(to_sp_stop_bits(bits))?;
        self.config.stop_bits = bits;
        Ok(())
    }

    fn set_parity(&mut self, parity: Parity) -> Result<()> {
        self.stream.set_parity(to_sp_parity(parity))?;
        self.config.parity = parity;
        Ok(())
    }

    fn set_flow_control(&mut self, flow: FlowControl) -> Result<()> {
        self.stream.set_flow_control(to_sp_flow(flow))?;
        self.config.flow_control = flow;
        Ok(())
    }

    fn set_dtr(&mut self, level: bool) -> Result<()> {
        self.stream.write_data_terminal_ready(level)?;
        Ok(())
    }

    fn set_rts(&mut self, level: bool) -> Result<()> {
        self.stream.write_request_to_send(level)?;
        Ok(())
    }

    fn send_break(&mut self, duration: Duration) -> Result<()> {
        self.stream.set_break()?;
        thread::sleep(duration);
        self.stream.clear_break()?;
        Ok(())
    }

    fn modem_status(&mut self) -> Result<ModemStatus> {
        Ok(ModemStatus {
            cts: self.stream.read_clear_to_send()?,
            dsr: self.stream.read_data_set_ready()?,
            ri: self.stream.read_ring_indicator()?,
            cd: self.stream.read_carrier_detect()?,
        })
    }

    fn config(&self) -> &SerialConfig {
        &self.config
    }
}

const fn to_sp_data_bits(b: DataBits) -> serialport::DataBits {
    match b {
        DataBits::Five => serialport::DataBits::Five,
        DataBits::Six => serialport::DataBits::Six,
        DataBits::Seven => serialport::DataBits::Seven,
        DataBits::Eight => serialport::DataBits::Eight,
    }
}

const fn to_sp_stop_bits(b: StopBits) -> serialport::StopBits {
    match b {
        StopBits::One => serialport::StopBits::One,
        StopBits::Two => serialport::StopBits::Two,
    }
}

// Mark/Space parity are not represented in the serialport crate; map them to
// None and let a future backend-specific hook override if a driver supports
// them natively.
const fn to_sp_parity(p: Parity) -> serialport::Parity {
    match p {
        Parity::Even => serialport::Parity::Even,
        Parity::Odd => serialport::Parity::Odd,
        Parity::None | Parity::Mark | Parity::Space => serialport::Parity::None,
    }
}

const fn to_sp_flow(f: FlowControl) -> serialport::FlowControl {
    match f {
        FlowControl::None => serialport::FlowControl::None,
        FlowControl::Hardware => serialport::FlowControl::Hardware,
        FlowControl::Software => serialport::FlowControl::Software,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_bits_round_trip() {
        assert_eq!(
            to_sp_data_bits(DataBits::Eight),
            serialport::DataBits::Eight
        );
        assert_eq!(to_sp_data_bits(DataBits::Five), serialport::DataBits::Five);
    }

    #[test]
    fn stop_bits_round_trip() {
        assert_eq!(to_sp_stop_bits(StopBits::One), serialport::StopBits::One);
        assert_eq!(to_sp_stop_bits(StopBits::Two), serialport::StopBits::Two);
    }

    #[test]
    fn parity_round_trip() {
        assert_eq!(to_sp_parity(Parity::Even), serialport::Parity::Even);
        assert_eq!(to_sp_parity(Parity::Odd), serialport::Parity::Odd);
        assert_eq!(to_sp_parity(Parity::None), serialport::Parity::None);
    }

    #[test]
    fn flow_round_trip() {
        assert_eq!(to_sp_flow(FlowControl::None), serialport::FlowControl::None);
        assert_eq!(
            to_sp_flow(FlowControl::Hardware),
            serialport::FlowControl::Hardware
        );
        assert_eq!(
            to_sp_flow(FlowControl::Software),
            serialport::FlowControl::Software
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pair_returns_default_config() {
        let (a, b) = SerialPortDevice::pair().expect("pty pair");
        assert_eq!(a.config(), &SerialConfig::default());
        assert_eq!(b.config(), &SerialConfig::default());
    }
}
