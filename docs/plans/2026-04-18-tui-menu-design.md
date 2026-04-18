# rtcom TUI 菜单设计 — 新 v0.2

**日期**:2026-04-18
**状态**:设计确认,待实施
**作者**:TrekMax(协同 Claude)

## 1. 背景与目标

rtcom v0.1.2 沿用 picocom 风格的 `^A`+单键命令交互。对齐 minicom 的长期规划中,需要为**无法用单键表达的复杂配置**提供正经的 dialog 编辑界面。

同时,v0.1 路线图里 v1.0+ 的"多串口对比视图"和"录制回放"都需要多窗口 TUI 基础设施。把这个基础设施提前到 v0.2,可以:

- 避免 v0.2 日志模块先写一版 stdout 渲染、v1.0+ 再重写
- 为后续所有 UI 特性(日志 pane、hex view、过滤条件输入)提供统一的 ratatui 绘制层
- 一次性引入 ratatui 依赖,而不是分散在多个版本

## 2. 决策摘要

| 维度 | 决定 | 备注 |
|---|---|---|
| 菜单定位 | 复杂配置的 dialog 树 | 不做 cheat sheet / 浮层 |
| 作用面 | Live runtime + profile 文件双向 | 菜单内明确区分两条线 |
| 渲染形态 | 全屏 ratatui TUI,alternate screen | |
| ANSI 解析 | `vt100` crate(完整 VT100 语义) | 为后续鼠标选择拷贝 / 远程 ncurses 应用预留 |
| 路线图位置 | 取代原 v0.2,后续版本全部右移 | |
| 菜单条目(v0.2) | Serial port setup / Line endings / Modem control / Profile save-load / Screen options | |
| 触发键 | `^A m` | minicom 的 `^A o` 助记偏弱,`m` 直接对应 menu |
| 导航 | 方向键 + Enter,无首字母 shortcut | 比 minicom 更直观 |
| 与 `^A` 单键关系 | 追加,零破坏 | 所有现有单键保留;菜单打开时 `^A b/t/g/c/\` 屏蔽,仅 `q/x/m/?` 生效 |

## 3. 架构

### 3.1 Crate 布局

新增 **`rtcom-tui`** 独立 crate(不并入 rtcom-cli)。职责:ratatui 渲染循环、SerialPane、ModalStack、dialog 实现。通过 `broadcast::Sender<Event>` 和 mpsc 指令通道与 `rtcom-core::Session` 解耦。

```
crates/
├── rtcom-core/      # 不变,Session / SerialDevice / Event / Mapper / CommandKeyParser
├── rtcom-config/    # 新增,Profile load/save(最小形态)
├── rtcom-tui/       # 新增,TUI + 菜单
└── rtcom-cli/       # 瘦身,删除 terminal.rs + stdin.rs
```

### 3.2 事件流

```
serial_reader  ─▶ Event::RxBytes ──┐
                                    ├─▶ TuiApp (每帧渲染)
crossterm input ─▶ TuiApp ──────────┤   ├─ SerialPane(vt100 emulator + cell grid)
                                    │   └─ ModalStack(menu / dialog)
                                    │
TuiApp ─▶ Event::TxBytes / Command / ConfigChanged / MenuOpened / MenuClosed / ProfileSaved
```

- 删除 `crates/rtcom-cli/src/terminal.rs`(225 行)与 `stdin.rs`(194 行)
- `CommandKeyParser` 留在 `rtcom-core`,由 TuiApp 复用
- 新增 Event variant:`MenuOpened` / `MenuClosed` / `ProfileSaved { path }` / `ProfileLoadFailed { path, error }`
- 新增 `Command::OpenMenu`(`^A m` 触发)

### 3.3 输入路由状态机

```
crossterm KeyEvent
     │
     ▼
 TuiApp.dispatch
     │
   menu_open?
   ┌─┴─┐
  no   yes
   │    │
   ▼    ▼
 key→bytes         modal_stack.handle(key)
 → CommandKeyParser       │
 → TxBytes/Command  ┌─────┴─────┐
      │          Consumed    Action
      ├─ OpenMenu  (吞)   ┌───┴────────┐
      │                ApplyLive    CloseDialog
      ▼                ApplyAndSave  CloseMenu
 Session handles
