//! Stdin reader task.
//!
//! Reads bytes from any [`AsyncRead`], feeds them through a
//! [`CommandKeyParser`], and publishes the resulting events to the
//! [`EventBus`]. In production it is fed by [`tokio::io::stdin`] inside a
//! [`RawModeGuard`](crate::tty::RawModeGuard); in tests it accepts any
//! `AsyncRead`, typically a [`tokio::io::DuplexStream`] half.
//!
//! Stub: only the public function shape is in place. The body is filled
//! in by the green commit.

use rtcom_core::EventBus;
use tokio::io::AsyncRead;
use tokio_util::sync::CancellationToken;

/// Drives the stdin → parser → bus pipeline until either:
///
/// - the [`CancellationToken`] is tripped,
/// - the reader returns EOF (`Ok(0)`),
/// - or the reader returns an error (treated as EOF; the caller has
///   already lost the stream).
#[allow(
    dead_code,
    clippy::unused_async,
    reason = "stub for the red commit; green commit fills the body"
)]
pub async fn run_stdin_reader<R>(
    _reader: R,
    _bus: EventBus,
    _cancel: CancellationToken,
    _escape: u8,
) where
    R: AsyncRead + Unpin,
{
    todo!("run_stdin_reader — implementation lands in the green commit")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rtcom_core::{Command, Event};
    use tokio::io::{duplex, AsyncWriteExt};
    use tokio::time::timeout;

    use super::*;

    const ESC: u8 = 0x14; // ^T
    const STEP: Duration = Duration::from_millis(500);

    /// Builds a fresh duplex pair, writes the given bytes to one side,
    /// drops that side to signal EOF, and returns the read end ready to
    /// be handed to `run_stdin_reader`.
    async fn reader_with(bytes: &[u8]) -> tokio::io::DuplexStream {
        let (mut writer, reader) = duplex(64);
        writer.write_all(bytes).await.unwrap();
        drop(writer);
        reader
    }

    async fn next(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Event {
        timeout(STEP, rx.recv())
            .await
            .expect("timed out waiting for event")
            .expect("bus closed unexpectedly")
    }

    #[tokio::test]
    async fn plain_bytes_become_tx_events() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        let cancel = CancellationToken::new();
        let reader = reader_with(b"hi").await;

        let task = tokio::spawn(run_stdin_reader(reader, bus, cancel, ESC));

        match next(&mut rx).await {
            Event::TxBytes(b) => assert_eq!(&b[..], b"h"),
            other => panic!("unexpected: {other:?}"),
        }
        match next(&mut rx).await {
            Event::TxBytes(b) => assert_eq!(&b[..], b"i"),
            other => panic!("unexpected: {other:?}"),
        }
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn escape_sequence_emits_command_event() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        let cancel = CancellationToken::new();
        let reader = reader_with(&[ESC, b'?']).await;

        let task = tokio::spawn(run_stdin_reader(reader, bus, cancel, ESC));

        match next(&mut rx).await {
            Event::Command(Command::Help) => {}
            other => panic!("unexpected: {other:?}"),
        }
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn baud_change_sequence_emits_set_baud() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        let cancel = CancellationToken::new();
        let mut input = vec![ESC, b'b'];
        input.extend_from_slice(b"9600\r");
        let reader = reader_with(&input).await;

        let task = tokio::spawn(run_stdin_reader(reader, bus, cancel, ESC));

        match next(&mut rx).await {
            Event::Command(Command::SetBaud(9600)) => {}
            other => panic!("unexpected: {other:?}"),
        }
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn unknown_command_byte_does_not_publish_anything_but_drains_stream() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();
        let cancel = CancellationToken::new();
        // ^T followed by a byte that is not in the command table, then
        // a normal byte that should still pass through.
        let reader = reader_with(&[ESC, b'z', b'a']).await;

        let task = tokio::spawn(run_stdin_reader(reader, bus, cancel, ESC));

        match next(&mut rx).await {
            Event::TxBytes(b) => assert_eq!(&b[..], b"a"),
            other => panic!("unexpected: {other:?}"),
        }
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancellation_stops_pending_read_promptly() {
        let bus = EventBus::default();
        let cancel = CancellationToken::new();
        // Keep the writer half alive so the reader stays pending.
        let (_writer, reader) = duplex(64);

        let task = tokio::spawn(run_stdin_reader(reader, bus, cancel.clone(), ESC));
        // Let the task park itself in select.
        tokio::task::yield_now().await;
        cancel.cancel();
        timeout(STEP, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn eof_terminates_task() {
        let bus = EventBus::default();
        let cancel = CancellationToken::new();
        let reader = reader_with(b"").await;
        let task = tokio::spawn(run_stdin_reader(reader, bus, cancel, ESC));
        timeout(STEP, task).await.unwrap().unwrap();
    }
}
