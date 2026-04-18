# rtcom v0.2 — TUI 菜单实现计划

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 把 rtcom 从 stdout 行式渲染升级为 ratatui 全屏 TUI,加入 `^A m` 配置菜单和最小 profile 读写,完成新 v0.2 交付范围。

**Architecture:** 新增 `rtcom-config` 和 `rtcom-tui` 两个 crate;`rtcom-core` 扩 Event / 加 `Session::apply_config`;`rtcom-cli` 删除 `terminal.rs`+`stdin.rs`,改成 TuiApp 启动器。SerialPane 基于 `vt100` crate 做 2D cell grid,`tui-term` 适配到 ratatui widget。CLI × Profile × Menu 三源按 `defaults < profile < CLI` 合并,`--save` 显式回写。

**Tech Stack:** Rust 2021 / MSRV 1.85,tokio,ratatui 0.26+,vt100,tui-term,directories,crossterm,clap,serde+toml,insta(dev)。

**前置条件:**
- 已完成设计评审:`docs/plans/2026-04-18-tui-menu-design.md`(commit `16506cd`)
- 实施在独立分支:`feat/v0.2-tui-menu`(或拆成多个 `feat/v0.2-#N-xxx` 逐 PR)
- 每个 Task 结束时必跑:`cargo fmt && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features`
- 遵循 CLAUDE.md §8 TDD 工作流:红 → 绿 → 重构,**每完成一个循环立即 commit**

**阶段概览:**

| 阶段 | Tasks | 说明 |
|---|---|---|
| Phase 1:rtcom-config 底座 | 1–3 | 最小 profile + CLI 钩子,与 TUI 解耦可独立验证 |
| Phase 2:rtcom-core 扩展 | 4–5 | Event 新 variant + Session.apply_config |
| Phase 3:rtcom-tui 骨架 + SerialPane | 6–9 | TuiApp 渲染循环、alternate screen、vt100 集成 |
| Phase 4:菜单 + 四个 dialog | 10–15 | ModalStack、顶层菜单、Serial/LineEndings/Modem/Screen/Profile dialogs |
| Phase 5:集成与 apply 流 | 16–19 | rtcom-cli 瘦身、Apply live、Apply+Save、toast |
| Phase 6:测试与发布 | 20–23 | snapshot、e2e、文档、v0.2.0 tag |

---

## Phase 1 — rtcom-config 底座

### Task 1:创建 `rtcom-config` crate 骨架 + `Profile` 结构

**Files:**
- Create: `crates/rtcom-config/Cargo.toml`
- Create: `crates/rtcom-config/src/lib.rs`
- Create: `crates/rtcom-config/src/profile.rs`
- Modify: `Cargo.toml`(根 workspace,加 members + `rtcom-config` 到 workspace.dependencies)
- Modify: `Cargo.toml`(根 workspace.dependencies,加 `toml`, `serde`, `directories`)

**Step 1:写失败测试**

`crates/rtcom-config/src/profile.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_default_values() {
        let p = Profile::default();
        assert_eq!(p.serial.baud, 115200);
        assert_eq!(p.serial.data_bits, 8);
        assert_eq!(p.screen.modal_style, ModalStyle::Overlay);
        assert_eq!(p.screen.scrollback_rows, 10_000);
    }

    #[test]
    fn profile_roundtrip_toml() {
        let original = Profile::default();
        let serialized = toml::to_string(&original).expect("serialize");
        let parsed: Profile = toml::from_str(&serialized).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn profile_unknown_keys_are_dropped() {
        let with_unknown = r#"
            [serial]
            baud = 9600
            unknown_field = "ignored"
        "#;
        let parsed: Profile = toml::from_str(with_unknown).expect("parse");
        assert_eq!(parsed.serial.baud, 9600);
    }
}
```

**Step 2:Run test — verify fail**

```bash
cargo test -p rtcom-config
```

Expected:编译失败(`Profile` / `ModalStyle` 未定义)。

**Step 3:实现最小 `Profile`**

```rust
// crates/rtcom-config/src/profile.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub serial: SerialSection,
    #[serde(default)]
    pub line_endings: LineEndingsSection,
    #[serde(default)]
    pub modem: ModemSection,
    #[serde(default)]
    pub screen: ScreenSection,
}

impl Default for Profile {
    fn default() -> Self { /* all sections default */ }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialSection {
    pub baud: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: String,   // use enum later; keep String for v0.2 Task 1
    pub flow: String,
}
// ... LineEndingsSection, ModemSection, ScreenSection

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModalStyle { Overlay, DimmedOverlay, Fullscreen }
```

