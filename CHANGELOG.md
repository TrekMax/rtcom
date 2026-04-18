# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] — 2026-04-18

Patch release cleaning up the partially-published v0.2.0: `rtcom-core
0.2.0` made it to crates.io, but `rtcom-config`, `rtcom-tui`, and
`rtcom-cli` did not — this release ships the full four-crate set and
folds in the post-tag security updates.

### Changed

- **MSRV bumped from 1.85 to 1.88** (driven by the dependency
  upgrades below).

### Security / Dependencies

- Upgraded `ratatui` `0.28` → `0.30` and `tui-term` `0.1` → `0.3` to
  pick up `lru 0.16.3+`, which patches [GHSA-rhfx-m35p-ff5j]
  (`IterMut` Stacked Borrows soundness; CVSS v4 LOW 2.7). `lru` is a
  transitive dep via `ratatui-core`; the advisory is not reachable
  from rtcom's own code but the fix flows in automatically. Also
  pulls `vt100` `0.15` → `0.16` to match tui-term 0.3's `Screen`
  trait bound.
- Upgraded `time` to `0.3.47+` to patch [GHSA-r6v5-fh4h-64xc] (stack
  exhaustion DoS via untrusted time parsing; Medium severity). rtcom
  does not parse time from untrusted input, so the risk is low, but
  the patched version is free to pick up once MSRV allows it.
- Added `.github/dependabot.yml` that ignores the `rand < 0.9.3`
  advisory ([GHSA-cq8v-f236-94qc], CVSS 0 Low). `rand 0.8.6` appears
  in `Cargo.lock` only as a ghost entry pulled by the unused
  `termwiz` feature of `ratatui` (via `phf_generator`); `cargo tree
  --workspace --all-features` confirms it is never compiled into the
  binary.

### Fixed (release pipeline)

- `release.yml` now publishes all four crates in dependency order
  (`rtcom-core` → `rtcom-config` → `rtcom-tui` → `rtcom-cli`).
  Previously only `rtcom-core` and `rtcom-cli` were listed; the
  `rtcom-cli` step failed during v0.2.0 because its path deps were
  not yet on crates.io.

## [0.2.0] — 2026-04-18

### Added

- Full-screen ratatui TUI with three modal styles (overlay / dimmed /
  fullscreen). See [`docs/tui.md`](./docs/tui.md).
- `^A m` opens a minicom-style configuration menu covering serial port
  setup, line endings, modem lines, profile save/load, and screen
  options.
- Profile persistence via `~/.config/rtcom/default.toml` (XDG standard,
  platform-native equivalents on macOS / Windows).
- `-c PATH` / `--config PATH` to override the profile location.
- `--save` writes the effective startup configuration to the profile.
- `Event::MenuOpened` / `Event::MenuClosed` / `Event::ProfileSaved` /
  `Event::ProfileLoadFailed` / `Event::ModemLinesChanged` for
  subscribers (log capture, scripts).
- `Session::apply_config` applies a full `SerialConfig` atomically with
  rollback on partial failure.
- Toast notifications for profile IO + errors (3-second auto-dismiss).
- `LineEndingConfig`, `ModemLineSnapshot`, `ModalStyle` public types
  for downstream consumers.
- Snapshot-tested UI at 80×24 and 120×40 for regression safety.
- Two new crates: `rtcom-config` (profile persistence) and `rtcom-tui`
  (ratatui UI layer).
- New ADRs: [`008-ratatui-tui`](./docs/adr/008-ratatui-tui.md),
  [`009-vt100-emulator`](./docs/adr/009-vt100-emulator.md),
  [`010-directories-xdg`](./docs/adr/010-directories-xdg.md).
- Scrollback navigation in the serial pane via `Shift+PageUp/Down`,
  `Shift+Up/Down`, `Shift+Home/End`, and the mouse wheel. Top bar
  shows `[SCROLL ↑N]` (yellow) when the view is above the live tail.
- New profile key `[screen].wheel_scroll_lines` (default `3`) tunes
  the mouse wheel scroll speed. Hand-edit the TOML to change; a
  menu-editable control lands in v0.2.1.

### Changed

- **BREAKING**: rtcom now requires a real TTY on stdin/stdout; piping
  through a non-TTY process no longer works.
- **BREAKING**: the v0.1 stdout line-by-line renderer is removed.
- `rtcom-cli` no longer owns the terminal lifecycle — delegated to
  `rtcom-tui`.
- `crossterm` bumped from 0.27 to 0.28 (ratatui transitive unification).
- Bottom-bar label corrected from `^A q quit` to `^A ^Q quit` — the
  actual binding is Ctrl-Q (or Ctrl-X), not the plain letter `q`.
- Serial port setup dialog now surfaces a hint line when any CLI flag
  (`-b`, `-d`, `-s`, `-p`, `-f`, `--omap`/`--imap`/`--emap`) is
  overriding the loaded profile value. Clarifies the defaults <
  profile < CLI merge priority that tripped users during smoke testing.
