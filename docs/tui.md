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
   the device. Supports ANSI styling, cursor positioning, and a
   10,000-row scrollback buffer (see [Scrollback and
   selection](#scrollback-and-selection)).
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

## Scrollback and selection

The serial pane keeps a 10,000-row scrollback buffer. Navigate with:

### Keyboard

| Keystroke           | Action                               |
| ------------------- | ------------------------------------ |
| `Shift+PageUp`      | Scroll up half a screen              |
| `Shift+PageDown`    | Scroll down half a screen            |
| `Shift+Up`          | Scroll up one line                   |
| `Shift+Down`        | Scroll down one line                 |
| `Shift+Home`        | Jump to oldest row                   |
| `Shift+End`         | Jump back to live tail               |

### Mouse

The mouse wheel scrolls the serial pane — 3 lines per notch by
default. Override via `[screen].wheel_scroll_lines` in the profile
(hand-edit the TOML; a menu-editable control lands in v0.2.1).
Values less than 1 are clamped to 1 at runtime, so the wheel
always moves at least one line.

### Top-bar indicator

When the view is above the live tail, the top bar shows
`[SCROLL ↑N]` (yellow) with N lines above live. New data keeps
streaming into the buffer, but the view does not follow until you
`Shift+End` (or scroll back down past the bottom).

### Selection and copy

Native mouse-driven text selection + copy lands in v0.2.1. For
v0.2:

- **To copy visible text**: hold `Shift` while clicking and
  dragging. Most terminals (xterm, gnome-terminal, iterm2, kitty,
  alacritty, `Windows Terminal`) treat `Shift+drag` as a bypass of
  rtcom's mouse capture, letting the terminal's native selection +
  copy work.
- **To copy older scrollback content**: not yet supported
  directly. Scroll up with `Shift+PageUp` first to bring the
  target lines on-screen, then `Shift+drag` once they are visible.

## Line endings recipes

rtcom's line-ending mappers inherit the minicom / picocom vocabulary,
which is compact but non-obvious. Here is what each rule does and when
to use it.

### The rules (all dispatch on the INPUT byte)

| Rule    | Trigger       | Effect                      |
| ------- | ------------- | --------------------------- |
| `none`  | —             | pass through unchanged      |
| `crlf`  | `\n` in input | prepend `\r` → CRLF         |
| `lfcr`  | `\r` in input | append `\n` → CRLF          |
| `igncr` | `\r` in input | drop the `\r`               |
| `ignlf` | `\n` in input | drop the `\n`               |

Both `crlf` and `lfcr` produce CRLF — the difference is **which byte
triggers them**. If you pick a rule that doesn't match what's in the
stream, it does nothing. This trips most users at least once.

### Recipes by direction

Set these in the Line endings dialog (`^A m → Line endings`) or by
hand in `~/.config/rtcom/default.toml`.

#### Receiving from the device (`imap`)

Pick the rule that matches what your device emits:

| Device behavior                                          | `imap`  |
| -------------------------------------------------------- | ------- |
| sends `\n` only (most MCUs, Zephyr, Linux-hosted apps)   | `crlf`  |
| sends `\r` only (old Mac, some DOS tools)                | `lfcr`  |
| sends `\r\n` (Windows, standard serial)                  | `none`  |
| spams both and you want clean lines                      | `igncr` |

**Symptom → cure**:

- **Staircase output** (each line indents further right):
  your device sends `\n` only → set `imap = crlf`.
- **No newlines at all**, text overwrites itself on one row:
  device sends `\r` only → set `imap = lfcr`.
- **Double-spaced output** (blank row between every line):
  device sends `\r\n` and rtcom is also doubling it → set `imap = none`
  (or `igncr` if you want to keep only `\n`).

#### Sending to the device (`omap`)

Pick what your device **expects**:

| Device expects                                                     | `omap` |
| ------------------------------------------------------------------ | ------ |
| line terminated by `\r\n` (most firmware REPLs, AT-command modems) | `crlf` |
| line terminated by `\r` (old Mac, some serial consoles)            | `lfcr` |
| line terminated by `\n` only                                       | `none` |

**Symptom → cure**:

- **Commands don't execute** after pressing Enter:
  device needs CR → try `omap = crlf` (or `lfcr` if it wants CR only).
- **Every command runs twice**:
  you're sending `\r\n` to a device that splits it → set `omap = none`
  and let the device's CR-on-enter mode handle it.

#### Echo (`emap`)

rtcom v0.2 has no local-echo rendering yet — `emap` is persisted to the
profile but doesn't affect display. It lands in v0.3's logging module.

### Trying it

After editing the dialog, press `F10` (Apply + Save). Because runtime
mapper swap is deferred to v0.2.1, the new rule applies on the next
`rtcom` invocation — **exit** (`^A ^Q`) **and relaunch** to see the
effect.

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
