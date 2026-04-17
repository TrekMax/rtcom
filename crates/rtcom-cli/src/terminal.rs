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
#[allow(dead_code, reason = "wired into main in the next commit")]
pub async fn run_terminal_renderer<W>(
    mut rx: broadcast::Receiver<Event>,
    cancel: CancellationToken,
    mut writer: W,
) where
    W: AsyncWrite + Unpin + Send + 'static,
{
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            msg = rx.recv() => match msg {
                Ok(Event::RxBytes(bytes)) => {
                    if writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                    // Flush so the bytes reach the user immediately;
                    // serial terminals are typically interactive and
                    // line buffering would mask incoming chunks.
                    let _ = writer.flush().await;
                }
                Ok(Event::SystemMessage(text)) => {
                    if writer.write_all(SYSTEM_PREFIX.as_bytes()).await.is_err() {
                        break;
                    }
                    if writer.write_all(text.as_bytes()).await.is_err() {
                        break;
                    }
                    if writer.write_all(b"\n").await.is_err() {
                        break;
                    }
                    let _ = writer.flush().await;
                }
                // Diagnostics flow through tracing; the wire stream is
                // for serial bytes + system messages only.
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
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
}
