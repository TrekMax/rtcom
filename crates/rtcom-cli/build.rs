//! Build-time script: emit `RTCOM_VERSION` so the CLI can render
//! `0.1.2 (abc12345)` or `0.1.2 (abc12345-dirty)` instead of the bare
//! Cargo package version.
//!
//! Hash resolution order:
//!
//! 1. **Local git checkout**: `git rev-parse --short=8 HEAD`. Also
//!    detects `git diff` for the `-dirty` suffix.
//! 2. **crates.io tarball**: `.commit-hash` file in the crate root.
//!    The release workflow writes this just before `cargo publish`
//!    so users who `cargo install rtcom-cli` get the same hash the
//!    GitHub release was built from. Tarballs are never "dirty".
//! 3. **Neither**: bare `CARGO_PKG_VERSION`.

use std::process::Command;

fn main() {
    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());

    let version = match resolve_hash() {
        Some((hash, dirty)) => {
            let suffix = if dirty { "-dirty" } else { "" };
            format!("{pkg_version} ({hash}{suffix})")
        }
        None => pkg_version,
    };

    println!("cargo:rustc-env=RTCOM_VERSION={version}");

    // Re-run when HEAD moves (local dev) or the baked file changes
    // (tarball install) so the embedded hash stays current without a
    // forced clean rebuild.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    println!("cargo:rerun-if-changed=.commit-hash");
}

/// Returns `(hash, is_dirty)`. `None` means "no hash discoverable".
fn resolve_hash() -> Option<(String, bool)> {
    if let Some(hash) = short_hash() {
        // Local git checkout: dirty status is meaningful.
        return Some((hash, working_tree_is_dirty()));
    }
    // Fall back to the baked file the release workflow writes for
    // crates.io publishes. Tarballs are immutable -> never dirty.
    if let Some(hash) = read_baked_hash() {
        return Some((hash, false));
    }
    None
}

/// `git rev-parse --short=8 HEAD`, or `None` if git is unavailable
/// (crates.io tarball, no .git, ...).
fn short_hash() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let hash = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if hash.is_empty() {
        None
    } else {
        Some(hash)
    }
}

/// Reads `.commit-hash` from the crate root (where `Cargo.toml`
/// lives). The release workflow writes this before `cargo publish`.
fn read_baked_hash() -> Option<String> {
    let raw = std::fs::read_to_string(".commit-hash").ok()?;
    let hash = raw.trim().to_string();
    if hash.is_empty() {
        None
    } else {
        Some(hash)
    }
}

/// `true` iff `git diff --quiet` reports tracked-file changes. False
/// for clean working trees and for environments where git is missing.
fn working_tree_is_dirty() -> bool {
    Command::new("git")
        .args(["diff", "--quiet"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false)
}
