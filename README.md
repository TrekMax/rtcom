# rtcom

**Rust Terminal Communication** — a modern, safe, cross-platform serial
terminal for embedded and hardware engineers. Written in Rust, aiming for
feature parity with [tio](https://github.com/tio/tio) while adding a native
Windows backend, first-class library API, and a pluggable architecture for
future protocol decoding, scripting, and network sharing.

> **Status:** pre-alpha. Workspace skeleton only (v0.1 Issue #1 — see
> [`CLAUDE.md`](./CLAUDE.md)). Not usable yet.

## Workspace layout

| Crate | Role |
|---|---|
| `rtcom-core` | Serial device trait, event bus, session orchestrator (library) |
| `rtcom-cli`  | `rtcom` binary: argument parsing, TTY setup, entry point |

## Building from source

```bash
cargo build --workspace
cargo run -p rtcom-cli
```

Requires Rust 1.85+ (pinned via `rust-toolchain.toml`). On Linux you'll need
`libudev-dev` for the `serialport` backend.

## License

Apache-2.0. See [`LICENSE`](./LICENSE).