`crates/rtcom-config/src/lib.rs`:
```rust
//! Profile persistence for rtcom.
#![forbid(unsafe_code)]
pub mod profile;
pub use profile::{Profile, ModalStyle};
```

**Step 4:Run test — verify pass**

```bash
cargo test -p rtcom-config
```

Expected:3 tests pass。

**Step 5:Commit**

```bash
git add crates/rtcom-config Cargo.toml
git commit -m "feat(config): Profile struct + TOML roundtrip (v0.2 task 1)"
```

---

### Task 2:XDG 路径 + `read_default` / `write_default`

**Files:**
- Create: `crates/rtcom-config/src/paths.rs`
- Modify: `crates/rtcom-config/src/lib.rs`
- Create: `crates/rtcom-config/tests/io.rs`

**Step 1:写失败测试**

`crates/rtcom-config/tests/io.rs`:

```rust
use rtcom_config::{read, write, Profile};
use tempfile::tempdir;

#[test]
fn write_then_read_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("default.toml");
    let mut p = Profile::default();
    p.serial.baud = 9600;

    write(&path, &p).expect("write");
    let loaded = read(&path).expect("read");
    assert_eq!(loaded.serial.baud, 9600);
}

#[test]
fn read_missing_returns_default() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("absent.toml");
    // Explicit contract: caller decides whether missing = default;
    // read() itself returns Err(NotFound). Thin wrapper in integration tests.
    assert!(read(&path).is_err());
}

#[test]
fn read_malformed_returns_parse_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "this is not valid = = toml").unwrap();
    let err = read(&path).unwrap_err();
    assert!(matches!(err, rtcom_config::Error::Parse(_)));
}
```

**Step 2:Run — fail**
```bash
cargo test -p rtcom-config --test io
```

**Step 3:实现**

`crates/rtcom-config/src/lib.rs` 新增:

```rust
pub mod paths;

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")] Io(#[from] std::io::Error),
    #[error("parse error: {0}")] Parse(#[from] toml::de::Error),
    #[error("serialize error: {0}")] Serialize(#[from] toml::ser::Error),
}

pub fn read(path: &Path) -> Result<Profile, Error> {
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

pub fn write(path: &Path, profile: &Profile) -> Result<(), Error> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let text = toml::to_string_pretty(profile)?;
    std::fs::write(path, text)?;
    Ok(())
}
```

`crates/rtcom-config/src/paths.rs`:

```rust
use directories::ProjectDirs;
use std::path::PathBuf;

pub fn default_profile_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rtcom")
        .map(|dirs| dirs.config_dir().join("default.toml"))
}
```

依赖:在根 `Cargo.toml` workspace.dependencies 加 `directories = "5"`,`tempfile` 已在(v0.1 有)。

**Step 4:pass**

**Step 5:Commit**
```bash
git commit -m "feat(config): read/write + XDG paths (v0.2 task 2)"
```

---

### Task 3:CLI `-c PATH` 覆盖 + `--save` 开关

**Files:**
- Modify: `crates/rtcom-cli/src/args.rs`(加 `-c`, `--save`)
- Modify: `crates/rtcom-cli/Cargo.toml`(加 `rtcom-config` 依赖)
- Modify: `crates/rtcom-cli/src/main.rs`(加载 profile → merge CLI args → 可选 `--save` 回写)

**Step 1:写失败测试(在 args.rs tests 或新增 tests/cli_config.rs)**

```rust
#[test]
fn cli_accepts_config_path() {
    let args = Cli::try_parse_from([
        "rtcom", "/dev/ttyUSB0", "-c", "/tmp/alt.toml"
    ]).unwrap();
    assert_eq!(args.config.as_deref(), Some(Path::new("/tmp/alt.toml")));
}

#[test]
fn cli_save_flag_requires_device() {
    // Plain `rtcom --save` with no device is an error
    let err = Cli::try_parse_from(["rtcom", "--save"]).unwrap_err();
    // clap rejects because positional `device` is required
    assert!(err.to_string().contains("device"));
}

#[test]
fn cli_save_with_device_parses() {
    let args = Cli::try_parse_from([
        "rtcom", "/dev/ttyUSB0", "-b", "9600", "--save"
    ]).unwrap();
    assert!(args.save);
    assert_eq!(args.baud, Some(9600));
}
```

**Step 2:fail** → **Step 3:实现**

```rust
// args.rs additions
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct Cli {
    // ... existing fields ...

    /// Override profile path (default: XDG config)
    #[arg(short = 'c', long = "config", value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Save effective config to profile on startup
    #[arg(long = "save")]
    pub save: bool,
}
```

`main.rs` 改:

