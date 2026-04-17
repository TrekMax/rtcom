//! End-to-end tests for the `rtcom` binary.
//!
//! Spawns a real `socat` process to allocate a PTY pair, then drives
//! the `rtcom` binary against one end while the test writes/reads on
//! the other. Tests are gated to Linux because the e2e shape (socat +
//! `/dev/pts/N`) is what the spec calls for; macOS and Windows boxes
//! get the unit + integration coverage in `rtcom-core` instead.
//!
//! Skipped automatically when `socat` is not installed — keeps `cargo
//! test` green on a minimal dev box.

#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// `^A` — the default escape key used by the binary (matches the
/// `default_value = "^A"` on `Cli::escape`). Tests rely on the
/// production default; if it changes here, change it there too.
const ESC: u8 = 0x01;

const RTCOM_BIN: &str = env!("CARGO_BIN_EXE_rtcom");

/// Fast-fail upper bound for any single child wait. CI runners with
/// loaded I/O can be slow; 5s is generous but still keeps the test
/// suite under the 30s budget the spec calls out.
const STEP: Duration = Duration::from_secs(5);

/// True when the host has `socat` on `$PATH`. Tests skip when false.
fn socat_available() -> bool {
    Command::new("socat")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// PTY pair held open by a backgrounded `socat` process. Both `a` and
/// `b` are slave-side device paths usable as `/dev/ttyXXX` substitutes.
/// Drop kills the `socat` process and deallocates the PTYs.
struct PtyPair {
    socat: Child,
    a: String,
    b: String,
}

impl PtyPair {
    /// Spawns `socat -d -d PTY,raw,echo=0 PTY,raw,echo=0` and parses
    /// the two `"PTY is /dev/pts/N"` lines out of its stderr.
    fn spawn() -> Option<Self> {
        let mut child = Command::new("socat")
            .args(["-d", "-d", "PTY,raw,echo=0", "PTY,raw,echo=0"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;

        let stderr = child.stderr.take()?;
        let mut paths = Vec::with_capacity(2);
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(idx) = line.find("PTY is ") {
                let path = line[idx + "PTY is ".len()..].trim().to_string();
                paths.push(path);
                if paths.len() == 2 {
                    break;
                }
            }
        }
        if paths.len() != 2 {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }

        // Give socat a moment to enter its data-transfer loop. Without
        // this, an immediate write on one end can be lost because socat
        // hasn't started forwarding yet.
        thread::sleep(Duration::from_millis(100));

        Some(Self {
            socat: child,
            a: paths.remove(0),
            b: paths.remove(0),
        })
    }
}

impl Drop for PtyPair {
    fn drop(&mut self) {
        let _ = self.socat.kill();
        let _ = self.socat.wait();
    }
}

/// Polls `child.try_wait` until it exits or `timeout` expires. Kills
/// the child on timeout so the test does not leak a process.
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code();
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// Runs `read` in a thread so the test can give up after `timeout`
/// rather than blocking forever on a quiet pipe.
fn read_with_timeout<R>(mut reader: R, n: usize, timeout: Duration) -> Vec<u8>
where
    R: Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = vec![0_u8; n];
        let mut filled = 0;
        while filled < n {
            match reader.read(&mut buf[filled..]) {
                // EOF or read error: nothing more is coming.
                Ok(0) | Err(_) => break,
                Ok(k) => filled += k,
            }
        }
        buf.truncate(filled);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(timeout).unwrap_or_default()
}

fn spawn_rtcom(device: &str) -> Child {
    Command::new(RTCOM_BIN)
        .arg(device)
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rtcom")
    // Note: stderr is piped to null because tracing writes there.
    // Production users want it; tests do not.
}

/// Sentinel used at the start of every test to bail out on hosts that
/// lack `socat`. Uses `eprintln!` rather than `cargo:rerun-if` so
/// running locally without socat just yields a green skip.
fn require_socat() -> bool {
    if socat_available() {
        return true;
    }
    eprintln!("skipping e2e test: socat not on PATH");
    false
}

#[test]
fn quit_command_via_stdin_exits_with_zero() {
    if !require_socat() {
        return;
    }
    let pty = PtyPair::spawn().expect("allocate pty pair");
    let mut rtcom = spawn_rtcom(&pty.a);

    // Give rtcom enough time to open the device + spawn its tasks.
    thread::sleep(Duration::from_millis(200));

    {
        let mut stdin = rtcom.stdin.take().expect("stdin");
        stdin.write_all(&[ESC, b'q']).expect("write quit sequence");
    }

    let exit = wait_with_timeout(&mut rtcom, STEP);
    assert_eq!(exit, Some(0), "expected clean exit, got {exit:?}");
}

#[test]
fn external_writes_appear_on_rtcom_stdout() {
    if !require_socat() {
        return;
    }
    let pty = PtyPair::spawn().expect("allocate pty pair");
    let mut rtcom = spawn_rtcom(&pty.a);

    thread::sleep(Duration::from_millis(300));

    // Write on the other PTY end; bytes flow through socat to rtcom's
    // device side, which publishes Event::RxBytes -> renderer writes
    // them to its piped stdout.
    {
        let mut other = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pty.b)
            .expect("open other end");
        other.write_all(b"hello").expect("write to other end");
        other.flush().expect("flush");
    }

    let stdout = rtcom.stdout.take().expect("stdout");
    let received = read_with_timeout(stdout, 5, STEP);
    assert_eq!(&received, b"hello");

    // Tell rtcom to quit so wait() does not hang.
    {
        let mut stdin = rtcom.stdin.take().expect("stdin");
        let _ = stdin.write_all(&[ESC, b'q']);
    }
    let exit = wait_with_timeout(&mut rtcom, STEP);
    assert_eq!(exit, Some(0));
}

#[test]
fn rtcom_stdin_bytes_reach_external_end() {
    if !require_socat() {
        return;
    }
    let pty = PtyPair::spawn().expect("allocate pty pair");
    let mut rtcom = spawn_rtcom(&pty.a);

    thread::sleep(Duration::from_millis(300));

    // Open the other end for reading; spawn the read on a thread
    // before sending so we never miss the bytes.
    let other = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&pty.b)
        .expect("open other end");
    let received_handle = thread::spawn(move || read_with_timeout(other, 4, STEP));

    {
        let mut stdin = rtcom.stdin.take().expect("stdin");
        stdin.write_all(b"ping").expect("write ping");
        stdin.write_all(&[ESC, b'q']).expect("write quit");
    }

    let received = received_handle.join().expect("read thread join");
    assert_eq!(&received, b"ping");

    let exit = wait_with_timeout(&mut rtcom, STEP);
    assert_eq!(exit, Some(0));
}
