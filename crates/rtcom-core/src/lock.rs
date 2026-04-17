//! UUCP-style lock files.
//!
//! Multiple programs sharing a serial port is a recipe for corrupt data
//! and confused users. The UUCP convention — adopted by `uucp`,
//! `picocom`, `tio`, `minicom`, and most modems-with-fortunes utilities
//! since the 1980s — solves this with a per-device flag file:
//!
//! - **Path**: `/var/lock/LCK..<basename>` for `<basename>` derived from
//!   the device path (`/dev/ttyUSB0` → `ttyUSB0`).
//! - **Content**: the owning process's PID, formatted as 10 ASCII
//!   characters right-aligned with leading spaces, followed by `\n`.
//! - **Stale-lock recovery**: a process opening the device reads the
//!   PID, sends signal 0 with `kill(pid, 0)`. `Ok` means the holder is
//!   alive, refuse to open. `ESRCH` means the holder is gone, remove
//!   the file and proceed.
//!
//! `/var/lock` is typically `root:lock 1775` on modern distros, so an
//! unprivileged user without group `lock` cannot create files there.
//! [`UucpLock::acquire`] falls back to `/tmp` in that case (with a
//! `tracing::warn`) — the lock is then per-user instead of system-wide,
//! but still protects the same user from racing themselves.
//!
//! On Windows, `UucpLock` is a no-op shim — the OS already serialises
//! `CreateFile` on a COM port via `SHARE_MODE = 0`. v0.1 leaves the
//! Windows path empty so the call site stays cross-platform.
//!
//! Stub: only the public API shape is in place; the green commit fills
//! the body.

use std::path::{Path, PathBuf};

use crate::Result;

/// RAII handle for a UUCP lock file. Drops it when this value goes out
/// of scope.
#[derive(Debug)]
pub struct UucpLock {
    path: PathBuf,
    /// `false` on Windows / non-Unix targets where the lock is a no-op.
    /// Drop must skip `remove_file` for these.
    active: bool,
}

impl UucpLock {
    /// Acquires a lock for `device_path`, trying `/var/lock` first and
    /// falling back to `/tmp` on permission errors.
    ///
    /// # Errors
    ///
    /// - [`Error::AlreadyLocked`](crate::Error::AlreadyLocked) if a
    ///   live process already owns the device.
    /// - [`Error::Io`](crate::Error::Io) for filesystem failures the
    ///   fallback cannot recover from.
    pub fn acquire(device_path: &str) -> Result<Self> {
        let _ = device_path;
        todo!("UucpLock::acquire — implementation lands in the green commit")
    }

    /// Acquires a lock for `device_path` in the explicit `lock_dir`.
    /// Tests use this entry to point the lock at a temporary directory
    /// instead of the system-wide path.
    ///
    /// # Errors
    ///
    /// Same as [`UucpLock::acquire`] but without the `/tmp` fallback.
    pub fn acquire_in(device_path: &str, lock_dir: &Path) -> Result<Self> {
        let _ = (device_path, lock_dir);
        todo!("UucpLock::acquire_in — implementation lands in the green commit")
    }

    /// Returns the lock file path this guard owns.
    #[must_use]
    pub fn lock_file_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for UucpLock {
    fn drop(&mut self) {
        if self.active {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Returns the basename of `path` (the part after the last `/`), or the
/// whole string if no separator is present.
#[allow(dead_code, reason = "called by acquire_in once the green commit lands")]
fn basename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn basename_strips_directory_components() {
        assert_eq!(basename_of("/dev/ttyUSB0"), "ttyUSB0");
        assert_eq!(basename_of("ttyS1"), "ttyS1");
        assert_eq!(basename_of("/a/b/c/d"), "d");
    }

    #[test]
    fn acquire_in_creates_lock_file_at_uucp_path() {
        let dir = tempdir().unwrap();
        let lock = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap();
        let expected = dir.path().join("LCK..ttyUSB0");
        assert_eq!(lock.lock_file_path(), &expected);
        assert!(expected.exists(), "lock file should exist on disk");
    }

    #[test]
    fn acquire_in_writes_pid_in_uucp_format() {
        let dir = tempdir().unwrap();
        let _lock = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("LCK..ttyUSB0")).unwrap();
        // Format is "%10d\n": 10 chars right-aligned, trailing newline.
        assert_eq!(content.len(), 11, "expected 10 PID chars + LF: {content:?}");
        assert!(content.ends_with('\n'), "trailing LF: {content:?}");
        let parsed: i32 = content.trim().parse().expect("decimal PID");
        #[allow(clippy::cast_possible_wrap)]
        let our_pid = std::process::id() as i32;
        assert_eq!(parsed, our_pid);
    }

    #[test]
    fn drop_removes_lock_file() {
        let dir = tempdir().unwrap();
        let path = {
            let lock = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap();
            lock.lock_file_path().to_path_buf()
        };
        assert!(!path.exists(), "lock file should be removed on Drop");
    }

    #[test]
    fn second_acquire_for_same_device_reports_already_locked() {
        let dir = tempdir().unwrap();
        let _first = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap();
        let err = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap_err();
        match err {
            crate::Error::AlreadyLocked { device, pid, .. } => {
                assert_eq!(device, "/dev/ttyUSB0");
                #[allow(clippy::cast_possible_wrap)]
                let our_pid = std::process::id() as i32;
                assert_eq!(pid, our_pid);
            }
            other => panic!("expected AlreadyLocked, got {other:?}"),
        }
    }

    #[test]
    fn stale_lock_with_dead_pid_is_overwritten() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("LCK..ttyUSB0");
        // PID well above any realistic kernel.pid_max; kill(pid, 0)
        // will return ESRCH.
        fs::write(&lock_path, "1999999999\n").unwrap();

        let lock = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap();
        let content = fs::read_to_string(lock.lock_file_path()).unwrap();
        let parsed: i32 = content.trim().parse().unwrap();
        #[allow(clippy::cast_possible_wrap)]
        let our_pid = std::process::id() as i32;
        assert_eq!(parsed, our_pid);
    }

    #[test]
    fn stale_lock_with_garbage_content_is_overwritten() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("LCK..ttyUSB0");
        fs::write(&lock_path, "not-a-pid\n").unwrap();

        let lock = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap();
        // We should now own the file with our own PID.
        #[allow(clippy::cast_possible_wrap)]
        let our_pid = std::process::id() as i32;
        let content = fs::read_to_string(lock.lock_file_path()).unwrap();
        assert_eq!(content.trim().parse::<i32>().unwrap(), our_pid);
    }

    #[test]
    fn unrelated_lock_files_are_left_alone() {
        let dir = tempdir().unwrap();
        let other = dir.path().join("LCK..ttyS9");
        fs::write(&other, "1\n").unwrap();
        let _lock = UucpLock::acquire_in("/dev/ttyUSB0", dir.path()).unwrap();
        assert!(other.exists(), "we must not touch unrelated lock files");
    }
}