```rust
// pseudocode; adapt to existing main.rs flow
let cli = Cli::parse();

let profile_path = cli.config.clone()
    .or_else(rtcom_config::paths::default_profile_path);

let profile = match profile_path.as_ref().and_then(|p| rtcom_config::read(p).ok()) {
    Some(p) => p,
    None => Profile::default(),
};

// Merge: profile → CLI overrides → effective config
let serial_cfg = merge_cli_over_profile(&profile, &cli);

if cli.save {
    if let Some(path) = profile_path.as_ref() {
        let mut updated = profile.clone();
        updated.serial = to_profile_serial(&serial_cfg);
        if let Err(e) = rtcom_config::write(path, &updated) {
            eprintln!("rtcom: --save failed: {e}");
            std::process::exit(1);
        }
    }
}
// ... continue to Session::new(serial_cfg, ...)
```

`merge_cli_over_profile` 作为私有 fn 在 `main.rs` 或新 `config_merge.rs`。

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(cli): -c PATH + --save flags (v0.2 task 3)"
```

---

## Phase 2 — rtcom-core 扩展

### Task 4:Event enum 新 variant + `Command::OpenMenu`

**Files:**
- Modify: `crates/rtcom-core/src/event.rs`
- Modify: `crates/rtcom-core/src/command.rs`

**Step 1:写失败测试**

`command.rs` 的 tests 模块:

```rust
#[test]
fn command_parser_recognizes_open_menu() {
    let mut parser = CommandKeyParser::new(Escape::CtrlA);
    // ^A then 'm'
    assert!(matches!(parser.feed(0x01), ParseOutcome::AwaitingCommand));
    assert!(matches!(parser.feed(b'm'), ParseOutcome::Command(Command::OpenMenu)));
}
```

`event.rs` 的 tests:

```rust
#[test]
fn event_menu_opened_closed_are_clone() {
    let e = Event::MenuOpened;
    let _ = e.clone();
    let e = Event::MenuClosed;
    let _ = e.clone();
}
```

**Step 2:fail** → **Step 3:实现**

```rust
// event.rs
pub enum Event {
    // ... existing variants ...
    MenuOpened,
    MenuClosed,
    ProfileSaved { path: PathBuf },
    ProfileLoadFailed { path: PathBuf, error: Arc<Error> },
}

// command.rs
pub enum Command {
    // ... existing ...
    OpenMenu,  // new, triggered by ^A m
}

// command parser table:
// ... existing key → Command mapping ...
b'm' => Command::OpenMenu,
```

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(core): Event::MenuOpened/Closed/ProfileSaved/LoadFailed + ^A m (v0.2 task 4)"
```

---

### Task 5:`Session::apply_config` + snapshot 回滚

**Files:**
- Modify: `crates/rtcom-core/src/session.rs`
- Modify: `crates/rtcom-core/src/config.rs`(如需配套 helper)

**Step 1:写失败测试(用 mock device)**

```rust
#[tokio::test]
async fn apply_config_success_publishes_changed_event() {
    let (mut session, mut bus_rx) = new_session_with_mock(initial_cfg()).await;
    let new_cfg = SerialConfig { baud: 9600, ..initial_cfg() };
    session.apply_config(new_cfg.clone()).await.unwrap();
    let ev = bus_rx.recv().await.unwrap();
    assert_eq!(ev, Event::ConfigChanged(new_cfg));
}

#[tokio::test]
async fn apply_config_rolls_back_on_middle_failure() {
    // mock device rejects set_flow_control
    let (mut session, _) = new_session_with_mock_failing_flow(initial_cfg()).await;
    let snapshot = session.current_config().clone();
    let new_cfg = SerialConfig { baud: 9600, flow: FlowControl::Hardware, ..initial_cfg() };
    let err = session.apply_config(new_cfg).await.unwrap_err();
    // device must be back at snapshot
    assert_eq!(session.current_config(), &snapshot);
    assert!(matches!(err, Error::FlowControlUnsupported));
}
```

**Step 2:fail** → **Step 3:实现(需要 mock SerialDevice trait object)**

