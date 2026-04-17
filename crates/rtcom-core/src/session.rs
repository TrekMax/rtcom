//! Session orchestrator: bridges a [`SerialDevice`](crate::SerialDevice)
//! with the [`EventBus`].
//!
//! At v0.1 a [`Session`] runs a single task that multiplexes the serial
//! device, the cancellation token, and the bus subscription via
//! `tokio::select!`:
//!
//! - bytes arriving from the device → [`Event::RxBytes`];
//! - [`Event::TxBytes`] published on the bus → bytes written to the device;
//! - [`Event::Command`] published on the bus → handler dispatch (Issue #7);
//! - cancellation token tripped or fatal I/O error → publish
//!   [`Event::DeviceDisconnected`] (when applicable) and exit cleanly.
//!
//! The single-task model gives the dispatch handlers exclusive `&mut`
//! access to the device, which is required for the synchronous control
//! operations (`set_baud_rate`, `set_dtr`, `send_break`, ...). The
//! tradeoff is that a long write momentarily delays reads — acceptable
//! for an interactive serial terminal where writes are short and rare.
//!
//! Later issues plug in mappers (Issue #8), logging, scripting, and so
//! on as additional bus subscribers.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::command::Command;
use crate::config::{Parity, StopBits};
use crate::device::SerialDevice;
use crate::event::{Event, EventBus};
use crate::mapper::{LineEndingMapper, Mapper};

const fn parity_letter(p: Parity) -> char {
    match p {
        Parity::None => 'N',
        Parity::Even => 'E',
        Parity::Odd => 'O',
        Parity::Mark => 'M',
        Parity::Space => 'S',
    }
}

const fn stop_bits_number(s: StopBits) -> u8 {
    match s {
        StopBits::One => 1,
        StopBits::Two => 2,
    }
}

/// Read buffer size. 4 KiB matches the typical USB-serial driver burst
/// granularity; larger buffers waste memory, smaller ones fragment events.
const READ_BUFFER_BYTES: usize = 4096;

/// Duration of the line break asserted by the `SendBreak` command.
const SEND_BREAK_DURATION: Duration = Duration::from_millis(250);

/// Static cheatsheet text for `Command::Help`.
const HELP_TEXT: &str = "commands: ?/h help, q/x quit, c show config, t toggle DTR, \
                         g toggle RTS, b<rate><Enter> set baud, \\ send break";

/// Owns a serial device and a bus, and runs the I/O + command loop.
///
/// `Session` is generic over the device type so tests can substitute a
/// PTY pair (`SerialPortDevice::pair`) or, in the future, a fully mocked
/// backend without dynamic dispatch overhead.
pub struct Session<D: SerialDevice + 'static> {
    device: D,
    bus: EventBus,
    cancel: CancellationToken,
    /// Outbound mapper applied to `Event::TxBytes` payloads before they
    /// hit the device. Defaults to a no-op `LineEndingMapper::default()`.
    omap: Box<dyn Mapper>,
    /// Inbound mapper applied to bytes read from the device before they
    /// are republished as `Event::RxBytes`. Defaults to no-op.
    imap: Box<dyn Mapper>,
    /// Tracked DTR state. Initialised to `true` because `SerialDevice`
    /// gives no way to query the line, and most backends open with DTR
    /// asserted; the first toggle therefore deasserts.
    dtr_asserted: bool,
    /// Tracked RTS state. See `dtr_asserted` for the rationale.
    rts_asserted: bool,
}

