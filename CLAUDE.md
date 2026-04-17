# rtcom 开发计划

> **Rust Terminal Communication** — 用 Rust 实现对齐 tio 功能的现代化串口终端工具,目标 v1.0,可控升级架构。
>
> 本文档作为项目的 `CLAUDE.md`,为 Claude Code 提供完整项目上下文。

---

## 目录

1. [项目概述](#1-项目概述)
2. [目标与非目标](#2-目标与非目标)
3. [架构设计](#3-架构设计)
4. [技术栈决策](#4-技术栈决策)
5. [项目结构](#5-项目结构)
6. [版本路线图](#6-版本路线图)
7. [v0.1 详细任务分解](#7-v01-详细任务分解)
8. [开发工作流](#8-开发工作流)
9. [编码规范](#9-编码规范)
10. [测试策略](#10-测试策略)
11. [发布流程](#11-发布流程)
12. [Claude Code 使用指南](#12-claude-code-使用指南)

---

## 1. 项目概述

### 基本信息

| 字段 | 值 |
|---|---|
| 项目名 | rtcom |
| 全称 | Rust Terminal Communication |
| 类型 | CLI 工具 + 可复用库 |
| 语言 | Rust (MSRV 1.85) |
| License | Apache-2.0 |
| 发布渠道 | crates.io, GitHub Releases, Homebrew, AUR, winget |

### 一句话描述

面向嵌入式 / 硬件工程师的跨平台串口终端,对齐 tio 功能,Rust 实现保证内存与并发安全,架构预留扩展槽位支持后续自定义协议、网络共享、脚本自动化等能力演进。

### 用户画像

- 做 MCU / SoC / 嵌入式系统开发,日常需要连 UART 调试
- 用过 picocom / minicom / screen / tio,对已有工具的痛点有感知
- 需要跨平台(Linux + macOS 为主,Windows 是加分项)
- 有脚本化和日志抓取的自动化需求

---

## 2. 目标与非目标

### 目标

- **功能对齐 tio**:v0.7 前把 tio 的常用功能做完
- **内存与并发安全**:不使用 `unsafe`(FFI 除外),资源清理用 RAII 保证
- **跨平台一等公民**:Linux / macOS / BSD / **Windows 原生**(tio 不支持)
- **可扩展架构**:trait 化设备抽象 + 事件总线,后续功能以订阅者形式加入
- **库 + CLI 双形态**:`rtcom-core` 可被其他 Rust 项目直接 link
- **现代发布**:单二进制分发,`cargo install rtcom` 一键装

### 非目标(明确不做)

- ❌ GUI(与 minicom/PuTTY 划清界限)
- ❌ 网络协议终端(telnet / ssh / serial-over-SSH 客户端)
- ❌ BBS 时代的批量通信套件
- ❌ 取代 pyOCD / OpenOCD 做 JTAG/SWD 调试

### 与 tio 的差异化(长期)

| 能力 | tio | rtcom 计划 |
|---|---|---|
| 原生 Windows 支持 | ❌ | ✅ v0.8 |
| 内置 modem 协议 (不依赖 lrzsz) | ❌ | ✅ v0.6 |
| 结构化输出 (JSON Lines) | ❌ | ✅ v1.0+ |
| 二进制协议解码 (SLIP/COBS/Modbus) | ❌ | ✅ v1.0+ |
| 多串口对比视图 | ❌ | ✅ v1.0+ |
| 录制回放 | ❌ | ✅ v1.0+ |

---

## 3. 架构设计

### 分层结构

```
┌─────────────────────────────────────────────┐
│  USER   Keyboard      Terminal Display      │
└──────────┬─────────────────▲────────────────┘
           ▼                 │
┌─────────────────────────────────────────────┐
│  rtcom-cli   clap · profile · entry         │
└──────────┬─────────────────▲────────────────┘
           ▼                 │
┌─────────────────────────────────────────────┐
│  rtcom-core                                  │
│                                              │
│  stdin reader ──▶ Command Key SM ──▶ Event  │
│                                      Bus     │
│                                       ▲ ▼    │
│  Session Orchestrator ◀─────────────▶ ─────  │
│                                              │
│  Serial Reader ──▶ Mapper Chain ──▶ Term    │
│                                              │
│  [Support crates: config / log / xfer /     │
│   script  subscribe via broadcast channel]  │
└──────────┬─────────────────▲────────────────┘
           ▼                 │
┌─────────────────────────────────────────────┐
│  SerialDevice trait (AsyncRead + AsyncWrite)│
└──────────┬─────────────────▲────────────────┘
           ▼                 │
  serialport crate → OS (termios / Win32 COM) → Hardware
```

详见项目根目录 `docs/architecture.svg`。

### 核心抽象

```rust
// rtcom-core/src/device.rs
pub trait SerialDevice: AsyncRead + AsyncWrite + Send + Unpin {
    fn set_baud_rate(&mut self, baud: u32) -> Result<()>;
    fn set_data_bits(&mut self, bits: DataBits) -> Result<()>;
    fn set_stop_bits(&mut self, bits: StopBits) -> Result<()>;
    fn set_parity(&mut self, parity: Parity) -> Result<()>;
    fn set_flow_control(&mut self, flow: FlowControl) -> Result<()>;
    fn set_dtr(&mut self, level: bool) -> Result<()>;
    fn set_rts(&mut self, level: bool) -> Result<()>;
    fn send_break(&mut self, duration: Duration) -> Result<()>;
    fn modem_status(&self) -> Result<ModemStatus>;
    fn config(&self) -> &SerialConfig;
}

// rtcom-core/src/event.rs
#[derive(Clone, Debug)]
pub enum Event {
    RxBytes(Bytes),
    TxBytes(Bytes),
    UserInput(InputKind),
    Command(Command),
    ConfigChanged(SerialConfig),
    DeviceConnected,
    DeviceDisconnected { reason: String },
    Error(Arc<Error>),
}

// rtcom-core/src/mapper.rs
pub trait Mapper: Send {
    fn map_rx(&mut self, bytes: &[u8], out: &mut dyn io::Write) -> Result<()>;
    fn map_tx(&mut self, bytes: &[u8], out: &mut dyn io::Write) -> Result<()>;
}

// rtcom-core/src/session.rs
pub struct Session {
    device: Box<dyn SerialDevice>,
    bus: broadcast::Sender<Event>,
    mappers: Vec<Box<dyn Mapper>>,
    // ...
}
```

### 并发模型

基于 `tokio`,主流程 task:

| Task | 职责 | 输入 | 输出 |
|---|---|---|---|
| `serial_reader` | 读串口非阻塞 | device.read() | `Event::RxBytes` |
| `stdin_reader` | 读键盘 raw 模式 | crossterm events | `Event::UserInput` |
| `command_parser` | 识别命令键 | `UserInput` | `Event::Command` or bypass to TxBytes |
| `orchestrator` | 调度分派 | 所有 Event | writer/logger/script 调用 |
| `serial_writer` | 写串口 | `Event::TxBytes` | device.write() |
| `terminal_writer` | 写终端 | `Event::RxBytes` | stdout |

事件广播用 `tokio::sync::broadcast`,多订阅者天然支持。统一用 `CancellationToken` 关停。

### 错误处理

- **库层**(`rtcom-*`):`thiserror` 定义具体错误类型,**绝不 panic**,绝不 swallow
- **二进制层**(`rtcom-cli`):`anyhow` 汇总,带来源链友好报告
- **可恢复错误**(如设备断开):变成 `Event::DeviceDisconnected`,重连状态机处理
- **终端状态恢复**:RAII guard,即使 panic 也能恢复 termios

---

## 4. 技术栈决策

### ADR-001: 串口库选 `serialport`

**决策**:使用 `serialport` crate (v4.x)
**理由**:跨平台最成熟,社区主流,API 覆盖 termios 全部能力
**取舍**:底层 API 略啰嗦,但省掉写多平台后端的工作量
**备选考虑**:`tokio-serial`(基于 serialport,适配 AsyncRead/Write)

### ADR-002: 异步运行时选 `tokio`

**决策**:使用 `tokio` 多线程运行时
**理由**:串口 + stdin + socket 三路 I/O 天然适合 async;生态最全
**取舍**:二进制体积略大,可接受
**备选考虑**:纯线程 + channel(对简单拓扑也可行,但 v0.7 socket 共享会复杂化)

### ADR-003: CLI 解析选 `clap` derive

**决策**:`clap` v4 derive 宏
**理由**:类型安全,帮助自动生成,subcommand 支持好
**备选考虑**:`argh`(更轻,但灵活性不够)

### ADR-004: 终端交互选 `crossterm`

**决策**:`crossterm` 做 raw 模式与按键读取
**理由**:跨平台,覆盖 Windows;v1.0+ 做 TUI 时(ratatui)原生兼容
**取舍**:某些底层 termios 控制仍需 `nix`/`rustix` 直操

### ADR-005: 配置格式选 TOML

**决策**:TOML + serde
**理由**:Rust 生态标配,注释友好,对嵌入式工程师习惯
**备选考虑**:YAML(锯齿问题多),JSON(无注释)

### ADR-006: 日志门面选 `tracing`

**决策**:`tracing` + `tracing-subscriber`
**理由**:结构化,便于未来 JSON Lines 输出
**注意**:`tracing` 是给开发者看的诊断日志;用户的"串口数据日志"是另一套(`rtcom-log` crate)

### ADR-007: 脚本引擎选 `mlua`

**决策**:`mlua` (Lua 5.4),feature gate `script`
**理由**:与 tio 兼容思路,成熟绑定,嵌入式工程师普遍接受
**取舍**:Lua 不是 Rust 原生,增加构建复杂度;用 feature 隔离,默认开启可按需关闭

---

## 5. 项目结构

```
rtcom/
├── Cargo.toml                      # workspace root
├── rust-toolchain.toml             # 固定工具链版本
├── rustfmt.toml
├── clippy.toml
├── .github/
│   └── workflows/
│       ├── ci.yml                  # fmt / clippy / test (Linux/macOS/Windows)
│       ├── release.yml             # cargo-dist 自动发布
│       └── audit.yml               # cargo-audit 依赖安全
├── crates/
│   ├── rtcom-core/                 # 核心库
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── device.rs           # SerialDevice trait + 实现
│   │       ├── event.rs            # Event enum + Bus
│   │       ├── session.rs          # Session orchestrator
│   │       ├── mapper.rs           # Mapper trait + 内置实现
│   │       ├── command.rs          # Command 枚举与状态机
│   │       ├── config.rs           # SerialConfig 等核心类型
│   │       └── error.rs
│   ├── rtcom-cli/                  # CLI 二进制
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── args.rs             # clap derive
│   │       ├── signal.rs           # SIGINT/TERM/WINCH 处理
│   │       └── tty.rs              # raw 模式 guard
│   ├── rtcom-config/               # Profile + TOML
│   ├── rtcom-log/                  # 日志文件写入、ANSI 剥离
│   ├── rtcom-xfer/                 # xmodem/ymodem(v0.6)
│   └── rtcom-script/               # Lua(v0.5,feature gate)
├── tests/                          # 端到端测试(socat 造 PTY)
│   ├── e2e_basic.rs
│   └── e2e_commands.rs
├── docs/
│   ├── architecture.svg            # 架构图
│   ├── usage.md                    # 用户手册
│   ├── config.md                   # 配置文件与 profile
│   ├── scripting.md                # v0.5 后补
│   └── adr/                        # Architecture Decision Records
│       └── 001-serialport-choice.md
├── examples/                       # rtcom-core 库使用示例
│   └── minimal_session.rs
├── man/
│   └── rtcom.1                     # man page 源文件
├── CHANGELOG.md                    # Keep a Changelog 格式
├── CLAUDE.md                       # 本文档(项目上下文)
├── CONTRIBUTING.md
├── README.md
└── LICENSE                         # Apache-2.0
```

### 根 Cargo.toml 骨架

```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.85"
license = "Apache-2.0"
repository = "https://github.com/YOUR_USER/rtcom"
authors = ["TrekMax <your@email>"]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
serialport = "4"
clap = { version = "4", features = ["derive", "env"] }
crossterm = "0.27"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
bytes = "1"
# workspace-internal
rtcom-core   = { path = "crates/rtcom-core",   version = "0.1.0" }
rtcom-config = { path = "crates/rtcom-config", version = "0.1.0" }
rtcom-log    = { path = "crates/rtcom-log",    version = "0.1.0" }

[workspace.lints.rust]
unsafe_code = "deny"
missing_docs = "warn"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
# 按需放行(后续 ADR 记录)
module_name_repetitions = "allow"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

---

## 6. 版本路线图

| 版本 | 主题 | 关键交付 |
|---|---|---|
| **v0.1** | MVP 可用版 | 基础串口交互,替代 picocom |
| **v0.2** | 日志与显示 | 时间戳 / hex / 彩色 / 日志文件 |
| **v0.3** | 设备管理 | `-L` 列表 / 热插拔 / 等待设备 |
| **v0.4** | 配置与 Profile | TOML 配置 / 命名 profile |
| **v0.5** | 脚本自动化 | Lua 脚本 / 触发器 |
| **v0.6** | 文件传输 | 内置 xmodem / ymodem |
| **v0.7** | 网络与多会话 | TCP/Unix socket 共享 |
| **v0.8** | Windows 原生 | 跨平台完整 |
| **v1.0** | 稳定版 | 文档完善 / 性能基线 |

详细功能清单见本文档首次输出的"功能规划"章节,已内化为上表。每个版本一个 GitHub Milestone,每条功能一个 issue。

---

## 7. v0.1 详细任务分解

这一章是 **Claude Code 执行的主要参考**。每个 issue 独立可验证,按顺序做。

### Issue #1: workspace 骨架与 CI

**目标**:建立可编译的空项目,CI 跑通

**任务**:
- 创建 workspace 目录结构(见 §5)
- 写根 `Cargo.toml`(见 §5 骨架)
- 写 `rust-toolchain.toml` 固定 stable 1.85(满足 clap 4.6+ 等依赖的 `edition2024` 要求)
- 写 `rustfmt.toml`(`edition = "2021"`, `max_width = 100`)
- 创建 `.github/workflows/ci.yml`:
  - 矩阵:`ubuntu-latest`, `macos-latest`, `windows-latest`
  - 步骤:`cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo test --all-features` → `cargo doc --no-deps`
- 创建空的 `crates/rtcom-core`(lib)和 `crates/rtcom-cli`(bin)
- `rtcom-cli` 的 `main.rs` 只需 `fn main() { println!("rtcom"); }`
- 写最小 `README.md`,沿用现有 `LICENSE`(Apache-2.0)

**验收标准**:
- `cargo build --workspace` 成功
- `cargo clippy --workspace -- -D warnings` 无 warning
- CI 在三个平台绿灯
- `cargo run -p rtcom-cli` 打印 "rtcom"

---

### Issue #2: `SerialDevice` trait 与 serialport 后端

**目标**:核心设备抽象 + 首个实现

**任务**:
- `rtcom-core/src/device.rs`:定义 `SerialDevice` trait(见 §3)
- 定义配套类型:`SerialConfig`, `DataBits`, `StopBits`, `Parity`, `FlowControl`, `ModemStatus`
- 实现 `SerialPortDevice`:包装 `serialport::SerialPort`,提供 tokio 友好的 async read/write(用 `spawn_blocking` 或 `tokio-serial`)
- `rtcom-core/src/error.rs`:定义 `Error` 和 `Result<T>`(`thiserror`)

**验收标准**:
- 单元测试:`SerialConfig::default()` 返回 115200/8N1
- 集成测试(有 socat):打开 PTY,写入字节,从另一端读到
- rustdoc 对 trait 所有方法有文档 + 示例

---

### Issue #3: CLI 参数解析

**目标**:基础命令行接口

**任务**:
- `rtcom-cli/src/args.rs`:`clap` derive 定义 `Cli` 结构
- 支持参数:
  - `device`(位置参数,可选,用于 `--list` 时省略)
  - `-b, --baud <RATE>`(默认 115200)
  - `-d, --databits <5|6|7|8>`(默认 8)
  - `-s, --stopbits <1|2>`(默认 1)
  - `-p, --parity <none|even|odd|mark|space>`(默认 none)
  - `-f, --flow <none|hw|sw>`(默认 none)
  - `--no-reset`(启动不 toggle DTR)
  - `--echo`(开本地回显)
  - `--escape <CHAR>`(命令键,默认 `^A`;`^T` 在 tmux/某些终端会被截走)
  - `-q, --quiet`
  - `-v, --verbose`(可累加)
- 解析结果转换成 `rtcom_core::SerialConfig`

**验收标准**:
- `rtcom --help` 输出整齐的帮助
- `rtcom /dev/ttyUSB0 -b 9600 -p even` 正确解析
- `rtcom` 不带参数报错清晰提示需要 device
- 单元测试覆盖所有参数组合

---

### Issue #4: raw 模式 guard

**目标**:退出时必恢复 termios

**任务**:
- `rtcom-cli/src/tty.rs`:实现 `RawModeGuard` 结构
- `new()` 时进入 raw 模式(crossterm `enable_raw_mode`)
- `Drop` 时恢复(`disable_raw_mode`)
- 使用 `ctrlc` crate 或 `tokio::signal` 注册信号处理,保证 Ctrl-C 路径也走 Drop
- 处理 `std::panic::set_hook`:panic 时也恢复

**验收标准**:
- 手动测试:运行 → Ctrl-C → 终端回显正常
- 手动测试:运行 → `panic!()` → 终端回显正常
- 测试脚本:启动、杀死进程、验证 `stty -g` 输出与启动前一致

---

### Issue #5: 事件总线与 orchestrator 骨架

**目标**:事件驱动基础

**任务**:
- `rtcom-core/src/event.rs`:定义 `Event` enum 与 `EventBus`(`broadcast::Sender<Event>`)
- `rtcom-core/src/session.rs`:`Session` 结构体,持有 device、bus、mapper 列表
- `Session::run()`:启动所有 task 并等待
- 实现 `serial_reader_task`:读 device → `Event::RxBytes`
- 实现 `serial_writer_task`:订阅 `Event::TxBytes` → 写 device
- 使用 `CancellationToken` 协调退出

**验收标准**:
- 单元测试:mock device,发送字节 → 从 bus 订阅到 `RxBytes`
- 单元测试:向 bus 发 `TxBytes` → mock device 收到字节
- 取消 token 触发后所有 task 干净退出(无 warning,无孤儿)

---

### Issue #6: stdin 读取与命令键状态机

**目标**:区分"数据字节"与"命令"

**任务**:
- `rtcom-cli/src/stdin.rs`:`stdin_reader_task`,用 crossterm event stream
- `rtcom-core/src/command.rs`:定义 `Command` enum 和 `CommandKeyParser` 状态机
- 状态机逻辑:
  - 默认状态:字节直接变 `Event::TxBytes`
  - 遇到 escape char(默认 `^A`):进入命令等待状态
  - 收到下一字节:查表转成 `Command`(或 `Unknown`)
  - 超时 / Esc:退出命令状态
- 命令键表(v0.1 最小集):
  - `?` / `h` → Help
  - `^X` / `^Q`(Ctrl-X / Ctrl-Q,picocom 风格)→ Quit
  - `b` → 进入改波特率子状态
  - `t` → Toggle DTR
  - `g` → Toggle RTS
  - `\\` → Send Break
  - `c` → Show Config

**验收标准**:
- 单元测试覆盖状态机所有转移
- 集成测试:输入 `^T ?` 触发 Help command
- 集成测试:普通输入透传,Escape 后的非法字符回到默认状态

---

### Issue #7: 运行时命令执行

**目标**:命令从 enum 变成行为

**任务**:
- `Session` 订阅 `Event::Command`,dispatch 到 handler
- 实现各 handler:
  - `Help`:打印命令列表到终端
  - `Quit`:触发 cancellation
  - `ShowConfig`:打印当前 `SerialConfig`
  - `ToggleDtr` / `ToggleRts`:调 device
  - `SendBreak`:250ms break
  - `SetBaud(u32)`:调 `device.set_baud_rate`,发 `Event::ConfigChanged`
- Help / ShowConfig 输出走"系统消息"通道,与串口数据区分(加前缀 `*** rtcom: `)

**验收标准**:
- 手动测试:每个命令都能执行,输出清晰
- 集成测试(mock device):发 `Command::SetBaud(9600)`,验证 device 配置变了
- 系统消息不写进日志文件(需要后续 #10 日志 issue 配合,此 issue 只定好接口)

---

### Issue #8: 行结束符映射 Mapper

**目标**:CR/LF/CRLF 双向转换

**任务**:
- `rtcom-core/src/mapper.rs`:`Mapper` trait + `LineEndingMapper` 实现
- 参考 picocom 的 omap / emap / imap 语义
- 支持配置:`SendMap { AddCrToLf, AddLfToCr, DropCr, DropLf, None }`、`ReceiveMap` 同理
- CLI 参数:`--omap <OPTION>` / `--imap <OPTION>` / `--emap <OPTION>`(echo map)
- `Session` 在发送前应用 omap,接收后应用 imap

**验收标准**:
- 单元测试覆盖所有映射组合
- 集成测试:输入 `\n`,omap=AddCrToLf,实际发送 `\r\n`
- 默认配置下数据透传不变

---

### Issue #9: UUCP 锁文件(Unix only)

**目标**:防多实例打同一串口

**任务**:
- `rtcom-core/src/lock.rs`:`UucpLock` 结构
- 路径:`/var/lock/LCK..ttyUSB0`(需要写权限,失败时降级到 `/tmp/` 并 warn)
- 内容:PID(10 字节 ASCII + `\n`)
- 打开前检查:文件存在 → 读 PID → `kill(pid, 0)` 验活 → 活着报错,死了覆盖
- `Drop` 删除锁文件
- Windows:no-op(空实现)

**验收标准**:
- 集成测试:启动两个 rtcom 连同一 PTY,第二个报错退出
- 手动测试:kill -9 第一个,第二个能启动(清理陈旧锁)
- Windows CI:编译通过(功能 no-op)

---

### Issue #10: 信号处理与优雅退出

**目标**:所有退出路径都干净

**任务**:
- SIGINT / SIGTERM:触发 cancellation → 各 task 退出 → RawModeGuard drop → UucpLock drop
- SIGWINCH:目前仅日志记录(v1.0+ TUI 会用)
- SIGHUP:treated as quit
- 退出码约定:正常 0,错误 1,信号退出 128+signum
- 日志 via tracing,初始化在 main 最早期

**验收标准**:
- 手动测试:kill -TERM / kill -INT / kill -HUP 均干净退出
- 终端状态恢复,锁文件清理
- Windows:对应 Ctrl-C 与 Ctrl-Break 正确处理

---

### Issue #11: 端到端集成测试框架

**目标**:自动化回归

**任务**:
- `tests/e2e_basic.rs`:用 `assert_cmd` 驱动 rtcom 二进制
- 辅助函数:`spawn_pty_pair()` 通过 `socat -d -d PTY,raw,echo=0 PTY,raw,echo=0` 创建两端,解析 stderr 得到两个路径
- 测试用例:
  - `test_basic_passthrough`:rtcom 连一端,测试端写字节,验证读到
  - `test_baud_change`:发 `^T b` 改波特率,验证生效(通过 `stty` 读对端)
  - `test_quit_command`:发 `^T q`,验证进程退出码 0
  - `test_dtr_toggle`:验证 modem status 变化
- macOS 可能没 socat,用 `openpty()` 或跳过这些测试(feature flag `pty-tests`)
- CI 仅在 Linux 跑 e2e 测试(Windows/macOS 主要跑单元测试)

**验收标准**:
- 本地 `cargo test -p rtcom-cli --test e2e_basic` 全绿
- Linux CI 集成 e2e 测试
- 测试运行时间 < 30s

---

### Issue #12: README / man page / v0.1 发布

**目标**:可对外宣布的 0.1.0

**任务**:
- 完善 `README.md`:
  - 徽章:crates.io / docs.rs / CI / license
  - 一段话介绍 + 动图(可选)
  - 安装:`cargo install rtcom` / Homebrew / AUR(占位)
  - 快速开始:`rtcom /dev/ttyUSB0 -b 115200`
  - 命令键列表
  - 与 picocom/tio 对比表
  - 贡献指南链接
- 写 `man/rtcom.1`(roff 源码或用 `help2man` 生成)
- 写 `CHANGELOG.md`(Keep a Changelog 格式),记录 0.1.0
- 更新所有 `Cargo.toml` 的 `version = "0.1.0"`
- 打 tag `v0.1.0`,触发发布 workflow

**验收标准**:
- `cargo publish --dry-run` 成功(所有 crate)
- `man rtcom`(假设安装)显示正常
- GitHub Release 自动生成,包含 Linux/macOS/Windows 二进制
- crates.io 显示 rtcom 0.1.0

---

## 8. 开发工作流

### 核心原则:TDD + 边开发边提交

本项目**强制采用 TDD(测试驱动开发)** 与 **频繁提交** 的工作模式。这两条不是建议,是基本盘。

#### TDD 红 → 绿 → 重构

每个新增/修改的功能,按以下顺序推进:

1. **红(Red)**:先写一个能表达"目标行为"的最小测试,运行它,**确认它失败**。
   - 单元测试 → 放在被测模块下的 `#[cfg(test)] mod tests`
   - 跨模块行为 → 放在 crate 的 `tests/` 下做集成测试
   - 验收级行为 → 放在 `crates/rtcom-cli/tests/` 做端到端
   - 测试必须**先于实现**写出。失败信息要清晰地反映"差什么"。

2. **绿(Green)**:写**最小可用**实现让测试通过,允许丑陋、允许重复,**先跑过再说**。
   - 不要顺便 refactor 已有代码
   - 不要为了"以后可能要"提前抽象
   - `cargo test` 全绿是过这一关的唯一标准

3. **重构(Refactor)**:测试还在绿的前提下,清理刚写的实现。
   - 提取重复 / 命名 / 模块边界
   - 跑 `cargo fmt && cargo clippy --all-targets -- -D warnings` 必须无 warning
   - 重构期间**不允许动测试**;测试是契约,改测试 = 改契约 = 回到红色阶段

#### 提交节奏

**完成一个 TDD 循环就提交一次,不要攒。** Claude Code 在工作过程中,每完成下面任一动作都应当立即 `git commit`:

| 触发 | 提交类型 | 例子 |
|---|---|---|
| 写完红色测试,确认失败 | `test(...)` | `test(core): add failing test for EventBus.publish` |
| 让测试转绿的最小实现 | `feat(...)` / `fix(...)` | `feat(core): implement EventBus.publish` |
| 重构通过后的清理 | `refactor(...)` | `refactor(core): extract spawn_reader_task helper` |
| 文档 / 配置 / CI 改动 | `docs/chore/ci/build(...)` | `docs(core): add SerialDevice trait example` |

**禁止**:
- 一个 commit 同时含两个 issue 的工作
- 一个 commit 同时含"实现 + 重构 + 无关文档调整"
- 把"还在红"的测试和"补救实现"塞同一个 commit(看不出 TDD 痕迹)

**允许且鼓励**:
- 一个 issue 拆成 5+ 个小 commit
- 把测试 commit 和实现 commit 分开,读 git log 时一眼能看到 TDD 痕迹
- 在 commit message 里写为什么这么改(WHY),代码本身已经说了 WHAT

每个 commit 之前必须本地通过:

```bash
cargo fmt --check && \
cargo clippy --workspace --all-targets --all-features -- -D warnings && \
cargo test --workspace --all-features
```

### 分支模型

- `main`:保护分支,只接受 PR
- feature branch:`feat/issue-N-短描述` / `fix/...` / `docs/...`
- 一个 issue 一个 PR,小而快

### Commit 规范

Conventional Commits:

```
<type>(<scope>): <subject>

[body]

[footer]
```

`type`:`feat` `fix` `docs` `style` `refactor` `perf` `test` `build` `ci` `chore`
`scope`:`core` `cli` `config` `log` `xfer` `script` 等

示例:
```
feat(core): implement SerialDevice trait with serialport backend

Closes #2
```

用 `git-cliff` 从提交生成 CHANGELOG。

### PR 必过检查

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features
```

可以用 pre-commit hook 或 `cargo-husky`。

### 里程碑节奏

- 每个 v0.x 开一个 GitHub Milestone
- Milestone 内 issue 按顺序做(有依赖关系时标注 blocked by)
- 完成一个版本 → tag → 自动发布 → 更新 CHANGELOG → 开下一个 Milestone

---

## 9. 编码规范

### Rust 风格

- `rustfmt` 默认 + `max_width = 100`
- `clippy::pedantic` + `clippy::nursery`(按需 allow,记 ADR)
- 禁用 `unsafe`(FFI 场景在 `unsafe` 模块隔离,加 `// SAFETY:` 注释)
- 公开 API 必须有 rustdoc,含 `# Examples`

### 模块组织

- 一个文件一个主要概念
- `pub use` 在 `lib.rs` 或 `mod.rs` 做 re-export,外部只需 `use rtcom_core::{Session, Event}`
- 内部实现用 `pub(crate)`

### 命名

- 类型:`UpperCamelCase`,trait 与结构体同命名空间要避免冲突
- 函数 / 变量:`snake_case`
- 常量:`SCREAMING_SNAKE_CASE`
- 缩写按单词处理:`TcpStream` 不是 `TCPStream`,`SerialId` 不是 `SerialID`

### 错误处理

- 库函数返回 `Result<T, rtcom_core::Error>`
- 枚举每个 variant 对应一个失败域,用 `#[from]` 自动转换 source error
- 不用 `unwrap()` / `expect()` 在生产代码(测试可以)
- `anyhow::Result` 只在 `rtcom-cli` 的 main 及其直接辅助函数

### 依赖引入

- 新增依赖必须在 PR 描述中说明理由
- 优先 workspace.dependencies 统一版本
- 避免 "小工具" 依赖(rev stdlib 的别引)
- 定期 `cargo +nightly udeps` 清无用依赖

---

## 10. 测试策略

### 分层

| 层 | 工具 | 范围 | 触发 |
|---|---|---|---|
| 单元测试 | `cargo test` | 纯逻辑:mapper / 状态机 / config 解析 | 每次 PR |
| 集成测试 | `assert_cmd` + socat PTY | CLI 行为 / 端到端数据流 | 每次 PR(Linux) |
| 模糊测试 | `cargo-fuzz` | Mapper 字节流 / 配置解析 | 夜间 CI(v0.5+) |
| 手动测试 | 真实硬件 | 每次 release 前矩阵 | release 前 |

### 手动测试矩阵(每个 release 过一遍)

- USB-Serial 转换器:CP210x, FTDI FT232, CH340
- MCU 开发板:至少一块 STM32 和一块 ESP32
- 波特率:9600, 115200, 921600, 3000000(自定义)
- 流控:无 / 硬件(RTS/CTS)
- OS:Ubuntu 最新 LTS / macOS latest / Windows 11(v0.8 开始)

### 测试辅助

```rust
// tests/common/pty.rs
pub struct PtyPair {
    pub master: PathBuf,
    pub slave: PathBuf,
    socat: std::process::Child,
}

impl PtyPair {
    pub fn new() -> Self { /* socat 启动,解析 stderr */ }
}
impl Drop for PtyPair {
    fn drop(&mut self) { self.socat.kill().ok(); }
}
```

---

## 11. 发布流程

### 自动化(首选)

用 `cargo-dist`:

```bash
cargo install cargo-dist
cargo dist init
# 编辑 dist-workspace.toml,选目标平台
git add . && git commit -m "chore: setup cargo-dist"
```

workflow 触发:tag `v*` → 构建多平台二进制 → GitHub Release → crates.io publish

### 手动兜底

```bash
# 1. 确认所有 crate 版本号一致
grep -r 'version = "' crates/*/Cargo.toml

# 2. 更新 CHANGELOG
git cliff --tag v0.X.0 > CHANGELOG.md

# 3. 打 tag
git tag -a v0.X.0 -m "Release v0.X.0"
git push origin v0.X.0

# 4. 发布(顺序:依赖 → 被依赖)
cargo publish -p rtcom-core
cargo publish -p rtcom-config
cargo publish -p rtcom-log
cargo publish -p rtcom-cli
```

### 分发渠道时间表

| 渠道 | 什么时候上 |
|---|---|
| crates.io | v0.1 |
| GitHub Releases(二进制) | v0.1 |
| Homebrew tap(自建) | v0.2 |
| AUR | v0.3 |
| winget | v0.8 |
| Debian/Fedora 官方源 | v1.0+ |

---

## 12. Claude Code 使用指南

### 把本文件作为项目上下文

把本文件保存为项目根目录的 `CLAUDE.md`,Claude Code 会自动读取作为上下文。

### 推荐的启动提示词

**首次初始化项目**:
```
阅读 CLAUDE.md,按照 §7 Issue #1 的任务列表建立 workspace 骨架。
完成后运行 cargo build 验证,然后报告给我。
```

**实现单个 Issue**:
```
参考 CLAUDE.md §7 Issue #2 的任务与验收标准,
实现 SerialDevice trait 与 serialport 后端。
遵循 §9 编码规范,为每个公开项写 rustdoc。
完成后跑 cargo clippy 和 cargo test。
```

**代码审查模式**:
```
我在 crates/rtcom-core/src/session.rs 写了 Session::run(),
对照 CLAUDE.md §3 并发模型和 §9 错误处理规范,做一次代码审查。
```

### Claude Code 工作习惯建议

1. **每次动工前让它重读相关章节**:Claude Code 跨 session 会忘,显式引用章节号能大幅减少偏差
2. **以 Issue 为原子单元**:一次对话完成一个 issue,完成后 commit,开新对话做下一个
3. **Issue 完成前必跑三件套**:`cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
4. **让 Claude Code 自己写测试**:在 prompt 里明确"包括单元测试覆盖 X Y Z"
5. **PR 描述让 Claude Code 起草**:给它 issue 链接和 commit 列表,让它按 PR 模板写

### 本项目 Claude Code 相关的注意事项

- **不要让 Claude Code 自作主张引入新依赖**:依赖引入需要 ADR 记录,在 prompt 里明确
- **不要让 Claude Code 改架构**:§3 和 §4 是决策文档,改动需要先讨论
- **允许 Claude Code 自由发挥的范围**:实现细节、测试用例、文档措辞、错误消息
- **需要人工决策的**:新 crate 拆分、新依赖、公开 API 签名变更、破坏性变更

### 推荐的辅助文件

在项目中还可以创建:

- `.clauderc` 或 `.claude/config`:项目级 Claude Code 配置
- `docs/adr/`:架构决策记录(每条重大技术决策一个文件)
- `docs/issues/v0.1/`:把本文件 §7 每个 issue 拆成独立 md 文件,便于 Claude Code 精确定位

---

## 附录 A:有用的命令速查

```bash
# 创建虚拟串口对(开发必备)
socat -d -d PTY,raw,echo=0 PTY,raw,echo=0

# 查看串口配置
stty -F /dev/ttyUSB0 -a

# 快速发字节到串口
echo -ne '\x01\x02\x03' > /dev/ttyUSB0

# 快速读串口
cat /dev/ttyUSB0 | xxd

# 列 USB 串口设备
ls -la /dev/serial/by-id/

# udev 规则调试
udevadm monitor --udev --subsystem-match=tty

# 构建与检查
cargo build --workspace
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features --open

# 发布前检查
cargo publish --dry-run -p rtcom-core
```

## 附录 B:参考资料

- [tio 源码](https://github.com/tio/tio) — 主要对标
- [picocom 源码](https://github.com/npat-efault/picocom) — 精神源头
- [serialport-rs 文档](https://docs.rs/serialport/)
- [tokio 异步书](https://tokio.rs/tokio/tutorial)
- [Rust API 设计指南](https://rust-lang.github.io/api-guidelines/)
- [Keep a Changelog](https://keepachangelog.com/)
- [Conventional Commits](https://www.conventionalcommits.org/)

---

_本文档由 Claude 协助起草,TrekMax 项目所有。最后更新:2026-04-17_