```

逃生通道(菜单打开时仍生效):
- `^A q` / `^A x` → 关菜单 + 退出
- `^A m` → 关菜单(toggle)
- `^C` / SIGTERM → cancellation,TuiApp drop 路径恢复 termios

### 3.4 Modal 呈现模式

不支持真·半透明(终端无 alpha)。三种样式,菜单内 Screen options 切换:

| 模式 | 背景 serial pane | 用途 |
|---|---|---|
| `overlay`(默认) | 每帧重绘,亮度不变 | 大终端保留上下文 |
| `dimmed-overlay` | 每帧重绘,cell 降亮度 | 聚焦 modal 但保留余光 |
| `fullscreen` | 隐藏,但后台继续缓冲 | 小终端 / 80x24 / minicom 风味 |

`windowed`(上下分栏)延后到 v0.3+。

### 3.5 SerialPane

- 底层:`vt100::Parser`(2D cell grid + scrollback)
- 适配层:`tui-term` crate(把 vt100 Screen 渲染成 ratatui widget;若其 API 不合适,自写 40 行 widget)
- RX 路径:`serial_reader → LineEndingMapper (imap) → vt100.process() → cell grid → render`
- Scrollback 容量:10,000 行(硬编码,v0.3 日志模块接手时开 CLI 选项)
- 终端 resize:`crossterm::event::Event::Resize` → `parser.set_size(rows, cols)`

## 4. UI/UX

### 4.1 主屏(无菜单态)

```
┌─── rtcom 0.2.0 ──────────────── /dev/ttyUSB0  115200 8N1 none ──┐
│                                                                   │
│  (SerialPane: vt100 cell grid,ANSI 透传,scrollback 10k)          │
│                                                                   │
├── ^A m menu · ^A ? help · ^A q quit ───── DTR● RTS● CTS○ DSR○ ───┤
└───────────────────────────────────────────────────────────────────┘
```

Top bar:版本 / 设备路径 / 实时配置摘要
Bottom bar:热键提示 + modem 状态灯(定期 poll)

### 4.2 顶层菜单(`^A m`)

```
┌───── Configuration ─────┐
│                         │
│  > Serial port setup    │
│    Line endings         │
│    Modem control        │
│    ─────────────────    │
│    Write profile        │
│    Read profile         │
│    ─────────────────    │
│    Screen options       │
│    Exit menu            │
│                         │
└─ ↑↓ · Enter · Esc close─┘
```

### 4.3 Serial port setup dialog(代表性)

```
┌──── Serial port setup ─────┐
│                            │
│  Baud rate     115200      │
│  Data bits     8           │
│  Stop bits     1           │
│  Parity        none        │
│  Flow ctrl     none        │
│                            │
├── actions ─────────────────┤
│  [Apply live]      (F2)    │
│  [Apply + Save]    (F10)   │
│  [Cancel]          (Esc)   │
│                            │
└────────────────────────────┘
```

- `↑↓` 移动当前字段 / action
- 数值字段 `Enter` 激活内嵌输入(或 `+`/`-` 在常见值循环,baud 为 9600/19200/38400/57600/115200/230400/460800/921600/3000000)
- 枚举字段 `Enter`/`Space` 循环值
- `F2` Apply live:立即推给 device,不落盘
- `F10` Apply + Save:先 live 应用,再写入 profile
- `Esc`:放弃

其它 dialog(Line endings、Modem control、Screen options、Write/Read profile)同构。

## 5. 数据流与 Profile

### 5.1 Profile 位置

| 平台 | 路径 |
|---|---|
| Linux/BSD | `$XDG_CONFIG_HOME/rtcom/default.toml`(fallback `~/.config/rtcom/default.toml`) |
| macOS | `~/Library/Application Support/rtcom/default.toml` |
| Windows | `%APPDATA%\rtcom\default.toml` |

`-c PATH` CLI 参数覆盖默认路径。文件不存在则以内置默认值启动,不报错。

### 5.2 Schema(v0.2)

```toml
[serial]
baud      = 115200
data_bits = 8          # 5|6|7|8
stop_bits = 1          # 1|2
parity    = "none"     # none|even|odd|mark|space
flow      = "none"     # none|hw|sw

[line_endings]
omap = "none"          # none|add-cr-to-lf|add-lf-to-cr|drop-cr|drop-lf
imap = "none"
emap = "none"

[modem]
initial_dtr = "unchanged"  # unchanged|raise|lower
initial_rts = "unchanged"

