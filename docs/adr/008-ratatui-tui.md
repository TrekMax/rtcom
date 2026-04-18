# ADR-008 — Use ratatui for the TUI

**Date**: 2026-04-18
**Status**: Accepted
**Context**: v0.2 plan

## Context

v0.1 of rtcom rendered serial data line-by-line to stdout. v0.2
introduces a configuration menu, modal dialogs, toast notifications,
and a vt100-backed serial pane — all of which require real screen-buffer
management.

## Decision

Adopt [ratatui](https://github.com/ratatui-org/ratatui) as the TUI
framework. Pin to 0.28.x for v0.2 (MSRV 1.85 constraint rules out
0.29+ at the time of writing).

## Consequences

- New dep + transitives; binary size ↑ ~400 KiB (thin LTO).
- TestBackend (included in ratatui) enables snapshot-driven test
  coverage; the v0.2 suite lands ~10 TUI snapshot tests.
- Alternate-screen lifecycle must be RAII-guarded — any panic in TUI
  code now needs termios recovery, which `RawModeGuard::Drop` handles.
- Binary no longer usable with piped stdin/stdout (not a supported
  mode post-v0.2).

## Alternatives considered

- **`tui-rs`**: predecessor to ratatui; unmaintained since 2022.
- **Hand-rolled** renderer with crossterm only: rejected — every
  v1.0+ feature (multi-port compare view, scrollback, selection)
  would need manual re-invention.
- **`termwiz`** (wezterm's internal stack): viable but heavier; pulls
  ~2x the transitives of ratatui without comparable ecosystem support.