impl<D: SerialDevice + 'static> Session<D> {
    /// Builds a session with a fresh bus and cancellation token,
    /// no-op mappers on both directions.
    #[must_use]
    pub fn new(device: D) -> Self {
        Self {
            device,
            bus: EventBus::default(),
            cancel: CancellationToken::new(),
            omap: Box::new(LineEndingMapper::default()),
            imap: Box::new(LineEndingMapper::default()),
            dtr_asserted: true,
            rts_asserted: true,
        }
    }

    /// Builds a session attached to a caller-supplied bus. Useful when
    /// several subsystems already share a bus and the session should join
    /// the existing fan-out instead of starting its own.
    #[must_use]
    pub fn with_bus(device: D, bus: EventBus) -> Self {
        Self {
            device,
            bus,
            cancel: CancellationToken::new(),
            omap: Box::new(LineEndingMapper::default()),
            imap: Box::new(LineEndingMapper::default()),
            dtr_asserted: true,
            rts_asserted: true,
        }
    }

    /// Replaces the outbound mapper applied to `Event::TxBytes`
    /// payloads before they reach the device.
    #[must_use]
    pub fn with_omap<M: Mapper + 'static>(mut self, mapper: M) -> Self {
        self.omap = Box::new(mapper);
        self
    }

    /// Replaces the inbound mapper applied to bytes read from the
    /// device before they are republished as `Event::RxBytes`.
    #[must_use]
    pub fn with_imap<M: Mapper + 'static>(mut self, mapper: M) -> Self {
        self.imap = Box::new(mapper);
        self
    }

    /// Returns a reference to the bus. Clone it before calling
    /// [`Session::run`] (which consumes `self`) if you need to publish or
    /// subscribe from outside the session.
    #[must_use]
    pub const fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Returns a clone of the cancellation token.
    ///
    /// Triggering [`CancellationToken::cancel`] on any clone causes
    /// [`Session::run`] to wind down and return.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Drives the session to completion.
    ///
    /// Subscribes to the bus, publishes [`Event::DeviceConnected`], then
    /// loops until the cancellation token trips or a fatal I/O error
    /// terminates the device.
    ///
    /// # Errors
    ///
    /// Currently always returns `Ok(())`; the variant is reserved for
    /// startup failures introduced by later issues (e.g. mapper
    /// initialisation).
    pub async fn run(mut self) -> crate::Result<()> {
        // Subscribe BEFORE publishing so the loop sees nothing it sent
        // itself, but external pre-existing subscribers still get
        // DeviceConnected.
        let mut subscriber = self.bus.subscribe();
        self.bus.publish(Event::DeviceConnected);

        let mut read_buf = vec![0_u8; READ_BUFFER_BYTES];
        loop {
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => break,

                res = self.device.read(&mut read_buf) => match res {
                    Ok(0) => {
                        self.bus.publish(Event::DeviceDisconnected {
                            reason: "EOF on serial read".into(),
                        });
                        break;
                    }
                    Ok(n) => {
                        let mapped = self.imap.map(&read_buf[..n]);
                        self.bus.publish(Event::RxBytes(mapped));
                    }
                    Err(err) => {
                        self.bus.publish(Event::DeviceDisconnected {
                            reason: format!("serial read failed: {err}"),
                        });
                        break;
                    }
                },

                msg = subscriber.recv() => match msg {
                    Ok(Event::TxBytes(bytes)) => {
                        let mapped = self.omap.map(&bytes);
                        if let Err(err) = self.device.write_all(&mapped).await {
                            self.bus.publish(Event::DeviceDisconnected {
                                reason: format!("serial write failed: {err}"),
                            });
                            break;
                        }
                    }
                    Ok(Event::Command(cmd)) => self.dispatch_command(cmd),
                    // Lagged: we missed some events but can resume.
                    // Other event variants are not the loop's concern.
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    // Closed: no senders left, nothing more will arrive.
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        Ok(())
    }

    /// Apply a [`Command`] to the device and bus.
    ///
    /// Commands that mutate the device run synchronously here; success
    /// emits [`Event::ConfigChanged`] (when applicable), failure emits
    /// [`Event::Error`]. The caller (the `Session::run` loop) does not
    /// need to await anything: every operation either completes
    /// immediately or is fire-and-forget.
    fn dispatch_command(&mut self, cmd: Command) {
        match cmd {
            Command::Quit => self.cancel.cancel(),
            Command::Help => {
                self.bus.publish(Event::SystemMessage(HELP_TEXT.into()));
            }
            Command::ShowConfig => {
                let cfg = self.device.config();
                self.bus.publish(Event::SystemMessage(format!(
                    "config: {} {}{}{} flow={:?}",
                    cfg.baud_rate,
                    cfg.data_bits.bits(),
                    parity_letter(cfg.parity),
                    stop_bits_number(cfg.stop_bits),
                    cfg.flow_control,
                )));
            }
            Command::SetBaud(rate) => match self.device.set_baud_rate(rate) {
                Ok(()) => {
                    self.bus
                        .publish(Event::ConfigChanged(*self.device.config()));
                }
                Err(err) => {
                    self.bus.publish(Event::Error(Arc::new(err)));
                }
            },
            Command::ToggleDtr => {
                let new_state = !self.dtr_asserted;
                match self.device.set_dtr(new_state) {
                    Ok(()) => {
                        self.dtr_asserted = new_state;
                        self.bus.publish(Event::SystemMessage(format!(
                            "DTR: {}",
                            if new_state { "asserted" } else { "deasserted" }
                        )));
                    }
                    Err(err) => {
                        self.bus.publish(Event::Error(Arc::new(err)));
                    }
                }
            }
            Command::ToggleRts => {
                let new_state = !self.rts_asserted;
                match self.device.set_rts(new_state) {
                    Ok(()) => {
                        self.rts_asserted = new_state;
                        self.bus.publish(Event::SystemMessage(format!(
                            "RTS: {}",
                            if new_state { "asserted" } else { "deasserted" }
                        )));
                    }
                    Err(err) => {
                        self.bus.publish(Event::Error(Arc::new(err)));
                    }
                }
            }
            Command::SendBreak => match self.device.send_break(SEND_BREAK_DURATION) {
                Ok(()) => {
                    self.bus.publish(Event::SystemMessage(format!(
                        "sent {} ms break",
                        SEND_BREAK_DURATION.as_millis()
                    )));
                }
                Err(err) => {
                    self.bus.publish(Event::Error(Arc::new(err)));
                }
            },
        }
    }
}
