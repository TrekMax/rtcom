//! End-to-end tests that need a live pseudo-terminal.
//!
//! Gated to `target_os = "linux"` because socat — the PTY provider of
//! choice — isn't guaranteed everywhere else, and the TTY semantics
//! our helpers assume (paths under `/dev/pts/`, `try_wait`
//! responsiveness) match the Linux build target for the v0.2 matrix.
//!
//! Tests skip gracefully (`eprintln!` + early return) if socat is
//! absent from `PATH` or refuses to spawn. A skipped test still
//! counts as `ok` in the runner; the operator message is the only
//! signal that something was stepped over.
//!
//! # Testing philosophy
//!
//! These cases deliberately target startup / shutdown / profile-IO
//! paths, not the TUI itself. Keystroke-driven UI verification needs
//! a proper expect / vt100-replay harness and is scoped to a later
//! ticket. Today we prove:
//!
//! * `--save` writes the expected profile file before the session
//!   enters its render loop.
//! * A pre-seeded profile at a custom `-c` path is read without a
//!   parse-error warning, confirming rtcom is honouring `-c`.
//! * An intentionally-missing `-c PATH` is non-fatal — rtcom falls
//!   back to defaults without emitting a profile-related error.
//!
//! # Why we don't assert on exit status
//!
//! The test harness pipes rtcom's stdin, which isn't a TTY. rtcom's
//! `rtcom-tui::run()` therefore fails immediately at `enable raw
//! mode` and the process exits with status 1. That failure is
//! environmental, not a regression — so the profile tests observe
//! stderr patterns instead. A future ticket that pairs rtcom with a
//! proper PTY-backed stdin (via `nix::pty::openpty` or a `script`
//! wrapper) can upgrade these to full-exit-code assertions.

#![cfg(target_os = "linux")]

mod common;

use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use common::{socat_available, wait_with_timeout, PtyPair};