- Line endings dialog now shows inline recipes ("imap = crlf for
  \\n-only devices" etc.) so users don't have to consult the manual
  to translate the minicom rule names.
- `docs/tui.md` gains a "Line endings recipes" section with
  symptom→cure guidance for common device behaviors.

### Deprecated

- None for this release. A future v0.2.1 may add a `^A q` alias so
  typing the plain letter works too; the current fix is label-only.

### Fixed

- Partial section parsing in profile files now falls back to section
  defaults instead of erroring out.
- Line-ending changes applied via the menu's `F10` (Apply + Save) now
  persist to the profile file. Previously both Live and Save paths were
  blanket-deferred; only Live actually required the runtime-mapper
  refactor (v0.2.1).

### Deferred to v0.2.1 / later

- Live line-ending changes (currently requires restart).
- Real-time modem status display (CTS/DSR/RI/CD polling).
- Mouse-driven text selection + copy in the serial pane. For v0.2,
  hold `Shift` while clicking and dragging — most terminal emulators
  treat `Shift+drag` as a bypass of rtcom's mouse capture, letting
  the terminal's native selection + copy work.
- Menu-editable `[screen].wheel_scroll_lines` (hand-edit TOML for now).
- Multi-named-profile support (`--profile <name>`).

## [0.1.2] — 2026-04-17

### Added

- `--lower-dtr` / `--raise-dtr` / `--lower-rts` / `--raise-rts`
  CLI flags mirroring picocom 1:1. Each lower/raise pair is mutually
  exclusive at the clap level. The deassert / assert is applied to
  the device immediately after `open()` and before `Session` takes
  ownership, and the resulting line state is fed to
  `Session::with_initial_dtr` / `Session::with_initial_rts` so the
  cached state stays honest and the first `^A t` / `^A g` toggle
  produces the right transition. Closes [#1].

### Fixed

- `rtcom -V` now embeds the git commit hash for `cargo install`
  builds too, not just local `cargo install --path` checkouts.
  v0.1.1 from crates.io showed `rtcom 0.1.1` (no hash) because
  `build.rs`'s `git rev-parse` had no `.git` to read. The release
  workflow now writes `crates/rtcom-cli/.commit-hash` before
  `cargo publish`; `build.rs` falls back to that file when git
  is unavailable. Tarball builds therefore show
  `rtcom 0.1.2 (abc12345)`.

[#1]: https://github.com/TrekMax/rtcom/issues/1

## [0.1.1] — 2026-04-17

A "make the v0.1 release actually publishable" patch. Binary
behaviour is the same as the late-v0.1.0 development binary that was
used during the first hardware smoke test; this release pins those
changes to a properly-tagged version that flows through the new
GitHub release + crates.io publish pipeline.

### Infrastructure

- Reworked CI: `fmt` / `clippy` (3-OS matrix) / `test` (3-OS matrix
  with libudev + socat on Linux for the e2e PTY suite) / `doc`
  (`-D warnings`) jobs split for fast feedback. Swatinem/rust-cache
  per-key.
- New `release.yml` workflow on `v*` tag push:
  - 5-target build matrix (Linux x86_64 + aarch64-cross, macOS
    x86_64 + aarch64, Windows x86_64). Cross-compile uses
    [`cross`](https://github.com/cross-rs/cross) with a
    `Cross.toml` that installs `libudev-dev:arm64` for the aarch64
    target.
  - macOS universal binary via `lipo`.
  - GitHub Release page with auto-generated notes (CHANGELOG
    section preferred, commit-log fallback) and SHA-256 checksums.
  - `cargo publish -p rtcom-core` then `-p rtcom-cli` (with a 45 s
    sleep between for index propagation), gated on the release
    job succeeding.

### Added

- `-V` / `--version` now embeds the short git commit hash (and a
  `-dirty` marker when the working tree has uncommitted changes):
  `rtcom 0.1.0 (5a103b2a)` for clean checkouts,
  `rtcom 0.1.0 (5a103b2a-dirty)` otherwise. Falls back to the bare
  `rtcom 0.1.0` for crates.io tarball builds where git is not
  available.
- Lifecycle banner prints between the config summary and the
  interactive session (`Terminal ready`) and again on shutdown
  (`Terminating...` / `Thanks for using rtcom`). Suppressed by
  `--quiet`. Mirrors picocom's "Terminal ready" / "Terminating..."
  affordance so users can tell at a glance whether rtcom is up,
  in-session, or shutting down.

### Changed

- Default command-key escape switched from `^T` (Ctrl-T) to `^A`
  (Ctrl-A). Picocom's historical default; survives tmux's prefix
  binding and terminal emulators that use Ctrl-T for "new tab".
  Override with `--escape '^T'` to restore the previous behaviour.
- Quit command keys are now `^Q` (Ctrl-Q, 0x11) and `^X` (Ctrl-X,
  0x18) instead of the plain letters `q` / `x`. Mirrors picocom and
  frees the letters to be sent to the wire as data without an extra
  escape dance. Type the escape key followed by Ctrl-Q or Ctrl-X to
  exit; plain `q` / `x` after the escape now fall into the
  unknown-command silently-drop path.

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

[Unreleased]: https://github.com/TrekMax/rtcom/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/TrekMax/rtcom/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/TrekMax/rtcom/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/TrekMax/rtcom/releases/tag/v0.1.2
[0.1.1]: https://github.com/TrekMax/rtcom/releases/tag/v0.1.1
[0.1.0]: https://github.com/TrekMax/rtcom/releases/tag/v0.1.0

[GHSA-rhfx-m35p-ff5j]: https://github.com/advisories/GHSA-rhfx-m35p-ff5j
[GHSA-r6v5-fh4h-64xc]: https://github.com/advisories/GHSA-r6v5-fh4h-64xc
[GHSA-cq8v-f236-94qc]: https://github.com/advisories/GHSA-cq8v-f236-94qc
