//! Session orchestrator: spawns the per-task pipeline that bridges a
//! [`SerialDevice`](crate::SerialDevice) with the [`EventBus`].
//!
//! At v0.1 a [`Session`] runs two tasks:
//!
//! - **`serial_reader_task`** — reads bytes from the device and publishes
//!   [`Event::RxBytes`].
//! - **`serial_writer_task`** — subscribes to [`Event::TxBytes`] and writes
//!   them to the device.
//!
//! Both tasks observe a shared [`CancellationToken`] and exit cleanly when
//! it is tripped. A task that hits a fatal I/O error publishes
//! [`Event::DeviceDisconnected`] and trips the token itself so the peer
//! task also unwinds — no task is left orphaned.
//!
//! Later issues plug in the command parser (Issue #6/#7), mappers
//! (Issue #8), logging, scripting, and so on, all as additional bus
//! subscribers — the Session itself stays small.

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::device::SerialDevice;
use crate::event::{Event, EventBus};

/// Read buffer size. 4 KiB matches the typical USB-serial driver burst
/// granularity; larger buffers waste memory, smaller ones fragment events.
const READ_BUFFER_BYTES: usize = 4096;

/// Owns a serial device and a bus, and supervises the I/O tasks.
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
    /// [`Session::run`] to wind down both tasks and return.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Drives the session to completion.
    ///
    /// Spawns the reader and writer tasks, publishes
    /// [`Event::DeviceConnected`], and returns when both tasks have
    /// finished — either because the cancellation token was tripped or
    /// because a fatal I/O error knocked them down.
    ///
    /// # Errors
    ///
    /// Currently always returns `Ok(())`; the variant is reserved for
    /// startup failures introduced by later issues (e.g. mapper
    /// initialisation).
    pub async fn run(self) -> crate::Result<()> {
        let bus = self.bus.clone();
        let cancel = self.cancel.clone();
        let (reader_half, writer_half) = tokio::io::split(self.device);

        // Subscribe BEFORE publishing so the writer task does not miss
        // anything sent during start-up.
        let writer_rx = bus.subscribe();

        let _ = bus.publish(Event::DeviceConnected);

        let reader_handle = spawn_reader_task(reader_half, bus.clone(), cancel.clone());
        let writer_handle = spawn_writer_task(writer_half, writer_rx, bus, cancel);

        // Joins the two tasks. JoinError only happens on panic / abort,
        // both of which we surface via tracing rather than propagating —
        // there is nothing meaningful for the caller to do.
        let _ = reader_handle.await;
        let _ = writer_handle.await;
        Ok(())
    }
}

fn spawn_reader_task<R>(
    mut reader: R,
    bus: EventBus,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut read_buf = vec![0_u8; READ_BUFFER_BYTES];
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                res = reader.read(&mut read_buf) => match res {
                    Ok(0) => {
                        let _ = bus.publish(Event::DeviceDisconnected {
                            reason: "EOF on serial read".into(),
                        });
                        cancel.cancel();
                        break;
                    }
                    Ok(n) => {
                        let bytes = Bytes::copy_from_slice(&read_buf[..n]);
                        let _ = bus.publish(Event::RxBytes(bytes));
                    }
                    Err(err) => {
                        let _ = bus.publish(Event::DeviceDisconnected {
                            reason: format!("serial read failed: {err}"),
                        });
                        cancel.cancel();
                        break;
                    }
                }
            }
        }
    })
}

fn spawn_writer_task<W>(
    mut writer: W,
    mut rx: broadcast::Receiver<Event>,
    bus: EventBus,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(Event::TxBytes(bytes)) => {
                        if let Err(err) = writer.write_all(&bytes).await {
                            let _ = bus.publish(Event::DeviceDisconnected {
                                reason: format!("serial write failed: {err}"),
                            });
                            cancel.cancel();
                            break;
                        }
                    }
                    // Other event variants are not the writer's concern.
                    Ok(_) => {}
                    // Lagged: we missed some events but can resume.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    // Closed: bus has no more senders, nothing left to do.
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    })
}
