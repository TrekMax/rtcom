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

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::device::SerialDevice;
use crate::event::{Event, EventBus};

/// Read buffer size. 4 KiB matches the typical USB-serial driver burst
/// granularity; larger buffers waste memory, smaller ones fragment events.
const READ_BUFFER_BYTES: usize = 4096;

/// Owns a serial device and a bus, and runs the I/O + command loop.
///
/// `Session` is generic over the device type so tests can substitute a
/// PTY pair (`SerialPortDevice::pair`) or, in the future, a fully mocked
/// backend without dynamic dispatch overhead.
pub struct Session<D: SerialDevice + 'static> {
    device: D,
    bus: EventBus,
    cancel: CancellationToken,
}

impl<D: SerialDevice + 'static> Session<D> {
    /// Builds a session with a fresh bus and cancellation token.
    #[must_use]
    pub fn new(device: D) -> Self {
        Self {
            device,
            bus: EventBus::default(),
            cancel: CancellationToken::new(),
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
        }
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
                        let bytes = Bytes::copy_from_slice(&read_buf[..n]);
                        self.bus.publish(Event::RxBytes(bytes));
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
                        if let Err(err) = self.device.write_all(&bytes).await {
                            self.bus.publish(Event::DeviceDisconnected {
                                reason: format!("serial write failed: {err}"),
                            });
                            break;
                        }
                    }
                    // Command dispatch lands in the next commits (issue #7).
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
}