```rust
impl Session {
    pub async fn apply_config(&mut self, new: SerialConfig) -> Result<(), Error> {
        let old = self.current_config.clone();
        // apply in fixed order:
        //   baud → data_bits → stop_bits → parity → flow
        if let Err(e) = self.device.set_baud_rate(new.baud) { self.rollback(&old).await; return Err(e); }
        if let Err(e) = self.device.set_data_bits(new.data_bits) { self.rollback(&old).await; return Err(e); }
        if let Err(e) = self.device.set_stop_bits(new.stop_bits) { self.rollback(&old).await; return Err(e); }
        if let Err(e) = self.device.set_parity(new.parity) { self.rollback(&old).await; return Err(e); }
        if let Err(e) = self.device.set_flow_control(new.flow) { self.rollback(&old).await; return Err(e); }
        self.current_config = new.clone();
        let _ = self.bus.send(Event::ConfigChanged(new));
        Ok(())
    }

    async fn rollback(&mut self, snapshot: &SerialConfig) {
        // best-effort, ignore errors — device is already inconsistent
        let _ = self.device.set_baud_rate(snapshot.baud);
        let _ = self.device.set_data_bits(snapshot.data_bits);
        let _ = self.device.set_stop_bits(snapshot.stop_bits);
        let _ = self.device.set_parity(snapshot.parity);
        let _ = self.device.set_flow_control(snapshot.flow);
    }
}
```

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(core): Session::apply_config with rollback (v0.2 task 5)"
```

---

## Phase 3 — rtcom-tui 骨架 + SerialPane

### Task 6:创建 `rtcom-tui` crate 骨架 + TuiApp lifecycle

**Files:**
- Create: `crates/rtcom-tui/Cargo.toml`(依赖 ratatui, crossterm, vt100, tui-term, tokio, rtcom-core, anyhow, tracing)
- Create: `crates/rtcom-tui/src/lib.rs`
- Create: `crates/rtcom-tui/src/app.rs`(TuiApp 结构)
- Create: `crates/rtcom-tui/src/terminal.rs`(alternate-screen lifecycle + RawModeGuard 搬移)
- Modify: 根 `Cargo.toml` workspace.dependencies 加 `ratatui = "0.26"`, `vt100 = "0.15"`, `tui-term = "0.1"`

**Step 1:写测试(lifecycle smoke test)**

```rust
#[test]
fn tui_app_builds_without_running() {
    let (tx, _rx) = tokio::sync::broadcast::channel(64);
    let app = TuiApp::new(tx);
    assert!(!app.is_menu_open());
}
```

**Step 2:fail** → **Step 3:实现最小骨架**

```rust
// app.rs
pub struct TuiApp {
    bus: broadcast::Sender<Event>,
    menu_open: bool,
    // serial_pane, modal_stack, etc. added in later tasks
}

impl TuiApp {
    pub fn new(bus: broadcast::Sender<Event>) -> Self {
        Self { bus, menu_open: false }
    }
    pub fn is_menu_open(&self) -> bool { self.menu_open }
}
```

`terminal.rs` 承担:
- `enter_alt_screen()` / `leave_alt_screen()`
- 搬移 `rtcom-cli/src/tty.rs` 的 RawModeGuard(但保留旧文件直到 Task 16 集成)

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): rtcom-tui crate skeleton (v0.2 task 6)"
```

---

### Task 7:SerialPane widget(vt100 集成)

**Files:**
- Create: `crates/rtcom-tui/src/serial_pane.rs`

**Step 1:测试**

```rust
#[test]
fn serial_pane_ingests_bytes_into_vt100() {
    let mut pane = SerialPane::new(24, 80);
    pane.ingest(b"hello\r\nworld");
    let screen = pane.screen();
    assert_eq!(screen.cell(0, 0).unwrap().contents(), "h");
    assert_eq!(screen.cell(1, 0).unwrap().contents(), "w");
}

#[test]
fn serial_pane_resize_preserves_scrollback() {
    let mut pane = SerialPane::new(24, 80);
    for _ in 0..30 { pane.ingest(b"line\r\n"); }
    pane.resize(40, 80);
    // no panic, screen size reflects
    assert_eq!(pane.screen().size(), (40, 80));
}
```

**Step 2:fail** → **Step 3:实现**

```rust
pub struct SerialPane {
    parser: vt100::Parser,
}

impl SerialPane {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self { parser: vt100::Parser::new(rows, cols, 10_000) }
    }
    pub fn ingest(&mut self, bytes: &[u8]) { self.parser.process(bytes); }
    pub fn screen(&self) -> &vt100::Screen { self.parser.screen() }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(rows, cols);
    }
}
```

Widget 渲染:如果 `tui-term` 的 `PseudoTerminal` API 可用,直接用;否则写一个 `impl Widget for &SerialPane`,遍历 `screen().cells()` 填 `Buffer`。在 render 模块(`serial_pane_render.rs`)里。

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): SerialPane backed by vt100 (v0.2 task 7)"
```

---

### Task 8:主屏布局(top bar / body / bottom bar)

**Files:**
- Create: `crates/rtcom-tui/src/layout.rs`
- Modify: `crates/rtcom-tui/src/app.rs`(加入 render 方法)

**Step 1:snapshot 测试**

```rust
// 使用 ratatui::backend::TestBackend
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn main_screen_layout_80x24_snapshot() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let (tx, _rx) = tokio::sync::broadcast::channel(64);
    let mut app = TuiApp::new(tx);
    app.set_device_summary("/dev/ttyUSB0", "115200 8N1 none");
    terminal.draw(|f| app.render(f)).unwrap();
    insta::assert_debug_snapshot!(terminal.backend().buffer());
}
```

**Step 2:fail** → **Step 3:实现**

```rust
// layout.rs
pub fn main_chrome(area: Rect) -> (Rect /*top*/, Rect /*body*/, Rect /*bottom*/) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ]).split(area);
    (rows[0], rows[1], rows[2])
}

