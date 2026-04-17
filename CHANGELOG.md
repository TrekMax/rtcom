# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Default command-key escape switched from `^T` (Ctrl-T) to `^A`
  (Ctrl-A). Picocom's historical default; survives tmux's prefix
  binding and terminal emulators that use Ctrl-T for "new tab".
  Override with `--escape '^T'` to restore the previous behaviour.

### Fixed

- Terminal renderer now surfaces `Event::DeviceDisconnected` as a
  `*** rtcom: device disconnected: <reason>` system message, with a
  post-cancel drain so the message is not lost when `main` trips
  the cancellation token immediately after `Session::run` exits on
  a disconnect.
- `main` propagates session shutdown to the stdin reader and
  terminal renderer via a cloned cancel token, fixing a hang when
  the device disappears (previously `stdin` and the renderer kept
  running with nothing to do).
- TTY-stdin sessions now print a quit-key hint at startup so users
  can find their way out without consulting the man page:
  `rtcom: press ^A q to quit (Ctrl-C is sent to the device in raw mode)`.

## [0.1.0] — 2026-04-17

First tagged release. Establishes the workspace, the library API,
and the baseline CLI — enough to replace `picocom` for most
day-to-day "connect to a serial device, type at it, see its output"
use cases.

### Added

- **`rtcom-core`** library crate with the public API:
  - `SerialDevice` trait (`AsyncRead + AsyncWrite` + control plane:
    baud, framing, DTR/RTS, line break, modem status, cached config).
  - `SerialPortDevice` backend on top of `tokio-serial`; Unix-only
    `pair()` helper for PTY-based testing.
  - `SerialConfig` / `DataBits` / `StopBits` / `Parity` /
    `FlowControl` / `ModemStatus` value types with `validate()`.
  - `Event` enum (`RxBytes`, `TxBytes`, `Command`, `SystemMessage`,
    `DeviceConnected`, `DeviceDisconnected`, `ConfigChanged`,
    `Error`) and `EventBus` thin wrapper over
    `tokio::sync::broadcast`.
  - `Session<D>` orchestrator: single-task select loop that owns
    the device, drives reads/writes, and dispatches commands.
  - `CommandKeyParser` state machine for the `^T`-style command
    prefix (`?`/`h`, `q`/`x`, `c`, `t`, `g`, `\`, `b<rate><Enter>`,
    escape-literal, Esc-cancel, unknown-drop).
  - `Mapper` trait and `LineEndingMapper` with the picocom-style
    rules (`None` / `AddCrToLf` / `AddLfToCr` / `DropCr` /
    `DropLf`). `Session::with_omap` / `with_imap` builders apply
    them on the fly.
  - `UucpLock`: UUCP-format PID lock file with `/var/lock` -> `/tmp`
    fallback on Unix, no-op on Windows; RAII drop clears the file.
  - `Error` enum with `Io` / `Backend` / `InvalidConfig` /
    `AlreadyLocked` / `InvalidLock` variants (`#[non_exhaustive]`).
- **`rtcom-cli`** binary crate (`rtcom`):
  - `clap`-derived `Cli` with the full option surface (device,
    `-b/-d/-s/-p/-f`, `--omap`/`--imap`/`--emap`,
    `--no-reset`/`--echo`, `--escape`, `-q`, `-v`).
  - Raw-mode guard with chained panic hook; automatically skipped
    when stdin is not a TTY so scripts and tests work.
  - `SignalListener` built on `tokio::signal` for Unix signals
    (SIGINT / SIGTERM / SIGHUP / SIGWINCH log-only) and Windows
    (Ctrl-C / Ctrl-Break). Signals trip the session's
    cancellation token rather than `process::exit`, preserving
    the full `Drop` chain.
  - `run_stdin_reader` task converting keyboard bytes into
    `Event::TxBytes` / `Event::Command` via `CommandKeyParser`.
  - `run_terminal_renderer` task writing `Event::RxBytes` verbatim
    and `Event::SystemMessage` with a `*** rtcom: ` prefix.
  - `tracing` / `tracing-subscriber` initialisation honouring
    `RUST_LOG` and the `-v` verbosity count.
  - Exit-code convention: `0` clean, `1` startup error, `128 + N`
    for signal termination.
- **Tests**:
  - `rtcom-core` unit + integration suites (~52 tests covering
    parser transitions, mapper rules, session command dispatch,
    UUCP lock behaviour, PTY round-trip).
  - `rtcom-cli` module tests for CLI projection, stdin reader,
    terminal renderer, signal exit-code helpers (~36 tests).
  - Linux-only end-to-end suite in `crates/rtcom-cli/tests/e2e_basic.rs`:
    three socat-PTY scenarios (quit via stdin, external writes
    surface on stdout, stdin bytes reach the other PTY end). Full
    suite runs in well under the 30 s budget.
- **Docs**:
  - Architecture + design rationale in [`CLAUDE.md`](./CLAUDE.md).
  - README with installation, quick start, command-key cheatsheet,
    and comparison table vs picocom / tio.
  - [`man/rtcom.1`](./man/rtcom.1) for `man rtcom`.

### Project scaffolding

- Cargo workspace with shared lints (`deny(unsafe_code)`, clippy
  `pedantic` + `nursery`, `warn(missing_docs)`), pinned Rust 1.85
  toolchain via `rust-toolchain.toml`.
- GitHub Actions CI matrix (ubuntu / macos / windows) running fmt,
  clippy `-D warnings`, tests, and `cargo doc -D warnings`.

[Unreleased]: https://github.com/TrekMax/rtcom/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/TrekMax/rtcom/releases/tag/v0.1.0
