# rtcom TUI reference

## Launching

`rtcom /dev/ttyUSB0 -b 115200` opens the device and enters the TUI.
`rtcom` uses the alternate screen so the scrollback you had before
the invocation is preserved on exit.

## Screen layout

The main screen has three horizontal bands:

1. **Top bar** — rtcom version, device path, and current serial config
   (baud, framing, flow control).
2. **Serial pane** — a VT100 emulator that renders bytes received from
   the device. Supports ANSI styling, cursor positioning, and will host
   scrollback + copy/paste in a future release.
3. **Bottom bar** — quick-key hints (`^A m menu · ^A ? help · ^A ^Q quit`).

## Keyboard shortcuts

### Quick (escape-char prefixed)

All commands are prefixed with `^A` (Ctrl-A). The escape character is
configurable via `--escape`.

| Keystroke   | Action                              |
| ----------- | ----------------------------------- |
| `^A m`      | Open the configuration menu         |
| `^A ?`      | Show the command cheat sheet        |
| `^A ^Q`     | Quit rtcom                          |
| `^A ^X`     | Quit rtcom (picocom compatibility)  |
| `^A b`      | Change baud rate (inline prompt)    |
| `^A c`      | Show current config in the pane     |
| `^A t`      | Toggle DTR                          |
| `^A g`      | Toggle RTS                          |
| `^A \`      | Send a 250ms break                  |

### Menu navigation

Inside dialogs:

| Keystroke          | Action                                |
| ------------------ | ------------------------------------- |
| `↑` / `↓`          | Move cursor                           |
| `j` / `k`          | Vi-style cursor movement              |
| `Enter`            | Activate / edit / confirm             |
| `Space`            | Cycle enum values                     |
| `+` / `-`          | Step through common baud rates        |
| `F2`               | Apply pending changes to live session |
| `F10`              | Apply + save to profile               |
| `Esc`              | Cancel / close dialog                 |

## Modal styles

The screen-options dialog (within `^A m`) toggles how overlays render:

| Style              | Description                                            |
| ------------------ | ------------------------------------------------------ |
| `overlay`          | Modal centered; background pane keeps drawing as-is    |
| `dimmed-overlay`   | Modal centered; background pane is dimmed              |
| `fullscreen`       | Modal fills the body; background pane is hidden        |

The choice persists to the profile's `[screen].modal_style` key.

## Profile

The default profile lives at:

| Platform | Path                                                       |
| -------- | ---------------------------------------------------------- |
| Linux    | `$XDG_CONFIG_HOME/rtcom/default.toml` (`~/.config/rtcom/`) |
| macOS    | `~/Library/Application Support/rtcom/default.toml`         |
| Windows  | `%APPDATA%\rtcom\default.toml`                             |

Override with `-c PATH`. On first run the file doesn't exist; rtcom
uses built-in defaults and creates the file when you save from the
menu or pass `--save` on the command line.

**Tip**: hand-edit the TOML if you prefer — unknown keys are silently
ignored, missing leaf values fall back to the section default. Saving
from the menu will rewrite the file and lose any hand-written comments.

## Config merge priority

rtcom merges three sources in this fixed order:

```
built-in defaults  ─┐
profile file        ─┼─▶  effective runtime
CLI arguments       ─┘
```

**CLI arguments always win**. If you edit the baud rate in the menu and
press `F10` (Apply + Save), the profile gets the new baud — but the
next time you launch with `rtcom -b 115200 /dev/ttyXXX`, the CLI's
115200 overrides your saved value for that session.

Two ways to make the profile value effective:

1. **Drop the CLI flag**: `rtcom /dev/ttyXXX` reads the baud from the profile.
2. **Force a write**: `rtcom -b 921600 /dev/ttyXXX --save` rewrites
   the profile with 921600 and uses 921600 for the current session.

The dialog's bottom hint line (`* N field(s) overridden by CLI; ...`)
shows up when any CLI flag is overriding a profile value in the
current session.

## Related documentation

- [`CLAUDE.md`](../CLAUDE.md) — roadmap and architecture
- [`docs/adr/`](./adr/) — architectural decisions (ratatui, vt100,
  directories)
- `man rtcom` — offline reference