// app.rs
pub fn render(&mut self, f: &mut Frame<'_>) {
    let (top, body, bottom) = layout::main_chrome(f.area());
    f.render_widget(Paragraph::new(self.top_bar_line()), top);
    f.render_widget(&self.serial_pane, body);
    f.render_widget(Paragraph::new(self.bottom_bar_line()), bottom);
    // modal overlay in later task
}
```

加 `insta` 到 `[dev-dependencies]`。

**Step 4:pass(用 `cargo insta review` 确认首个 snapshot)** → **Step 5:Commit**

```bash
git commit -m "feat(tui): main screen layout with snapshot test (v0.2 task 8)"
```

---

### Task 9:输入分发 — 菜单关闭态走 CommandKeyParser

**Files:**
- Create: `crates/rtcom-tui/src/input.rs`
- Modify: `crates/rtcom-tui/src/app.rs`

**Step 1:测试**

```rust
#[test]
fn key_passthrough_when_menu_closed() {
    let mut app = build_app();
    let out = app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(out, Dispatch::TxBytes(b"h".to_vec()));
}

#[test]
fn ctrl_a_then_m_opens_menu() {
    let mut app = build_app();
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    let out = app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert_eq!(out, Dispatch::OpenedMenu);
    assert!(app.is_menu_open());
}
```

**Step 2:fail** → **Step 3:实现**

```rust
pub enum Dispatch {
    TxBytes(Vec<u8>),
    OpenedMenu,
    ClosedMenu,
    Quit,
    Noop,
}

// crossterm KeyEvent → byte(s) → CommandKeyParser
impl TuiApp {
    pub fn handle_key(&mut self, key: KeyEvent) -> Dispatch {
        if self.menu_open {
            // delegated in Task 11 (ModalStack)
            return Dispatch::Noop;
        }
        let bytes = key_to_bytes(key);
        match self.command_parser.feed_all(&bytes) {
            ParseOutcome::Passthrough(b) => Dispatch::TxBytes(b),
            ParseOutcome::Command(Command::OpenMenu) => {
                self.menu_open = true;
                let _ = self.bus.send(Event::MenuOpened);
                Dispatch::OpenedMenu
            }
            ParseOutcome::Command(Command::Quit) => Dispatch::Quit,
            // ... other existing commands
        }
    }
}
```

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): input dispatcher via CommandKeyParser (v0.2 task 9)"
```

---

## Phase 4 — 菜单 + Dialogs

### Task 10:ModalStack + `Dialog` trait

**Files:**
- Create: `crates/rtcom-tui/src/modal.rs`

**Step 1:测试**

```rust
#[test]
fn modal_stack_push_pop() {
    let mut stack = ModalStack::new();
    assert!(stack.top().is_none());
    stack.push(Box::new(DummyDialog::default()));
    assert!(stack.top().is_some());
    stack.pop();
    assert!(stack.top().is_none());
}

#[test]
fn modal_stack_routes_key_to_top() {
    let mut stack = ModalStack::new();
    let d = CountingDialog::default();
    let handle = d.count.clone();
    stack.push(Box::new(d));
    stack.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(*handle.lock().unwrap(), 1);
}
```

**Step 2:fail** → **Step 3:实现**

```rust
pub trait Dialog: Send {
    fn render(&self, area: Rect, buf: &mut Buffer, style: ModalStyle);
    fn handle_key(&mut self, key: KeyEvent) -> DialogOutcome;
    fn title(&self) -> &str;
}

pub enum DialogOutcome {
    Consumed,
    Close,
    Action(DialogAction),
}

pub enum DialogAction {
    ApplyLive(SerialConfig),
    ApplyAndSave(SerialConfig, Profile),
    SetModalStyle(ModalStyle),
    WriteProfile,
    ReadProfile,
}

pub struct ModalStack { stack: Vec<Box<dyn Dialog>> }
impl ModalStack { /* push/pop/top/handle_key */ }
```

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): ModalStack + Dialog trait (v0.2 task 10)"
```

---

### Task 11:顶层 `Configuration` 菜单

**Files:**
- Create: `crates/rtcom-tui/src/menu/mod.rs`
- Create: `crates/rtcom-tui/src/menu/root.rs`

**Step 1:snapshot + key 测试**

```rust
#[test]
fn root_menu_snapshot() {
    let menu = RootMenu::new();
    render_dialog_snapshot("root_menu", &menu, 80, 24);
}

