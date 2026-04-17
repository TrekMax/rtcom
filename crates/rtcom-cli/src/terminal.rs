//! Terminal renderer task.
//!
//! Subscribes to the `EventBus` and writes user-visible output to a
//! generic [`AsyncWrite`] sink (stdout in production, an in-memory pipe
//! in tests):
//!
//! - [`Event::RxBytes`] → bytes are written verbatim. Serial data must
//!   pass through untouched so log capture matches what the device
//!   actually emitted.
//! - [`Event::SystemMessage`] → prefixed with `"*** rtcom: "` and
//!   terminated with `\n`. The prefix is what tells the user (and any
//!   future log filter from Issue #10) that this line came from rtcom
//!   itself, not from the wire.
//! - Other events are ignored — diagnostics flow through `tracing`,
//!   not the terminal stream.
//!
use rtcom_core::Event;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// Prefix the renderer prepends to every [`Event::SystemMessage`].
#[allow(dead_code, reason = "wired into main in the next commit")]
pub const SYSTEM_PREFIX: &str = "*** rtcom: ";

/// Drives the renderer until either the cancellation token trips or
/// the bus closes (no senders left).
///
/// The caller is responsible for `bus.subscribe()`-ing before any
/// events that should reach this renderer are published. Subscribing
/// here would race with the spawn site — broadcast channels do not
/// replay messages sent before a subscriber attaches.
pub async fn run_terminal_renderer<W>(
    mut rx: broadcast::Receiver<Event>,
    cancel: CancellationToken,
    mut writer: W,
) where
    W: AsyncWrite + Unpin + Send + 'static,
{
    loop {
        tokio::select! {
            // No `biased` — if cancel and an event race (which is
            // exactly what Session::run does on a disconnect: it
            // publishes DeviceDisconnected, then the loop breaks and
            // main trips cancel), either arm may win. The drain-after
            // loop below covers the cancel-wins case.
            () = cancel.cancelled() => break,
            msg = rx.recv() => match msg {
                Ok(event) => {
                    if handle_event(&mut writer, event).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    // Drain anything the bus had buffered before cancel tripped. The
    // DeviceDisconnected message in particular must reach the user —
    // that is the primary reason this post-loop drain exists.
    while let Ok(event) = rx.try_recv() {
        if handle_event(&mut writer, event).await.is_err() {
            return;
        }
    }
}

/// Writes a single event to the sink. Returns `Err(())` when the sink
/// is gone; callers treat that as a signal to stop the renderer.
async fn handle_event<W>(writer: &mut W, event: Event) -> Result<(), ()>
where
    W: AsyncWrite + Unpin,
{
    match event {
        Event::RxBytes(bytes) => {
            write_or_fail(writer, &bytes).await?;
        }
        Event::SystemMessage(text) => {
            write_or_fail(writer, SYSTEM_PREFIX.as_bytes()).await?;
            write_or_fail(writer, text.as_bytes()).await?;
            write_or_fail(writer, b"\n").await?;
        }
        Event::DeviceDisconnected { reason } => {
            let line = format!("{SYSTEM_PREFIX}device disconnected: {reason}\n");
            write_or_fail(writer, line.as_bytes()).await?;
        }
        // Diagnostics flow through tracing; the wire stream is for
        // serial bytes + human-readable status only.
        _ => return Ok(()),
    }
    // Flush so interactive bytes and status lines reach the user
    // immediately; line buffering on stdout would otherwise hide
    // incoming chunks and the disconnect notice.
    let _ = writer.flush().await;
    Ok(())
}

async fn write_or_fail<W>(writer: &mut W, buf: &[u8]) -> Result<(), ()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(buf).await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use rtcom_core::{Event, EventBus};
    use tokio::io::{duplex, AsyncReadExt};
    use tokio::time::timeout;

    use super::*;

    const STEP: Duration = Duration::from_millis(500);

    /// Helper: spin up a renderer wired to a fresh duplex pipe.
    /// Subscribes synchronously *before* spawning the task so any
    /// `publish()` the test makes immediately afterwards is observed.
    fn launch() -> (
        EventBus,
        CancellationToken,
        tokio::task::JoinHandle<()>,
        tokio::io::DuplexStream,
    ) {
        let bus = EventBus::default();
        let cancel = CancellationToken::new();
        let rx = bus.subscribe();
        let (writer, reader) = duplex(1024);
        let task = tokio::spawn(run_terminal_renderer(rx, cancel.clone(), writer));
        (bus, cancel, task, reader)
    }

    async fn read_n(reader: &mut tokio::io::DuplexStream, n: usize) -> Vec<u8> {
        let mut buf = vec![0_u8; n];
        timeout(STEP, reader.read_exact(&mut buf))
            .await
            .expect("timed out waiting for bytes")
            .expect("read failed");
        buf
    }

    #[tokio::test]
    async fn rx_bytes_are_written_verbatim() {
        let (bus, cancel, task, mut reader) = launch();
        bus.publish(Event::RxBytes(Bytes::from_static(b"hello")));
        assert_eq!(read_n(&mut reader, 5).await, b"hello");
        cancel.cancel();
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn system_message_gets_prefix_and_newline() {
        let (bus, cancel, task, mut reader) = launch();
        bus.publish(Event::SystemMessage("hi".into()));
        let expected = format!("{SYSTEM_PREFIX}hi\n");
        assert_eq!(
            read_n(&mut reader, expected.len()).await,
            expected.as_bytes()
        );
        cancel.cancel();
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn unrelated_events_emit_no_output() {
        let (bus, cancel, task, mut reader) = launch();
        bus.publish(Event::DeviceConnected);
        // Confirm by sending a follow-up RxBytes and verifying it
        // is the *first* thing the reader sees — no leakage from the
        // ignored variant.
        bus.publish(Event::RxBytes(Bytes::from_static(b"x")));
        assert_eq!(read_n(&mut reader, 1).await, b"x");
        cancel.cancel();
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancellation_stops_the_renderer() {
        let (_bus, cancel, task, _reader) = launch();
        // Give the task a moment to subscribe and park in select.
        tokio::task::yield_now().await;
        cancel.cancel();
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn device_disconnected_prints_reason_as_system_message() {
        let (bus, cancel, task, mut reader) = launch();
        bus.publish(Event::DeviceDisconnected {
            reason: "EOF on serial read".into(),
        });
        let expected = format!("{SYSTEM_PREFIX}device disconnected: EOF on serial read\n");
        assert_eq!(
            read_n(&mut reader, expected.len()).await,
            expected.as_bytes()
        );
        cancel.cancel();
        timeout(STEP, task).await.unwrap().unwrap();
    }

    /// Regression: when the Session publishes DeviceDisconnected and
    /// cancellation is tripped at effectively the same time, the
    /// renderer must still surface the message before shutting down.
    #[tokio::test]
    async fn disconnect_published_then_cancelled_still_reaches_user() {
        let (bus, cancel, task, mut reader) = launch();
        // Publish the disconnect event, then immediately cancel. The
        // ordering mirrors what Session::run does on a fatal I/O
        // error.
        bus.publish(Event::DeviceDisconnected {
            reason: "pipe closed".into(),
        });
        cancel.cancel();
        let expected = format!("{SYSTEM_PREFIX}device disconnected: pipe closed\n");
        assert_eq!(
            read_n(&mut reader, expected.len()).await,
            expected.as_bytes()
        );
        timeout(STEP, task).await.unwrap().unwrap();
    }
}
