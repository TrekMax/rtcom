//! Integration tests for [`Session`]: drives a real PTY pair via the
//! default backend so we exercise the full read → bus → write → device
//! pipeline (no mocks).
//!
//! Unix-only because [`SerialPortDevice::pair`] is not available on Windows.

#![cfg(unix)]

use std::time::Duration;

use bytes::Bytes;
use rtcom_core::{Event, SerialPortDevice, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

/// Hard upper bound for any single operation. Generous because PTY
/// scheduling on busy CI runners can be slow.
const STEP_TIMEOUT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn session_publishes_rx_bytes_for_external_writes() {
    let (mut external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();
    let mut rx = bus.subscribe();

    let session_handle = tokio::spawn(session.run());

    // First event: connected.
    let event = timeout(STEP_TIMEOUT, rx.recv())
        .await
        .expect("timed out waiting for DeviceConnected")
        .expect("bus closed before DeviceConnected");
    assert!(matches!(event, Event::DeviceConnected));

    external.write_all(b"hello").await.unwrap();
    external.flush().await.unwrap();

    // We may receive bytes in one or more chunks depending on PTY chunking.
    let mut received = Vec::new();
    while received.len() < 5 {
        let event = timeout(STEP_TIMEOUT, rx.recv())
            .await
            .expect("timed out waiting for RxBytes")
            .expect("bus closed before RxBytes arrived");
        if let Event::RxBytes(bytes) = event {
            received.extend_from_slice(&bytes);
        }
    }
    assert_eq!(&received, b"hello");

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
}

#[tokio::test]
async fn session_writes_tx_bytes_to_device() {
    let (mut external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();

    let session_handle = tokio::spawn(session.run());

    // Give the writer task a moment to subscribe before we publish.
    tokio::task::yield_now().await;

    bus.publish(Event::TxBytes(Bytes::from_static(b"ping")));

    let mut wire = [0_u8; 4];
    timeout(STEP_TIMEOUT, external.read_exact(&mut wire))
        .await
        .expect("timed out reading from external end")
        .expect("read failed");
    assert_eq!(&wire, b"ping");

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
}

#[tokio::test]
async fn cancellation_unblocks_run_with_no_io_pending() {
    let (_external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let cancel = session.cancellation_token();

    let session_handle = tokio::spawn(session.run());

    // No traffic at all — both tasks are blocked on read/recv. Cancelling
    // must unblock them and let run() return promptly.
    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down on cancel")
        .expect("session task panicked")
        .expect("session returned error");
}
