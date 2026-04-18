# ADR-010 — `directories` crate for XDG/platform-native config path

**Date**: 2026-04-18
**Status**: Accepted
**Context**: v0.2 plan

## Context

rtcom v0.2 persists user configuration to a TOML file. We need a
platform-correct default location: `$XDG_CONFIG_HOME/rtcom/default.toml`
on Linux/BSD, `~/Library/Application Support/rtcom/default.toml` on
macOS, `%APPDATA%\rtcom\default.toml` on Windows.

## Decision

Use [`directories`](https://crates.io/crates/directories) 5.x via
`ProjectDirs::from("", "", "rtcom")`.

## Consequences

- Small dep (~1 KB of Rust source + unix/win32 platform code).
- The empty qualifier/organization arguments mean the macOS path is
  `Application Support/rtcom/` rather than Apple's preferred
  `dev.trekmax.rtcom/`. Users migrating from future reverse-DNS
  conventions will need to move the file manually.
- MSRV compatible (5.x stays on Rust 1.70+).

## Alternatives considered

- **Hand-roll** per-platform path logic: 3× the code, 3× the bugs.
- **`directories-next`**: fork of `directories`, but the main crate
  resumed active maintenance; no reason to use the fork.
- **`dirs` 5.x**: lower-level; doesn't embed the app-name convention.
  Would need a wrapper anyway.
