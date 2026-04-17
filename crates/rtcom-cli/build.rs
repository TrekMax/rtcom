//! Build-time script: emit `RTCOM_VERSION` so the CLI can render
//! `0.1.0 (abc12345)` or `0.1.0 (abc12345-dirty)` instead of the bare
//! Cargo package version. Falls back to the bare version when git
//! information is unavailable (crates.io tarball builds, etc.).

use std::process::Command;

fn main() {
    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());

    let version = match short_hash() {
        Some(hash) => {
            let suffix = if working_tree_is_dirty() {
                "-dirty"
            } else {
                ""
            };
            format!("{pkg_version} ({hash}{suffix})")
        }
        None => pkg_version,
    };

    println!("cargo:rustc-env=RTCOM_VERSION={version}");

    // Re-run when HEAD moves so the embedded hash stays current. Path
    // is relative to the crate root (crates/rtcom-cli/) -> workspace
    // .git is two levels up.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
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

/// `true` iff `git diff --quiet` reports tracked-file changes. False
/// for clean working trees and for environments where git is missing.
fn working_tree_is_dirty() -> bool {
    Command::new("git")
        .args(["diff", "--quiet"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false)
}