#[test]
fn root_menu_arrow_navigation() {
    let mut m = RootMenu::new();
    assert_eq!(m.selected(), 0);
    m.handle_key(down_arrow());
    assert_eq!(m.selected(), 1);
    m.handle_key(up_arrow());
    assert_eq!(m.selected(), 0);
}

#[test]
fn root_menu_enter_opens_serial_port_setup() {
    let mut m = RootMenu::new();
    let out = m.handle_key(enter());
    assert!(matches!(out, DialogOutcome::Action(DialogAction::OpenSubdialog(SubDialog::SerialPortSetup))));
}

#[test]
fn root_menu_esc_closes() {
    let mut m = RootMenu::new();
    let out = m.handle_key(esc());
    assert!(matches!(out, DialogOutcome::Close));
}
```

**Step 2:fail** → **Step 3:实现** RootMenu 带 7 项:Serial port setup / Line endings / Modem control / Write profile / Read profile / Screen options / Exit menu。

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): root Configuration menu (v0.2 task 11)"
```

---

### Task 12:Serial port setup dialog

**Files:**
- Create: `crates/rtcom-tui/src/menu/serial_port.rs`

**Step 1:测试**

```rust
#[test]
fn serial_port_dialog_snapshot() { /* insta */ }

#[test]
fn serial_port_edit_baud_cycles_common_values() {
    let mut d = SerialPortDialog::new(SerialConfig::default());
    d.focus_field(Field::Baud);
    d.handle_key(plus());
    assert_eq!(d.pending().baud, 230_400);  // assuming 115200 was current
}

#[test]
fn serial_port_f2_emits_apply_live() {
    let mut d = SerialPortDialog::new(SerialConfig::default());
    d.pending_mut().baud = 9600;
    let out = d.handle_key(f2());
    assert!(matches!(out, DialogOutcome::Action(DialogAction::ApplyLive(_))));
}

#[test]
fn serial_port_f10_emits_apply_and_save() { /* ... */ }

#[test]
fn serial_port_esc_discards_pending() {
    let mut d = SerialPortDialog::new(SerialConfig::default());
    d.pending_mut().baud = 9600;
    let out = d.handle_key(esc());
    assert!(matches!(out, DialogOutcome::Close));
    // pending dropped;caller的当前 config 不变
}
```

**Step 2:fail** → **Step 3:实现**(带 pending vs committed config 分离)

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): Serial port setup dialog (v0.2 task 12)"
```

---

### Task 13:Line endings dialog

**Files:**
- Create: `crates/rtcom-tui/src/menu/line_endings.rs`

同 Task 12 模式,枚举字段用 `↑↓`/`Space` 循环值,`F2`/`F10`/`Esc` 三动作。

**Commit:** `feat(tui): Line endings dialog (v0.2 task 13)`

---

### Task 14:Modem control dialog

**Files:**
- Create: `crates/rtcom-tui/src/menu/modem.rs`

显示当前 DTR/RTS/CTS/DSR 状态 + 提供 `Raise DTR` / `Lower DTR` / `Raise RTS` / `Lower RTS` / `Send break` 动作。每个动作立即生效(无 pending 阶段,因为这些是瞬时命令),结束后 Esc 回菜单。

**Commit:** `feat(tui): Modem control dialog (v0.2 task 14)`

---

### Task 15:Write/Read profile + Screen options dialogs

**Files:**
- Create: `crates/rtcom-tui/src/menu/profile_io.rs`(Write + Read 共用)
- Create: `crates/rtcom-tui/src/menu/screen.rs`

Write profile:一个确认对话框(显示目标路径,Y/N)→ `DialogAction::WriteProfile`。
Read profile:同上 → `DialogAction::ReadProfile`(会弹 toast 覆盖当前 unsaved 提示)。
Screen options:三个 radio(Overlay / Dimmed / Fullscreen)+ scrollback rows 只读显示。

**Commit:** `feat(tui): profile-io + screen options dialogs (v0.2 task 15)`

---

## Phase 5 — 集成与 apply 流

### Task 16:`rtcom-cli` 瘦身 — 接入 TuiApp

**Files:**
- Delete: `crates/rtcom-cli/src/terminal.rs`
- Delete: `crates/rtcom-cli/src/stdin.rs`
- Delete: `crates/rtcom-cli/src/tty.rs`(搬到 rtcom-tui)
- Modify: `crates/rtcom-cli/src/main.rs`(改用 `rtcom_tui::run(session, profile).await`)
- Modify: `crates/rtcom-cli/Cargo.toml`(加 `rtcom-tui` 依赖)

**Step 1:e2e 冒烟测试不破**(先跑现有 tests,识别破坏点)

```bash
cargo test -p rtcom-cli
```

**Step 2:实现 `rtcom_tui::run`**

```rust
// crates/rtcom-tui/src/lib.rs
pub async fn run(
    session: Session,
    initial_profile_path: Option<PathBuf>,
    initial_profile: Profile,
) -> Result<(), Error> {
    let _raw = RawModeGuard::new()?;
    let _alt = AltScreenGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut app = TuiApp::new(session.bus_sender().clone());
    // main event loop: select! {
    //   crossterm event,
    //   session bus events,
    //   tick
    // } → dispatch → render
    Ok(())
}
```

**Step 3:调整 e2e 测试预期**(Task 21 会补新测试;此处先让编译通过,旧测试可能部分需要临时 `#[ignore]` 并在 Task 21 里 un-ignore)

