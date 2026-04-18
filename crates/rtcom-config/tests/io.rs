//! Integration tests for the `read` / `write` profile IO surface.

use rtcom_config::{read, write, Error, Profile};
use tempfile::tempdir;

#[test]
fn write_then_read_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("default.toml");
    let mut p = Profile::default();
    p.serial.baud = 9600;

    write(&path, &p).expect("write");
    let loaded = read(&path).expect("read");
    assert_eq!(loaded.serial.baud, 9600);
}

#[test]
fn write_creates_parent_dirs() {
    let dir = tempdir().unwrap();
    let nested = dir.path().join("a").join("b").join("default.toml");
    let p = Profile::default();

    write(&nested, &p).expect("write should create parent dirs");
    assert!(nested.exists());
}

#[test]
fn read_missing_returns_io_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("absent.toml");
    let err = read(&path).unwrap_err();
    assert!(matches!(err, Error::Io(_)));
}

#[test]
fn read_malformed_returns_parse_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "this is not valid = = toml").unwrap();
    let err = read(&path).unwrap_err();
    assert!(matches!(err, Error::Parse(_)));
}