[screen]
modal_style     = "overlay"    # overlay|dimmed-overlay|fullscreen
scrollback_rows = 10000
```

### 5.3 策略

- **无 `schema_version` 字段**。未来破坏性改动直接换文件名(`default.v2.toml`)或靠 load 失败 fallback 默认值。
- **Unknown keys 静默丢弃**(`serde` 默认 ignore,save 只写已知字段)。README 明示:菜单 Save 会丢弃手写注释。
- **CLI × Profile × Menu 合并优先级**:
  ```
  defaults < profile < CLI args → effective runtime
  运行时:menu Apply live 覆盖 runtime,Apply+Save 同时更新 profile
  ```
- **CLI args 不自动回写 profile**。用 `--save` 开关显式持久化:`rtcom /dev/ttyUSB0 -b 9600 --save`。
- **Apply + Save 写盘时写整个文件**,不做字段级 merge。

### 5.4 合并冲突

| 场景 | 结果 |
|---|---|
| profile `baud=9600`,CLI `-b 115200` | runtime=115200,profile 不变(直到 Apply+Save 或 `--save`) |
| Profile 损坏 | rtcom 启动用默认值,toast 报错,不覆盖原文件 |
| Apply live 部分字段失败 | 单向回滚,设备回到 apply 前 snapshot,toast 报错 |

## 6. 错误处理

| 类别 | 处理 |
|---|---|
| Profile parse/load/save 错误 | toast,不阻塞启动,`tracing::warn!` |
| Apply live 中途失败 | 回滚前面字段,toast 指示失败字段 |
| vt100 parser 遇未知序列 | vt100 crate 内部 resilient 降级,`tracing::debug!` |
| Device 断连(菜单打开时) | 强制关菜单,顶层红 banner;不自动重连(v0.4 做) |
| ratatui render panic | `set_panic_hook` → RawModeGuard drop → stderr stacktrace |

## 7. 测试

### 7.1 Unit

- `rtcom-core::Session.apply_config`:成功 / 全失败 / 部分失败回滚
- `rtcom-config`:load/save roundtrip、parse error、unknown keys dropped
- `rtcom-tui`:dialog 状态机所有转移(field focus、value cycle、F2/F10/Esc)

### 7.2 Snapshot(新增)

- `ratatui::backend::TestBackend` + `insta`
- 尺寸矩阵:80×24、120×40
- 覆盖:无菜单主屏、顶层菜单、Serial port setup dialog(三种 modal_style 各一张)

### 7.3 Integration(Linux only,socat PTY)

- `test_menu_open_close`:发 `^A m` 检 alternate screen 切换
- `test_apply_live_baud`:改 baud 后 PTY 对端 `stty` 验证
- `test_apply_and_save`:F10 后读 profile 文件验证
- `test_profile_load_on_startup`:预写 profile,启动后 top bar 显示期望值
- `test_save_cli_flag`:`rtcom ... --save` 启动即写入 profile,交互中退出后验证文件内容

### 7.4 CI 矩阵

- Linux:unit + snapshot + e2e
- macOS:unit + snapshot
- Windows:unit + snapshot

## 8. 新增依赖(需 ADR)

| Crate | 用途 | ADR |
|---|---|---|
| `ratatui` 0.26+ | TUI 渲染 | ADR-008 |
| `vt100` | VT100 emulator(2D cell grid + scrollback) | ADR-009 |
| `tui-term` | vt100 ↔ ratatui 适配(可选,若 API 不合适自写 widget) | ADR-009 附录 |
| `directories` | 跨平台 config 路径 | ADR-010 |
| `insta` | snapshot 测试 | ADR-011(dev-dependency) |

## 9. 路线图变形

```
原  v0.2 日志 → v0.3 设备 → v0.4 Profile → v0.5 脚本 → v0.6 文件 → v0.7 网络 → v0.8 Windows → v1.0
新  v0.2 TUI+菜单+最小 Profile → v0.3 日志(基于 TUI) → v0.4 设备 → v0.5 命名 Profile → v0.6 脚本 → v0.7 文件 → v0.8 网络 → v0.9 Windows → v1.0
```

Profile 实际上被拆成两段:v0.2 落最小单文件骨架,v0.5 扩展到命名 profile + `--profile <name>` CLI。

## 10. 非目标(明确不做)

- v0.2 不做鼠标选择/拷贝(架构预留,代码留在 v0.3+)
- v0.2 不做多 pane 分栏(windowed modal style 延后)
- v0.2 不做命名 profile,只有单文件 `default.toml`
- v0.2 不做 profile schema 版本迁移
- v0.2 不做自动设备重连(菜单内断连只显示 banner)
- v0.2 不做 rtcom 自己的日志写盘(那是新 v0.3 的事)

## 11. 对外沟通

- CHANGELOG 标 v0.2.0 为 `Major`,说明 TUI 是默认开启无法关闭(本版本没有"经典 stdout 模式")
- README 更新:新截图、`^A m` 提示、profile 路径、`--save` / `-c` CLI flag
- `man/rtcom.1` 更新对应章节
- 新增 `docs/tui.md`:菜单导航速查、快捷键表、modal 模式切换