**Step 4:`cargo run -p rtcom-cli /dev/pts/X` 手动冒烟**

**Step 5:Commit**

```bash
git commit -m "feat(cli): wire rtcom-tui, delete old renderer (v0.2 task 16)"
```

---

### Task 17:Apply live 端到端

**Files:**
- Modify: `crates/rtcom-tui/src/app.rs`(消费 ModalStack 的 `Action` 并调 `Session::apply_config`)
- Modify: `crates/rtcom-tui/src/lib.rs`(主 loop 路由 action 到 session handle)

**Step 1:集成测试(在 rtcom-tui 层 mock device)**

```rust
#[tokio::test]
async fn apply_live_propagates_to_device() {
    let (session, mut device_observer) = build_session_with_observer();
    let mut app = TuiApp::with_session_handle(session.handle());
    // simulate: ^A m → Enter (Serial port setup) → change baud → F2
    app.handle_key(ctrl_a()); app.handle_key(char_m());
    app.handle_key(enter());                  // open Serial port setup
    app.handle_key(plus());                   // baud cycle up
    app.handle_key(f2());                     // apply live
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(device_observer.last_baud_set(), Some(230_400));
}
```

**Step 2:fail** → **Step 3:实现** action routing:

```rust
match dialog_action {
    DialogAction::ApplyLive(cfg) => {
        self.session_handle.apply_config(cfg).await?;
        // Session publishes ConfigChanged → top bar updates on next render
    }
    // ...
}
```

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): Apply live wiring dialog→session (v0.2 task 17)"
```

---

### Task 18:Apply + Save 端到端

**Files:**
- Modify: `crates/rtcom-tui/src/app.rs`
- Modify: `crates/rtcom-tui/src/lib.rs`

**Step 1:测试**

```rust
#[tokio::test]
async fn apply_and_save_writes_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("default.toml");
    let mut app = build_app_with_profile_path(&path);
    // navigate and F10
    app.press(ctrl_a()); app.press(char_m());
    app.press(enter()); app.press(plus()); app.press(f10());
    tokio::time::sleep(Duration::from_millis(20)).await;
    let on_disk = rtcom_config::read(&path).unwrap();
    assert_eq!(on_disk.serial.baud, 230_400);
}
```

**Step 2:fail** → **Step 3:实现** — action handler 对 `ApplyAndSave` 先调 `session.apply_config` 再 spawn blocking `rtcom_config::write(path, &profile)`;失败 publish `ProfileLoadFailed`(复用作为通用 profile IO 失败信号)。

**Step 4:pass** → **Step 5:Commit**

```bash
git commit -m "feat(tui): Apply+Save writes profile (v0.2 task 18)"
```

---

### Task 19:Toast 通知

**Files:**
- Create: `crates/rtcom-tui/src/toast.rs`
- Modify: `crates/rtcom-tui/src/app.rs`(订阅 `Event::Error` / `ProfileLoadFailed` / `ProfileSaved`)

**Step 1:测试**

```rust
#[test]
fn toast_auto_dismisses_after_3_seconds() {
    let mut q = ToastQueue::new();
    q.push("saved", ToastLevel::Info);
    assert_eq!(q.visible_count(), 1);
    q.tick(Duration::from_secs(4));
    assert_eq!(q.visible_count(), 0);
}

