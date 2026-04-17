//! Integration test: open a PTY pair via [`SerialPortDevice::pair`],
//! write bytes on one end, verify the other end reads them back.
//!
//! Linux-only. macOS PTY behaviour diverges enough from Linux that
//! `set_baud_rate` may reject and bidirectional reads can stall — both
//! observed on `macos-latest` GitHub runners. The Linux PTY path is
//! the canonical assumption for this suite; macOS coverage waits on
//! a real-device test plan, Windows on the v0.8 native backend.

#![cfg(target_os = "linux")]

use rtcom_core::{SerialConfig, SerialDevice, SerialPortDevice};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn write_one_end_read_other_end() {
    let (mut a, mut b) = SerialPortDevice::pair().expect("allocate pty pair");
    let payload = b"rtcom-roundtrip";

    a.write_all(payload).await.expect("write on end A");
    a.flush().await.expect("flush end A");

    let mut buf = vec![0u8; payload.len()];
    b.read_exact(&mut buf).await.expect("read on end B");
    assert_eq!(&buf, payload);
}

#[tokio::test]
async fn bidirectional_round_trip() {
    let (mut a, mut b) = SerialPortDevice::pair().expect("allocate pty pair");

    a.write_all(b"ping").await.unwrap();
    a.flush().await.unwrap();
    let mut buf = [0u8; 4];
    b.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");

    b.write_all(b"pong").await.unwrap();
    b.flush().await.unwrap();
    let mut buf = [0u8; 4];
    a.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"pong");
}

#[tokio::test]
async fn baud_change_updates_cached_config() {
    let (mut a, _b) = SerialPortDevice::pair().expect("allocate pty pair");
    assert_eq!(a.config().baud_rate, SerialConfig::default().baud_rate);
    a.set_baud_rate(9600).expect("set baud");
    assert_eq!(a.config().baud_rate, 9600);
}
