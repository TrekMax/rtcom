# rtcom

[![CI](https://github.com/TrekMax/rtcom/actions/workflows/ci.yml/badge.svg)](https://github.com/TrekMax/rtcom/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/rtcom-cli.svg)](https://crates.io/crates/rtcom-cli)
[![docs.rs](https://img.shields.io/docsrs/rtcom-core)](https://docs.rs/rtcom-core)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)

**Rust Terminal Communication** — a modern, safe, cross-platform serial
terminal for embedded and hardware engineers. Written in Rust, aiming for
feature parity with [tio](https://github.com/tio/tio) while adding a
native Windows backend, a first-class library API, and a pluggable
architecture for future protocol decoding, scripting, and network
sharing.

## What's new in v0.2 (Preview)

rtcom v0.2 switches from the v0.1 line-based stdout renderer to a full
[ratatui](https://github.com/ratatui-org/ratatui)-backed TUI:

- **Configuration menu** at `^A m` — minicom-style dialog tree for serial
  settings, line-endings, modem lines, profile save/load, and screen
  options.
- **Profile persistence** via `~/.config/rtcom/default.toml`
  (XDG standard). `rtcom -c <path>` to override; `rtcom ... --save` to
  write the effective configuration on startup.
- **Apply live vs. save** — every configuration dialog distinguishes
  `F2` (apply to the live session) from `F10` (apply + persist to profile),
  matching the minicom UX.
- **Proper VT100 emulation** in the serial pane — remote apps that use
  cursor positioning (ncurses UIs, Zephyr shell, etc.) render correctly.

<!-- TODO: add screenshot of the menu / serial pane -->

**Breaking change**: v0.2 requires a proper TTY. Piping `rtcom` output
through a non-TTY consumer (`rtcom /dev/ttyUSB0 | grep ...`) no longer
works. Use `tio`'s capture feature or `rtcom --log` (planned for v0.3)
instead.

See [`docs/tui.md`](./docs/tui.md) for the full keybinding reference.

## Features (v0.1)

- Async serial I/O built on `tokio-serial` (Linux / macOS / BSD).
- UUCP-style lock files so two `rtcom` instances cannot race the same
  device.
- Interactive command key (`^A` by default, configurable via
  `--escape`) with:
  `?`/`h` help, `^Q`/`^X` quit (picocom convention), `c` show config,
  `t`/`g` toggle DTR/RTS, `\` send break, `b<rate><Enter>` change
  baud rate.
- Picocom-style startup-time modem-line control:
  `--lower-dtr` / `--raise-dtr` / `--lower-rts` / `--raise-rts`.
  The classic "open the port without resetting the Arduino-style
  MCU" recipe is `--lower-dtr --lower-rts`.
- CR/LF mappers (`--omap` / `--imap` / `--emap`) following picocom's
  `crlf` / `lfcr` / `igncr` / `ignlf` rule names.
- Clean shutdown on SIGINT / SIGTERM / SIGHUP (termios restored,
  lock file removed, exit code `128 + signum`).
- Structured diagnostics via `tracing` (honours `RUST_LOG`; `-v` /
  `-vv` / `-vvv` raises the default level).
- End-to-end tests via `socat`-allocated pseudo-terminals — the full
  pipeline is regression-guarded.

## Quick start

```bash
# Connect to a USB-serial dongle at 115200 8N1 (default)
rtcom /dev/ttyUSB0

# Change baud + parity + enable LF->CRLF on send
rtcom /dev/ttyUSB0 -b 9600 -p even --omap crlf

# Load a custom profile and persist CLI overrides back to it
rtcom /dev/ttyUSB0 -b 921600 -c ~/.config/rtcom/board-x.toml --save

# Print the full option list
rtcom --help
```

Press `^A m` to open the configuration menu, `^A ^Q` (or `^A ^X`) to
quit. See [`docs/tui.md`](./docs/tui.md) for the full keybinding
reference.

## Installation

### From crates.io

```bash
cargo install rtcom-cli --locked
```

`--locked` pins the dependency versions shipped in `Cargo.lock`, which
keeps the MSRV contract (Rust 1.86) honest.

### From source

```bash
git clone https://github.com/TrekMax/rtcom
cd rtcom
cargo install --path crates/rtcom-cli --locked
```

On Linux you'll need `libudev-dev` (or the equivalent package on your
distro) for the underlying `serialport` crate.

### Pre-built binaries

Each tagged release publishes pre-built binaries for the major
platforms — grab one from
[GitHub Releases](https://github.com/TrekMax/rtcom/releases):

| Platform | Architecture | Asset |
|----------|--------------|-------|
| Linux | x86_64 | `rtcom-x86_64-unknown-linux-gnu` |
| Linux | aarch64 | `rtcom-aarch64-unknown-linux-gnu` |
| macOS | Intel | `rtcom-x86_64-apple-darwin` |
| macOS | Apple Silicon | `rtcom-aarch64-apple-darwin` |
| macOS | Universal | `rtcom-universal-apple-darwin` |
| Windows | x86_64 | `rtcom-x86_64-pc-windows-msvc.exe` |

Each release also ships a `checksums-sha256.txt` so you can verify the
download.

### Distribution packages

Homebrew tap, AUR, and winget packages are planned for v0.2+. In the
meantime, `cargo install` or the GitHub Releases tarball is the
canonical path.

## Command keys

Once a session is running, the escape key (default `^A` = Ctrl-A;
override with `--escape '^T'` etc.) puts the parser into command
mode. The next byte is matched against this table; unknown keys
silently return to default mode.

| Key | Action |
|-----|--------|
| `m` | Open the configuration menu (v0.2) |
| `?` or `h` | Print the command-key cheatsheet |
| `^Q` or `^X` | Quit the session cleanly (Ctrl-Q / Ctrl-X) |
| `c` | Show current serial configuration |
| `t` | Toggle DTR |
| `g` | Toggle RTS |
| `\` | Send a 250 ms line break |
| `b<rate><Enter>` | Change baud rate (e.g. `^A b 115200 <Enter>`) |
| The escape key again | Send the escape byte verbatim to the wire |
| `Esc` | Cancel command mode, return to default |

### Menu navigation (v0.2)

Inside a dialog opened via `^A m`:

| Keystroke | Action |
|-----------|--------|
| `↑` / `↓` / `j` / `k` | Move cursor |
| `Enter` | Activate / edit / confirm |
| `Space` | Cycle enum values |
| `+` / `-` | Step through common baud rates |
| `F2` | Apply pending changes to the live session |
| `F10` | Apply + save to the profile TOML |
| `Esc` | Cancel / close dialog |

See [`docs/tui.md`](./docs/tui.md) for the full TUI reference.

## vs. picocom / tio

| Capability | picocom | tio | rtcom (v0.1) |
|---|:-:|:-:|:-:|
| Async I/O (`tokio`) | ❌ | ❌ | ✅ |
| UUCP lock files | ✅ | ✅ | ✅ |
| Raw-mode cleanup on any exit path | ⚠️ | ⚠️ | ✅ |
| CR/LF omap/imap/emap | ✅ | ❌ | ✅ |
| `^A <key>` command parser | ✅ | ✅ | ✅ |
| Library API (`rtcom-core`) | ❌ | ❌ | ✅ |
| Native Windows backend | ❌ | ❌ | 🚧 v0.8 |
| Structured logging / `tracing` | ❌ | ❌ | ✅ |
| Built-in xmodem / ymodem | ❌ | ❌ | 🚧 v0.6 |

🚧 = on the roadmap.

## Workspace layout

| Crate | Role |
|---|---|
| `rtcom-core`   | Serial device trait, event bus, session orchestrator, mappers, UUCP lock (library) |
| `rtcom-config` | Profile TOML (serde) + XDG / platform-native path resolution |
| `rtcom-tui`    | ratatui-backed UI: serial pane (vt100), menu, dialogs, toasts |
| `rtcom-cli`    | `rtcom` binary: argument parsing, TTY setup, signal handling, main loop |

## Architecture

Event-driven, single-task [`Session`](./crates/rtcom-core/src/session.rs)
wired through a `tokio::broadcast` bus. See
[`docs/architecture.svg`](./docs/architecture.svg) and the
[`CLAUDE.md`](./CLAUDE.md) design document for the full rationale.

## Contributing

The development plan is maintained in [`CLAUDE.md`](./CLAUDE.md);
§8 covers the TDD + commit-per-phase workflow the project enforces,
§9 the coding conventions. Bug reports and PRs welcome — please open
an issue to discuss larger changes before sending code.

Running the full test suite locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## License

Apache-2.0. See [`LICENSE`](./LICENSE).