#[test]
fn toast_render_snapshot() { /* insta */ }
```

**Step 2-5:** 实现 + commit

```bash
git commit -m "feat(tui): toast notifications for profile IO + errors (v0.2 task 19)"
```

---

## Phase 6 — 测试 / 文档 / 发布

### Task 20:Snapshot 测试矩阵补齐

**Files:**
- Modify: `crates/rtcom-tui/src/snapshots/`(insta 生成)
- Add: 测试用例覆盖 80×24 和 120×40 下的 main 屏、root menu、Serial port dialog、三种 modal_style

**Commit:** `test(tui): snapshot coverage 80x24 + 120x40 (v0.2 task 20)`

---

### Task 21:E2E 测试(socat PTY)新增 / 更新

**Files:**
- Modify: `crates/rtcom-cli/tests/e2e_basic.rs`(把 Task 16 `#[ignore]` 的恢复)
- Create: `crates/rtcom-cli/tests/e2e_menu.rs`

新增用例:
- `test_menu_open_close`:发 `\x01m`(`^A m`)→ 检 alternate screen 切换
- `test_apply_live_baud`:交互改 baud → PTY 对端 stty 验证
- `test_apply_and_save`:F10 后检 profile 文件
- `test_profile_load_on_startup`:预写 profile,启动后 top bar 反映
- `test_save_cli_flag`:`rtcom ... -b 9600 --save` 启动写 profile,退出后验证

**Commit:** `test(cli): e2e coverage for menu + apply flows (v0.2 task 21)`

---

### Task 22:文档更新

**Files:**
- Modify: `README.md`(新截图、`^A m` 提示、profile 路径、`-c` + `--save` flags、v0.2 breaking 警示)
- Modify: `man/rtcom.1`(对应章节)
- Create: `docs/tui.md`(菜单速查、快捷键表、modal 模式说明)
- Create: `docs/adr/008-ratatui-tui.md`
- Create: `docs/adr/009-vt100-emulator.md`
- Create: `docs/adr/010-directories-xdg.md`
- Modify: `CLAUDE.md` §6 路线图表(反映右移)

**Commit:** `docs: v0.2 user-facing surface + ADRs (v0.2 task 22)`

---

### Task 23:CHANGELOG + 版本 bump + 发布

**Files:**
- Modify: `CHANGELOG.md`(加 `[0.2.0] — YYYY-MM-DD` 节,标 Major breaking:TUI 默认开启)
- Modify: 所有 `Cargo.toml` `version = "0.2.0"`(workspace + 四个 crate)
- Run:`cargo publish --dry-run -p rtcom-config`
- Run:`cargo publish --dry-run -p rtcom-core`(若版本依赖变了)
- Run:`cargo publish --dry-run -p rtcom-tui`
- Run:`cargo publish --dry-run -p rtcom-cli`
- Tag:`git tag -a v0.2.0 -m "Release v0.2.0"`(发布 workflow 自动接管)

**发布后**:GitHub Release 检查、crates.io 四个 crate 都 up、Linux/macOS/Windows 二进制下载验证。

**Commit:** `chore(release): bump to v0.2.0`

---

## 收尾验证清单

按 CLAUDE.md §10 的要求,发布前过一遍手工硬件矩阵:

- [ ] CP210x / FTDI / CH340 三种 USB-Serial
- [ ] STM32 + ESP32 各一块
- [ ] 波特率:9600 / 115200 / 921600 / 3000000
- [ ] 流控:none / hw
- [ ] Ubuntu / macOS / Windows(若 Windows CI 通过)
- [ ] 三种 modal style(overlay / dimmed / fullscreen)各操作一遍
- [ ] Apply live / Apply+Save 各走一遍
- [ ] Profile 手写 TOML → 启动加载 → `--save` 回写验证
- [ ] 菜单打开时 `^C` 清理 + 锁文件清理
- [ ] Device 断连(拔 USB)菜单自动关

---

## 依赖图(Task 之间)

```
T1 ─┬─ T2 ─ T3
    │
T4 ─┴────────┐
             │
T5 ──────────┤
             ▼
             T6 ─ T7 ─ T8 ─ T9
                          ▼
                          T10 ─ T11 ─┬─ T12
                                     ├─ T13
                                     ├─ T14
                                     └─ T15
                                         ▼
                                         T16 ─ T17 ─ T18 ─ T19
                                                              ▼
                                                              T20 ─ T21 ─ T22 ─ T23
```

T1–T5 可在不同分支并行。T6 依赖 T4(新 Event variants)。T11–T15 可并行(dialog 各自独立)。T17/T18 依赖 T16。

---

## 参考

- 设计稿:`docs/plans/2026-04-18-tui-menu-design.md`
- 架构说明:`CLAUDE.md` §3
- 编码规范:`CLAUDE.md` §9
- 测试策略:`CLAUDE.md` §10
- Commit 规范:`CLAUDE.md` §8(Conventional Commits)
