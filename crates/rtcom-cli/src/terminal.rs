//! Terminal renderer task.
//!
//! Subscribes to the [`EventBus`] and writes user-visible output to a
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
//! Stub: only the public API is in place; behaviour lands in the green
//! commit.

use rtcom_core::EventBus;
use tokio::io::AsyncWrite;
use tokio_util::sync::CancellationToken;

/// Prefix the renderer prepends to every [`Event::SystemMessage`].
#[allow(dead_code, reason = "wired into main in the next commit")]
pub const SYSTEM_PREFIX: &str = "*** rtcom: ";

/// Drives the renderer until either the cancellation token trips or
/// the bus closes (no senders left).
#[allow(
    dead_code,
    clippy::unused_async,
    reason = "wired into main in the next commit; tests cover it now"
)]
pub async fn run_terminal_renderer<W>(_bus: EventBus, _cancel: CancellationToken, _writer: W)
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    todo!("run_terminal_renderer — implementation lands in the green commit")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use rtcom_core::Event;
    use tokio::io::{duplex, AsyncReadExt};
    use tokio::time::timeout;

    use super::*;

    const STEP: Duration = Duration::from_millis(500);

    /// Helper: spin up a renderer wired to a fresh duplex pipe.
    /// Returns the cancel handle and the read side of the pipe.
    fn launch() -> (
        EventBus,
        CancellationToken,
        tokio::task::JoinHandle<()>,
        tokio::io::DuplexStream,
    ) {
        let bus = EventBus::default();
        let cancel = CancellationToken::new();
        let (writer, reader) = duplex(1024);
        let task = tokio::spawn(run_terminal_renderer(bus.clone(), cancel.clone(), writer));
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
