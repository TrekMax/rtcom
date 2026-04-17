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

## Features (v0.1)

- Async serial I/O built on `tokio-serial` (Linux / macOS / BSD).
- UUCP-style lock files so two `rtcom` instances cannot race the same
  device.
- Interactive command key (`^A` by default, configurable via
  `--escape`) with:
  `?`/`h` help, `q`/`x` quit, `c` show config, `t`/`g` toggle DTR/RTS,
  `\` send break, `b<rate><Enter>` change baud rate.
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

# Print the full option list
rtcom --help
```

## Installation

### From crates.io

```bash
cargo install rtcom-cli --locked
```

`--locked` pins the dependency versions shipped in `Cargo.lock`, which
keeps the MSRV contract (Rust 1.85) honest.

### From source

```bash
git clone https://github.com/TrekMax/rtcom
cd rtcom
cargo install --path crates/rtcom-cli --locked
```

On Linux you'll need `libudev-dev` (or the equivalent package on your
distro) for the underlying `serialport` crate.

### Distribution packages

Homebrew tap, AUR, and winget packages are planned for v0.2+. In the
meantime, `cargo install` is the canonical path.

## Command keys

Once a session is running, the escape key (default `^A` = Ctrl-A;
override with `--escape '^T'` etc.) puts the parser into command
mode. The next byte is matched against this table; unknown keys
silently return to default mode.

| Key | Action |
|-----|--------|
| `?` or `h` | Print the command-key cheatsheet |
| `q` or `x` | Quit the session cleanly |
| `c` | Show current serial configuration |
| `t` | Toggle DTR |
| `g` | Toggle RTS |
| `\` | Send a 250 ms line break |
| `b<rate><Enter>` | Change baud rate (e.g. `^A b 115200 <Enter>`) |
| The escape key again | Send the escape byte verbatim to the wire |
| `Esc` | Cancel command mode, return to default |

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
| `rtcom-core` | Serial device trait, event bus, session orchestrator, mappers, UUCP lock (library) |
| `rtcom-cli`  | `rtcom` binary: argument parsing, TTY setup, signal handling, main loop |

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