/// Spawning rtcom with `--save -b 9600 -c $TMP/default.toml` must
/// persist a profile file reflecting the `-b 9600` override before
/// (or immediately after) the session goes live.
///
/// `--save` runs synchronously in `main` before the TUI is entered,
/// so the file is written regardless of whether raw-mode setup later
/// fails. We poll for up to 3 s so a slow CI runner doesn't flake,
/// parse the TOML through `rtcom_config::read` (file presence alone
/// isn't enough — malformed output is a regression), and only then
/// tear the process down.
#[test]
fn save_cli_flag_persists_profile() {
    if !socat_available() {
        eprintln!("skipping save_cli_flag_persists_profile: socat not available");
        return;
    }
    let pty = match PtyPair::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skipping save_cli_flag_persists_profile: {e}");
            return;
        }
    };

    let tempdir = tempfile::tempdir().expect("create tempdir");
    let profile_path = tempdir.path().join("default.toml");

    let binary = assert_cmd::cargo::cargo_bin("rtcom");
    let mut child = Command::new(&binary)
        .arg(&pty.master)
        .args(["-b", "9600", "--save", "-c"])
        .arg(&profile_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtcom");

    // Poll up to 3 s for the profile file to appear and parse with
    // baud=9600. Each iteration sleeps 100 ms, so 30 tries covers
    // the full window.
    let mut loaded: Option<rtcom_config::Profile> = None;
    for _ in 0..30 {
        if profile_path.exists() {
            if let Ok(p) = rtcom_config::read(&profile_path) {
                if p.serial.baud == 9600 {
                    loaded = Some(p);
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Let the process wind down on its own (stdin is /dev/null, so
    // raw-mode setup fails fast and rtcom exits) or kill it as a
    // safety net.
    let status = wait_with_timeout(&mut child, Duration::from_secs(2));
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }

    assert!(
        loaded.is_some(),
        "profile was not written with baud=9600 at {}",
        profile_path.display()
    );
}

/// A profile pre-seeded at a custom `-c` path must parse cleanly.
///
/// Observation: a successful load emits no profile-related warning
/// on stderr. A failed load would print
/// `rtcom: profile at <path> unreadable (...)`. We collect stderr
/// for up to 2 s (with a hard kill if needed) and assert the
/// "unreadable" token is absent. That gives us a meaningful
/// regression gate without requiring the TUI loop itself to run.
#[test]
fn profile_load_controls_initial_config() {
    if !socat_available() {
        eprintln!("skipping profile_load_controls_initial_config: socat not available");
        return;
    }
    let pty = match PtyPair::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skipping profile_load_controls_initial_config: {e}");
            return;
        }
    };

    let tempdir = tempfile::tempdir().expect("create tempdir");
    let profile_path = tempdir.path().join("pre.toml");
    let mut pre = rtcom_config::Profile::default();
    pre.serial.baud = 19_200;
    rtcom_config::write(&profile_path, &pre).expect("seed profile");

    let stderr = run_rtcom_collecting_stderr(&[
        pty.master.as_os_str().to_str().expect("pty path utf-8"),
        "-c",
        profile_path.to_str().expect("profile path utf-8"),
    ]);

    assert!(
        !stderr.contains("unreadable"),
        "profile at {} unexpectedly reported unreadable; stderr was:\n{}",
        profile_path.display(),
        stderr,
    );
    assert!(
        !stderr.contains("parse error") && !stderr.contains("Parse"),
        "profile load surfaced a parse error; stderr was:\n{stderr}",
    );
}

/// `-c /nonexistent/path/...toml` must NOT be fatal. rtcom's
/// `load_profile` falls back to defaults silently when the path is
/// absent, so stderr should not carry any profile-related warning.
/// Raw-mode failure (from piped stdin) is expected and ignored.
#[test]
fn missing_config_path_is_not_fatal() {
    if !socat_available() {
        eprintln!("skipping missing_config_path_is_not_fatal: socat not available");
        return;
    }
    let pty = match PtyPair::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skipping missing_config_path_is_not_fatal: {e}");
            return;
        }
    };

    let stderr = run_rtcom_collecting_stderr(&[
        pty.master.as_os_str().to_str().expect("pty path utf-8"),
        "-c",
        "/nonexistent/path/to/profile.toml",
    ]);

    assert!(
        !stderr.contains("unreadable"),
        "missing profile unexpectedly reported unreadable; stderr was:\n{stderr}",
    );
    assert!(
        !stderr.contains("--save failed"),
        "missing profile unexpectedly flagged a save error; stderr was:\n{stderr}",
    );
}

/// Spawn rtcom with `stdin=/dev/null` so raw-mode setup fails fast,
/// then collect everything the process writes to stderr in the
/// window between spawn and either self-exit or the 2 s safety
/// timeout.
///
/// Kept as a helper because every non-save PTY test uses the same
/// shape and the same "we only care about stderr patterns" contract.
fn run_rtcom_collecting_stderr(args: &[&str]) -> String {
    let binary = assert_cmd::cargo::cargo_bin("rtcom");
    let mut child = Command::new(&binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtcom");

    // Let rtcom parse the profile and reach raw-mode failure.
    let status = wait_with_timeout(&mut child, Duration::from_secs(2));
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }

    // Drain stderr. wait_with_timeout already reaped exit status
    // if available; either way the pipe is still readable post-exit.
    let mut stderr_buf = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        // Bound the read so a misbehaving rtcom can't hang us: at
        // this point the process has exited (status.is_some()) or
        // been killed, so stderr should EOF promptly.
        let start = Instant::now();
        let mut buf = [0u8; 4096];
        loop {
            match stderr.read(&mut buf) {
                Ok(n) if n > 0 => stderr_buf.push_str(&String::from_utf8_lossy(&buf[..n])),
                // EOF (`Ok(0)`) or read error: nothing more we can
                // safely grab, stop.
                _ => break,
            }
            if start.elapsed() > Duration::from_secs(1) {
                break;
            }
        }
    }
    stderr_buf
}
