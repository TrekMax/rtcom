# ADR-009 — Back the serial pane with `vt100`

**Date**: 2026-04-18
**Status**: Accepted
**Context**: v0.2 plan

## Context

The serial pane needs to interpret incoming ANSI escape sequences
correctly — remote apps (ncurses UIs, microcontroller shells, etc.)
emit cursor movements, colour changes, and clear-screen codes. A naïve
"append bytes to a line buffer" approach garbles them.

## Decision

Wrap each session's serial pane with the
[`vt100`](https://crates.io/crates/vt100) crate's Parser + Screen.
Bytes received from the device flow through `vt100::Parser::process`,
producing a 2D cell grid that ratatui can render as a widget (via
`tui-term`'s `PseudoTerminal`).

## Consequences

- Full VT100 compatibility out of the box.
- Memory budget: 80-col × 10 000-row scrollback ≈ 800 KiB per pane.
  Acceptable for single-pane use; revisit when multi-pane (v1.0) lands.
- Gets us "copy/paste from serial pane" largely for free once the
  selection UI is wired (post-v0.2).
- `tui-term` 0.1 is thin and stable; no forked maintenance burden.

## Alternatives considered

- **`anstyle-parse`**: ANSI-style-only; doesn't track cursor or
  scrollback. Works for a log viewer but not a terminal emulator.
- **`alacritty_terminal`**: richer feature set but heavy; pulls a
  substantial GPU/font stack even when we don't use it.
- **Hand-rolled VT100 parser**: rejected — terminal emulation is its
  own full-time project.
