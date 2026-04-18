//! Shared test helpers for the rtcom-cli end-to-end suites.
//!
//! The primary export is [`PtyPair`], a drop-on-exit wrapper around
//! `socat -d -d PTY,raw,echo=0 PTY,raw,echo=0`. Socat logs the two
//! allocated PTY paths to stderr; we parse them and hand both back to
//! callers so one end can be fed to rtcom while the other stays free
//! for the test harness.
//!
//! [`socat_available`] lets individual tests skip gracefully on dev
//! boxes or CI runners that lack socat, rather than hard-failing.

#![allow(dead_code)]

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// A live socat PTY pair. Dropping the value kills the socat process,
/// which in turn releases both pseudo-terminals.
pub struct PtyPair {
    /// First PTY path printed by socat. rtcom connects to this end in
    /// tests; the naming is a convention, not a hardware distinction.
    pub master: PathBuf,
    /// Second PTY path printed by socat. Free for the test harness to
    /// read/write if a scenario needs to observe traffic.
    pub slave: PathBuf,
    socat: Child,
}

impl PtyPair {
    /// Spawn `socat -d -d PTY,raw,echo=0 PTY,raw,echo=0` and parse the
    /// two allocated PTY paths from stderr.
    ///
    /// Socat's `-d -d` dumps two lines of the form `... PTY is /dev/pts/N`
    /// on stderr at startup; we harvest them in order and return once
    /// both are captured.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if socat cannot be spawned (typically: not
    /// installed) or if stderr parsing fails to yield two paths within
    /// the first handful of lines.
    pub fn new() -> Result<Self, String> {
        let mut child = Command::new("socat")
            .args(["-d", "-d", "PTY,raw,echo=0", "PTY,raw,echo=0"])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn socat: {e}"))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "socat stderr was not piped".to_string())?;
        let reader = BufReader::new(stderr);

        let mut ptys: Vec<PathBuf> = Vec::with_capacity(2);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(idx) = line.find("PTY is ") {
                let path = line[idx + "PTY is ".len()..].trim().to_string();
                ptys.push(PathBuf::from(path));
                if ptys.len() == 2 {
                    break;
                }
            }
        }

        if ptys.len() != 2 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("socat did not emit 2 PTY paths (got {ptys:?})"));
        }

        // socat needs a breath after announcing the PTYs before they
        // are actually usable for open(2). Without this the first
        // rtcom open occasionally races and fails.
        thread::sleep(Duration::from_millis(100));

        let slave = ptys.remove(1);
        let master = ptys.remove(0);
        Ok(Self {
            master,
            slave,
            socat: child,
        })
    }
}

impl Drop for PtyPair {
    fn drop(&mut self) {
        let _ = self.socat.kill();
        let _ = self.socat.wait();
    }
}

/// Return `true` if `socat` is discoverable in `PATH`.
///
/// Tests that depend on socat call this first and `eprintln!` + `return`
/// when absent, so the test suite stays green on bare-bones
/// environments while still exercising the path wherever socat exists.
#[must_use]
pub fn socat_available() -> bool {
    Command::new("socat")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Poll `child.try_wait()` until it returns `Ok(Some(..))` or the
/// timeout elapses. Returns `None` if the child is still running at
/// the deadline.
///
/// Keeping this helper out of the individual tests means every PTY
/// test uses the same polling cadence (50 ms) and the same timeout
/// semantics.
#[must_use]
pub fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    None
}
