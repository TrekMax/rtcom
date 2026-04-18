//! End-to-end tests for `rtcom` command-line behaviour that does not
//! require a live pseudo-terminal.
//!
//! These cases exercise the argv-only paths — `--help`, `--version`,
//! missing-device error, `--save` without a device — so they are cheap
//! to run on every platform and don't depend on socat.
//!
//! PTY-driven cases live in `e2e_pty.rs`.

use assert_cmd::Command;
use predicates::prelude::*;

/// `rtcom --help` should print the `about` string and exit cleanly.
///
/// We match against "Rust Terminal Communication" because the tagline
/// is set in `args::Cli::command` and is stable across releases.
#[test]
fn help_flag_succeeds() {
    Command::cargo_bin("rtcom")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Rust Terminal Communication"));
}

/// `rtcom --version` should print something version-ish and exit 0.
///
/// The exact version string is assembled by `build.rs` into
/// `RTCOM_VERSION`; asserting on the literal number would couple this
/// test to every release bump, so we only check that the `rtcom` name
/// shows up in the output.
#[test]
fn version_flag_succeeds() {
    Command::cargo_bin("rtcom")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("rtcom"));
}

/// `rtcom` with no positional arg must fail: `device` is required by
/// clap. We only check that clap's usage message mentions the missing
/// argument, not the exact wording, because clap's phrasing varies
/// between major versions.
#[test]
fn missing_device_errors() {
    Command::cargo_bin("rtcom")
        .unwrap()
        .assert()
        .failure()
        .stderr(predicate::str::contains("device").or(predicate::str::contains("required")));
}

/// `--save` without a device should still fail for the same reason as
/// the bare invocation — clap rejects it before main ever runs.
/// Guards against a regression where someone might make `device`
/// conditionally optional.
#[test]
fn save_flag_without_device_errors() {
    Command::cargo_bin("rtcom")
        .unwrap()
        .arg("--save")
        .assert()
        .failure();
}
