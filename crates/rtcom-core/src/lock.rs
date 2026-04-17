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
use std::path::{Path, PathBuf};

use crate::Result;

/// Default system-wide UUCP lock directory.
#[cfg(unix)]
const PRIMARY_LOCK_DIR: &str = "/var/lock";

/// Per-user fallback directory used when [`PRIMARY_LOCK_DIR`] is
/// unwritable.
#[cfg(unix)]
const FALLBACK_LOCK_DIR: &str = "/tmp";

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
    /// falling back to `/tmp` on permission / not-found errors. On
    /// non-Unix targets this is a no-op.
    ///
    /// # Errors
    ///
    /// - [`Error::AlreadyLocked`](crate::Error::AlreadyLocked) if a
    ///   live process already owns the device.
    /// - [`Error::Io`](crate::Error::Io) for filesystem failures the
    ///   fallback cannot recover from.
    pub fn acquire(device_path: &str) -> Result<Self> {
        #[cfg(not(unix))]
        {
            let _ = device_path;
            return Ok(Self {
                path: PathBuf::new(),
                active: false,
            });
        }
        #[cfg(unix)]
        match Self::acquire_in(device_path, Path::new(PRIMARY_LOCK_DIR)) {
            Ok(lock) => Ok(lock),
            Err(crate::Error::Io(err)) if can_fallback(&err) => {
                tracing::warn!(
                    primary = PRIMARY_LOCK_DIR,
                    fallback = FALLBACK_LOCK_DIR,
                    error = %err,
                    "UUCP lock falling back to per-user directory",
                );
                Self::acquire_in(device_path, Path::new(FALLBACK_LOCK_DIR))
            }
            Err(other) => Err(other),
        }
    }

    /// Acquires a lock for `device_path` in the explicit `lock_dir`.
    /// Tests use this entry to point the lock at a temporary directory
    /// instead of the system-wide path. On non-Unix targets this is a
    /// no-op.
    ///
    /// # Errors
    ///
    /// Same as [`UucpLock::acquire`] but without the `/tmp` fallback.
    #[cfg_attr(not(unix), allow(unused_variables))]
    pub fn acquire_in(device_path: &str, lock_dir: &Path) -> Result<Self> {
        #[cfg(not(unix))]
        {
            return Ok(Self {
                path: PathBuf::new(),
                active: false,
            });
        }
        #[cfg(unix)]
        {
            use std::fs::{self, OpenOptions};
            use std::io::Write;

            let basename = basename_of(device_path);
            let lock_path = lock_dir.join(format!("LCK..{basename}"));

            // Pre-existing lock: probe liveness, take over if stale.
            if lock_path.exists() {
                match read_pid(&lock_path) {
                    Ok(pid) if pid_is_alive(pid) => {
                        return Err(crate::Error::AlreadyLocked {
                            device: device_path.to_string(),
                            pid,
                            lock_file: lock_path,
                        });
                    }
                    // Stale (dead PID) or unreadable (garbage / parse error):
                    // remove and continue. We deliberately swallow the
                    // remove error — if it fails, OpenOptions::create_new
                    // below will fail with EEXIST and we surface that.
                    _ => {
                        let _ = fs::remove_file(&lock_path);
                    }
                }
            }

            // O_CREAT | O_EXCL — atomic against a racing process that
            // landed between our exists() check and this open call.
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)?;

            // UUCP convention: 10-character right-aligned ASCII PID + LF.
            #[allow(clippy::cast_possible_wrap)]
            let pid = std::process::id() as i32;
            writeln!(f, "{pid:>10}")?;
            f.sync_all()?;

            Ok(Self {
                path: lock_path,
                active: true,
            })
        }
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
fn basename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Reads a UUCP lock file and parses its content as a PID.
#[cfg(unix)]
fn read_pid(lock_path: &Path) -> Result<i32> {
    let content = std::fs::read_to_string(lock_path)?;
    content
        .trim()
        .parse::<i32>()
        .map_err(|err| crate::Error::InvalidLock(format!("{err} in {content:?}")))
}

/// `kill(pid, 0)` returns `Ok` for a live process, `ESRCH` for a dead
/// one, `EPERM` for "exists but not ours". Treat the latter two as
/// "alive enough to refuse the lock" only when the OS says alive.
#[cfg(unix)]
fn pid_is_alive(pid: i32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    matches!(kill(Pid::from_raw(pid), None), Ok(()))
}

/// Categorises filesystem errors that justify falling back from
/// `/var/lock` to `/tmp`. Permission denied is the common case (no
/// `lock` group); not-found means the directory does not exist on this
/// system at all (some minimal containers).
#[cfg(unix)]
fn can_fallback(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
    )
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
