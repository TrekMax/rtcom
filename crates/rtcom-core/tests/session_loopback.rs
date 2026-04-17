//! Integration tests for [`Session`]: drives a real PTY pair via the
//! default backend so we exercise the full read → bus → write → device
//! pipeline (no mocks).
//!
//! Unix-only because [`SerialPortDevice::pair`] is not available on Windows.

// Linux-only for the same reason as pty_roundtrip.rs: macOS PTYs
// behave subtly differently and the Session loop's read/write timing
// assumptions don't hold there. Linux gives canonical PTY semantics
// that match real serial devices closely enough for the tests to
// stay deterministic.
#![cfg(target_os = "linux")]

use std::time::Duration;

use bytes::Bytes;
use rtcom_core::{Command, Event, LineEnding, LineEndingMapper, SerialPortDevice, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast::Receiver;
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

/// Drain bus events until the predicate matches one. Bounded by
/// `STEP_TIMEOUT` so a missing event fails the test instead of hanging.
async fn wait_for(rx: &mut Receiver<Event>, mut pred: impl FnMut(&Event) -> bool) -> Event {
    timeout(STEP_TIMEOUT, async move {
        loop {
            match rx.recv().await {
                Ok(event) if pred(&event) => return event,
                Ok(_) => {}
                Err(err) => panic!("bus error before match: {err:?}"),
            }
        }
    })
    .await
    .expect("predicate never matched within STEP_TIMEOUT")
}

#[tokio::test]
async fn omap_add_cr_to_lf_converts_lf_to_crlf_on_wire() {
    let (mut external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal).with_omap(LineEndingMapper::new(LineEnding::AddCrToLf));
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();

    let session_handle = tokio::spawn(session.run());
    tokio::task::yield_now().await;

    bus.publish(Event::TxBytes(Bytes::from_static(b"hi\n")));

    let mut wire = [0_u8; 4];
    timeout(STEP_TIMEOUT, external.read_exact(&mut wire))
        .await
        .expect("timed out reading mapped bytes")
        .expect("read failed");
    assert_eq!(&wire, b"hi\r\n");

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
}

#[tokio::test]
async fn imap_add_cr_to_lf_converts_received_lf_to_crlf_in_event() {
    let (mut external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal).with_imap(LineEndingMapper::new(LineEnding::AddCrToLf));
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();
    let mut rx = bus.subscribe();

    let session_handle = tokio::spawn(session.run());
    tokio::task::yield_now().await;

    external.write_all(b"hi\n").await.unwrap();
    external.flush().await.unwrap();

    let mut received = Vec::new();
    while received.len() < 4 {
        let event = wait_for(&mut rx, |e| matches!(e, Event::RxBytes(_))).await;
        if let Event::RxBytes(bytes) = event {
            received.extend_from_slice(&bytes);
        }
    }
    assert_eq!(&received[..4], b"hi\r\n");

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
}

#[tokio::test]
async fn quit_command_returns_run() {
    let (_external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let bus = session.bus().clone();

    let session_handle = tokio::spawn(session.run());
    tokio::task::yield_now().await;

    bus.publish(Event::Command(Command::Quit));

    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down on Quit command")
        .expect("session task panicked")
        .expect("session returned error");
}

/// Spawns a session, yields once so the loop subscribes, publishes
/// `cmd`, waits for a `SystemMessage` to arrive on `rx`, then shuts the
/// session down cleanly. Returns the message text.
async fn capture_system_message(cmd: Command) -> String {
    let (_external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();
    let mut rx = bus.subscribe();

    let session_handle = tokio::spawn(session.run());
    tokio::task::yield_now().await;

    bus.publish(Event::Command(cmd));

    let event = wait_for(&mut rx, |e| matches!(e, Event::SystemMessage(_))).await;
    let text = match event {
        Event::SystemMessage(t) => t,
        other => panic!("unexpected: {other:?}"),
    };

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
    text
}

#[tokio::test]
async fn show_config_command_emits_system_message_with_current_settings() {
    let text = capture_system_message(Command::ShowConfig).await;
    // Default config is 115200 8N1 — at minimum the baud should appear.
    assert!(
        text.contains("115200"),
        "expected baud in SystemMessage: {text:?}"
    );
}

#[tokio::test]
async fn help_command_emits_system_message_listing_keys() {
    let text = capture_system_message(Command::Help).await;
    assert!(
        text.to_lowercase().contains("quit"),
        "expected Help text to mention 'quit': {text:?}"
    );
}

/// Same idea as `capture_system_message`, but accepts an `Event::Error`
/// outcome too — useful for tests of device-control commands that PTYs
/// may reject (DTR/RTS/break ioctls). The error-path acceptance proves
/// the dispatcher tried, even if the kernel turned it down.
async fn dispatch_and_wait_for_message_or_error(cmd: Command) -> Option<String> {
    let (_external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();
    let mut rx = bus.subscribe();

    let session_handle = tokio::spawn(session.run());
    tokio::task::yield_now().await;

    bus.publish(Event::Command(cmd));

    let event = wait_for(&mut rx, |e| {
        matches!(e, Event::SystemMessage(_) | Event::Error(_))
    })
    .await;
    let text = match event {
        Event::SystemMessage(t) => Some(t),
        Event::Error(_) => None,
        other => panic!("unexpected: {other:?}"),
    };

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
    text
}

#[tokio::test]
async fn toggle_dtr_command_emits_system_message_or_error() {
    if let Some(text) = dispatch_and_wait_for_message_or_error(Command::ToggleDtr).await {
        assert!(
            text.contains("DTR"),
            "expected DTR mention in SystemMessage: {text:?}"
        );
    }
    // Err path means the PTY rejected the ioctl, which still proves the
    // dispatcher hit set_dtr — that is what we care about here.
}

#[tokio::test]
async fn toggle_rts_command_emits_system_message_or_error() {
    if let Some(text) = dispatch_and_wait_for_message_or_error(Command::ToggleRts).await {
        assert!(
            text.contains("RTS"),
            "expected RTS mention in SystemMessage: {text:?}"
        );
    }
}

#[tokio::test]
async fn send_break_command_emits_system_message() {
    // PTYs may reject set_break entirely (return Err); accept either
    // SystemMessage("...break...") or Event::Error as success — both
    // mean the dispatcher tried.
    let (_external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();
    let mut rx = bus.subscribe();

    let session_handle = tokio::spawn(session.run());
    tokio::task::yield_now().await;

    bus.publish(Event::Command(Command::SendBreak));

    let event = wait_for(&mut rx, |e| {
        matches!(e, Event::SystemMessage(_) | Event::Error(_))
    })
    .await;
    match event {
        Event::SystemMessage(text) => assert!(
            text.to_lowercase().contains("break"),
            "expected break mention: {text:?}"
        ),
        Event::Error(_) => {} // PTY rejected set_break, acceptable
        other => panic!("unexpected: {other:?}"),
    }

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
}

#[tokio::test]
async fn set_baud_command_updates_device_and_emits_config_changed() {
    let (_external, internal) = SerialPortDevice::pair().expect("pty pair");
    let session = Session::new(internal);
    let bus = session.bus().clone();
    let cancel = session.cancellation_token();
    let mut rx = bus.subscribe();

    let session_handle = tokio::spawn(session.run());
    tokio::task::yield_now().await;

    bus.publish(Event::Command(Command::SetBaud(9600)));

    let event = wait_for(&mut rx, |e| matches!(e, Event::ConfigChanged(_))).await;
    match event {
        Event::ConfigChanged(cfg) => assert_eq!(cfg.baud_rate, 9600),
        other => panic!("unexpected: {other:?}"),
    }

    cancel.cancel();
    timeout(STEP_TIMEOUT, session_handle)
        .await
        .expect("session did not shut down")
        .expect("session task panicked")
        .expect("session returned error");
}
