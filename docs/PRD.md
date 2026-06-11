# 📄 VoxInk — 产品需求文档 (PRD) — AI Agent 可执行版

> **声落成墨，让 AI 提示词快人一步。**
> *Speak your prompts, ink your thoughts.*

---

## 🤖 AI Agent 执行指南（请先阅读本节）

本文档既是产品需求文档，也是 AI 编程助手（如 Claude Code、Cursor、Copilot 等）的**可执行开发指南**。

### 如何使用本文档

1. **按里程碑顺序执行**：每个 Milestone 是独立的开发单元，完成一个再进入下一个
2. **先读技术约束**：每个 Milestone 开头会列出前置依赖、关键约束和避坑提示
3. **严格按照验收标准自检**：每个 Milestone 末尾有可勾选的验收清单，Agent 必须在所有项通过后才能标记完成
4. **遵循命名和目录规范**：附录 A 的项目目录结构是强制约定，不可随意偏离
5. **每个 Milestone 结束时停手**：等待人工验证通过后再进入下一阶段，不要一次性写完全部代码

### AI Agent 行为规范

- **一个 Milestone 一个 PR**：每个 Milestone 的所有变更应作为一个独立提交单元
- **先编译通过再写新代码**：每完成一个 Rust 源文件后运行 `cargo check`，确保编译无错误
- **Clippy 零 Warning**：`cargo clippy -- -D warnings` 必须通过
- **不要过度设计**：实现 PRD 中描述的功能即可，不要添加未被要求的功能
- **先读相关代码再修改**：修改文件前务必先使用 Read 工具确认最新内容
- **遇到阻塞必须停下来**：如果依赖的 crate 版本不兼容、API 行为与文档不符、或需要人工决策（如 API Key 申请），必须明确报告并等待指示

### 文档中使用的标记说明

| 标记 | 含义 |
|------|------|
| 🤖 **Agent 任务** | 需要 AI Agent 直接执行的开发任务 |
| ⚠️ **避坑提示** | 已知的技术难点或容易出错的地方，Agent 应特别注意 |
| 🛑 **Agent 检查点** | Agent 必须在此处停下来自检并汇报结果 |
| 🔧 **人工操作** | 需要人工完成的操作（如申请 API Key、下载模型等） |
| 📦 **依赖项** | 需要添加到 `Cargo.toml` 的 crate |

---

## 目录

1. [项目概述](#1-项目概述)
2. [系统架构设计](#2-系统架构设计)
3. [详细功能需求](#3-详细功能需求)
4. [非功能性需求](#4-非功能性需求)
5. [UI/UX 设计规范](#5-uiux-设计规范)
6. [ASR 后端插件架构](#6-asr-后端插件架构)
7. [配置与安全管理](#7-配置与安全管理)
8. [开发里程碑（Agent 执行单元）](#8-开发里程碑agent-执行单元)
9. [测试策略](#9-测试策略)
10. [打包与分发](#10-打包与分发)
11. [关键技术难点与解决方案](#11-关键技术难点与解决方案)
12. [术语表](#12-术语表)
13. [附录](#13-附录)

---

## 1. 项目概述

### 1.1 项目背景

在日常使用大语言模型（LLM）时，复杂的提示词（Prompt）往往需要耗费大量的键盘输入与修改时间。**VoxInk** 是一款专为大模型高频使用者设计的开源、轻量级桌面语音提示词辅助工具，帮助用户将脑海中的灵感快速"落笔成墨"，无缝输出至大模型对话框中。

### 1.2 产品定位

- **目标用户**：LLM 高频使用者（开发者、研究员、作家、产品经理等），需要频繁输入复杂提示词的用户
- **核心价值**：以语音替代键盘输入，大幅降低提示词编写的时间成本与认知负担
- **产品形态**：桌面常驻助手应用（Windows / macOS / Linux），系统托盘常驻，随叫随到

### 1.3 核心设计原则

| 原则 | 说明 |
|------|------|
| **轻量优先** | 极低的系统资源占用（内存 < 100MB 空闲），启动迅速（< 1s） |
| **隐私优先** | 敏感数据（API Key）本地加密存储；支持纯本地 ASR，音频不上传云端 |
| **插件化可扩展** | ASR 后端采用 trait 抽象，支持多种 ASR 服务商及本地引擎自由切换 |
| **渐进式体验** | 默认开箱即用，高级功能通过设置逐步发现，降低初次使用门槛 |
| **跨平台一致** | Windows / macOS / Linux 三平台核心体验一致，平台差异妥善处理 |

### 1.4 核心技术栈

| 层级 | 技术选型 | 版本要求 | 说明 |
|------|---------|---------|------|
| **GUI 框架** | GPUI (Zed 团队) | latest git | GPU 加速的高性能 Rust UI 框架 |
| **核心语言** | Rust | Edition 2024, MSRV 1.80+ | 零成本抽象、内存安全、高性能 |
| **音频采集** | `cpal` | 0.15+ | 跨平台音频 I/O 库 |
| **音频处理** | `hound` / `rubato` | 3.5+ / 0.15+ | WAV 文件读写 + 高质量音频重采样 |
| **异步运行时** | `tokio` | 1.x (multi-threaded) | 异步 I/O、并发任务调度 |
| **HTTP 客户端** | `reqwest` | 0.12+ | 异步 HTTP 请求，TLS 支持 |
| **WebSocket** | `tokio-tungstenite` | 0.24+ | 异步 WebSocket（实时 ASR） |
| **系统托盘** | `tray-icon` | 0.19+ | 跨平台托盘图标 |
| **开机自启** | `auto-launch` | 0.6+ | 跨平台自启动管理 |
| **剪贴板** | `arboard` | 3.4+ | 跨平台系统剪贴板访问 |
| **配置存储** | `serde` + `serde_json` + `directories` | 1.x / 1.x / 6+ | 类型安全的 JSON 配置持久化 |
| **加密存储** | `ring` 或 `aead` | 0.17+ | API Key 等敏感字段的 AES-256-GCM 加密 |
| **本地 ASR** | `qwen-asr` | latest git | CPU-only、纯 Rust 实现的 Qwen3-ASR 本地语音识别引擎 |
| **日志** | `tracing` + `tracing-subscriber` | 0.1+ / 0.3+ | 结构化日志，支持文件持久化 |
| **UI 组件** | `gpui-component` | latest git | GPUI 生态组件库 |
| **数据库** | `rusqlite` (bundled feature) | 0.31+ | SQLite 绑定用于历史记录 |
| **日期时间** | `chrono` | 0.4+ | 时间戳处理 |

---

## 2. 系统架构设计

### 2.1 整体架构（四层模型）

```
┌──────────────────────────────────────────────────────────┐
│                     Presentation Layer                    │
│  ┌──────────┐  ┌───────────┐  ┌────────┐  ┌──────────┐  │
│  │  Window   │  │  System   │  │Settings│  │  Hotkey   │  │
│  │  (GPUI)   │  │  Tray     │  │ Dialog │  │  Handler  │  │
│  └─────┬─────┘  └─────┬─────┘  └───┬────┘  └────┬─────┘  │
├────────┼──────────────┼────────────┼─────────────┼───────┤
│        │         Application Layer      │          │       │
│  ┌─────┴─────────┐  ┌──────────────────┴──────────┴──┐   │
│  │  Audio Capture │  │       State Manager             │   │
│  │  Controller    │  │  (AppState, RecState, Config)   │   │
│  └─────┬──────────┘  └──────────────────┬─────────────┘   │
├────────┼───────────────────────────────┼──────────────────┤
│        │         Service Layer         │                   │
│  ┌─────┴──────────┐  ┌────────────────┴──────────────┐   │
│  │  ASR Pipeline   │  │    Clipboard & Text Service    │   │
│  │  (AudioBuffer   │  │    (Copy, History, Session)    │   │
│  │   → Resample    │  └───────────────────────────────┘   │
│  │   → Encoder     │                                      │
│  │   → Backend)    │                                      │
│  └─────┬──────────┘                                      │
├────────┼──────────────────────────────────────────────────┤
│        │          Infrastructure Layer                     │
│  ┌─────┴──────────┐  ┌──────────┐  ┌──────────────────┐  │
│  │  ASR Backends   │  │  Config   │  │   Logging &      │  │
│  │  (Plugin Trait) │  │  Store    │  │   Diagnostics     │  │
│  └────────────────┘  └──────────┘  └──────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

### 2.2 数据流架构

```
Microphone ──[PCM]──▶ Audio Capture ──[f32 samples]──▶ Ring Buffer
                                                           │
                                          ┌─────────────────┤
                                          ▼                 ▼
                              [Offline Mode]        [Streaming Mode]
                                    │                      │
                              Temp WAV file          Resample Chunk
                                    │                 (16kHz, mono)
                                    │                      │
                                    ▼                 ┌────┴────┐
                              Upload to ASR     WebSocket Send ◀─┐
                              (HTTP POST)            │            │
                                    │            WS Recv          │
                                    ▼                 │            │
                              ┌──────────┐      ┌─────┴─────┐     │
                              │ Transcript│      │ Partial   │─────┘
                              │ Response  │      │ Result    │  (more chunks)
                              └─────┬─────┘      └─────┬─────┘
                                    │                  │
                                    ▼                  ▼
                              ┌────────────────────────────┐
                              │    GPUI State Update        │
                              │    (cx.spawn() / mpsc)      │
                              └───────────┬────────────────┘
                                          │
                                          ▼
                              ┌────────────────────────────┐
                              │    Text Editor (Editable)   │
                              └───────────┬────────────────┘
                                          │
                              ┌───────────┴────────────────┐
                              │  Copy to Clipboard / Manual│
                              │      Edit by User          │
                              └────────────────────────────┘
```

### 2.3 线程模型（关键架构约束）

```
┌────────────────────────────────────────────────────────────┐
│                     Main Thread (GPUI)                       │
│  - Window rendering & event loop                             │
│  - UI state mutations (via cx.spawn() callbacks)             │
│  - MUST NOT block (> 16ms target for 60fps)                 │
└──────────┬──────────────────────────────────────────────────┘
           │ cx.spawn() / mpsc::channel
           ▼
┌────────────────────────────────────────────────────────────┐
│                   Tokio Runtime Threads                      │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │ Audio Thread │  │ Network      │  │ ASR Background   │   │
│  │ (cpal I/O)   │  │ (reqwest/WS) │  │ (local inference)│   │
│  └─────────────┘  └──────────────┘  └──────────────────┘   │
│  - 音频采集回调（高优先级）                                    │
│  - HTTP/WS 网络 I/O                                          │
│  - 本地 ASR 推理（CPU 密集型）                                 │
└────────────────────────────────────────────────────────────┘
```

⚠️ **关键约束（Agent 必须遵守）**：

1. 音频采集线程使用 `cpal` 的专用 I/O 回调，**必须**是非阻塞的。回调中**仅做**数据拷贝到环形缓冲区（`ringbuf`），**不做任何重采样、网络操作或文件 I/O**
2. 所有耗时操作（重采样、编码、网络发送、本地推理）在 Tokio 工作线程中执行
3. UI 更新统一通过 `cx.spawn()` 或单消费者 MPSC Channel 投递回主线程
4. 任何情况下都不能在 GPUI 主线程上调用 `.await`（会导致死锁）

---

## 3. 详细功能需求

### 3.1 音频录制控制

#### 3.1.1 设备管理

- **默认设备自动选择**：应用启动时自动探测并选择系统默认录音设备。
- **设备热插拔**：监听系统音频设备变更事件，默认设备切换时自动跟随（无需重启应用）。
- **设备列表展示**（v1.1+）：设置面板中可查看可用录音设备列表，允许用户手动选择非默认设备。
- **权限处理**：若系统未授予麦克风权限（macOS 需 Info.plist 声明 `NSMicrophoneUsageDescription`），应用应弹出友好提示，引导用户开启权限。

#### 3.1.2 录音操作

- **开始录音**：点击主界面录音按钮或触发全局快捷键。
- **停止录音**：再次点击录音按钮 / 触发快捷键 / 点击系统托盘菜单"停止录音"。
- **按钮状态机**：

```
  ┌──────────┐   点击开始    ┌──────────┐   点击停止    ┌──────────┐
  │  IDLE    │──────────────▶│RECORDING │──────────────▶│PROCESSING│
  │  (绿色)   │◀──────────────│  (红色)   │              │  (橙色)   │
  └──────────┘   取消/错误    └──────────┘              └─────┬─────┘
       ▲                                                     │
       └─────────────────────────────────────────────────────┘
                         处理完成 / 错误
```

- **最长录音时长**：单次录音最长 600 秒（10 分钟），超时自动停止并提示。此值可在配置中调整。
- **静音检测**（v1.2+）：可选启用 VAD（Voice Activity Detection），检测到连续静音 N 秒后自动停止录音。

#### 3.1.3 录音状态反馈

- **视觉反馈**：
  - 录音按钮变红，外圈脉冲动画（`pulsing red ring`）
  - 状态栏显示实时录音时长格式 `MM:SS`
  - 任务栏/托盘图标叠加录音中标识（红色圆点）
- **音频电平指示**（v1.1+）：实时显示麦克风输入音量电平条，帮助用户确认麦克风工作正常。

### 3.2 语音转录模式

VoxInk 支持两种转录处理模式，用户可在主界面实时切换：

> **设计说明**：转录模式（实时/离线）与 ASR 后端（云端/本地）是两个正交维度。
> - **转录模式**决定音频数据的处理方式：实时流式发送 vs 录音结束后整段上传。
> - **ASR 后端**决定由谁来执行识别：云服务（如阿里云百炼）还是本地引擎（qwen-asr）。
> - 本地 qwen-asr 当前仅支持离线模式；若用户选择本地后端，实时模式自动不可用。
> - 后面板中"默认转录模式"设置项的值仅有两项：`streaming` / `offline`。

#### 3.2.1 实时流式转录 (Streaming ASR)

通过 WebSocket 连接 ASR 服务，边说边识别，文本即时呈现。

- **流程**：
  1. 用户点击"开始录音"
  2. 建立与 ASR 后端的 WebSocket 连接
  3. 音频采集线程持续将数据写入环形缓冲区
  4. 后台 Tokio 任务从缓冲区读取 chunk → 重采样至 16kHz/16bit/单声道 PCM → 通过 WebSocket 发送
  5. 接收服务端实时返回的中间识别结果（`partial_result`）
  6. 通过 `cx.spawn()` 增量更新到文本编辑器中
  7. 用户停止录音 → 发送结束信号 → 接收最终结果（`final_result`）→ 替换对应段落的中间结果
- **增量更新策略**：
  - 使用**句子级稳定标记**（`is_final` / `sentence_end`）判断某段文本是否可以固化为最终结果
  - 未稳定的尾部文本以斜体/浅色呈现（视觉区分），稳定后转为正常样式
  - 用户可在识别过程中随时手动编辑已稳定的文本
- **异常处理**：
  - WebSocket 断开：自动重连（最多 3 次，指数退避），重连期间继续本地录音（数据不丢失）
  - 重连失败：回退为离线模式，录音停止后将缓冲区数据作为离线请求发送
  - Token 过期/鉴权失败：立即停止录音，弹出重新配置 API Key 的提示

#### 3.2.2 离线整段转录 (Offline ASR)

录音时仅做本地缓存，结束后一次性上传整段音频进行高精度识别。

- **流程**：
  1. 用户点击"开始录音"
  2. 音频数据仅写入本地临时 WAV 文件（路径：`{OS temp dir}/voxink_recording_{timestamp}.wav`）
  3. 用户点击"停止录音" → 关闭 WAV 文件 → UI 显示"正在识别中..."和 Loading 动画
  4. 通过 HTTP POST 上传 WAV 文件至 ASR 后端
  5. 接收完整转写结果 → 更新到文本编辑器
  6. 删除临时 WAV 文件（或根据设置保留以便复查）
- **上传进度**：大文件上传显示进度条（百分比或 MB/总量）。
- **超时处理**：HTTP 请求超时时间 120 秒（可配置），超时后提示用户"转写超时，请检查网络或缩短录音时长"。

#### 3.2.3 本地 ASR 后端 (qwen-asr) — 完全离线运行

**这不是一种独立的转录模式，而是"离线转录"模式的一种后端实现。** 当用户在设置中选择 ASR 后端为"本地 qwen-asr"时，离线转录流程完全在本地设备上运行 ASR 推理，音频数据**不离开本机**。

##### 引擎选型与约束

- **唯一引擎**：`qwen-asr`（https://github.com/huanglizhuo/QwenASR）
  - 基于 **Qwen3-ASR** 模型的 CPU-only、纯 Rust 实现的语音识别引擎
  - 无需 GPU、无需 FFI/C 依赖，完全由 Rust 生态承载
  - 中文识别精度优秀，支持多语言混合识别

##### 技术约束

| 约束项 | 要求 | 说明 |
|--------|------|------|
| **推理设备** | CPU-only | 不支持 GPU 加速，设计上即纯 CPU 推理 |
| **运行时依赖** | 纯 Rust | 无 C/C++ FFI 依赖 |
| **模型格式** | Qwen3-ASR 原始权重 | 需通过 qwen-asr 提供的转换工具转为推理格式 |
| **模型下载** | 首次启动引导下载 | 模型文件存储在 `{app data dir}/models/qwen-asr/` |
| **音频输入** | 16kHz, 16-bit, 单声道 PCM | 与现有音频采集管线一致 |
| **推理延迟** | 目标 < 3× 实时 | 取决于 CPU 性能 |
| **内存占用** | 模型加载后额外 < 500MB | 取决于模型规格 |
| **线程安全** | `Send + Sync` | 必须满足 `AsrBackend` trait 的 trait bound |

##### 模型规格

| 规格 | 文件大小 | 推理速度 | WER | 推荐场景 |
|------|---------|---------|-----|---------|
| base | ~200MB | 最快 | ~10-15% | 日常快速转写 |
| small | ~500MB | 中等 | ~8-12% | 需要较高精度 |
| medium | ~1GB | 最慢 | ~5-10% | 专业场景，高精度需求 |

### 3.3 文本编辑与交互

#### 3.3.1 文本编辑器

- **多行编辑**：支持任意文本输入、修改、删除、选择、复制、粘贴
- **Undo/Redo**：支持撤销/重做操作（Ctrl+Z / Ctrl+Shift+Z 或 Cmd+Z / Cmd+Shift+Z）
- **自动追加 vs 覆盖**：
  - 实时模式：识别结果增量追加（后续句子追加到已有文本末尾）
  - 此行为可在设置中切换为"每次录音覆盖已有文本"
  - 离线模式：识别完成 → **追加**到已有文本末尾（保留之前的内容）
- **字数统计**：底部状态栏实时显示当前文本字数/字符数

#### 3.3.2 一键复制

- **复制按钮**：位于编辑器右下角，点击后将全部文本写入系统剪贴板
- **复制反馈**：
  - 按钮短暂变为"✓ 已复制"（1.5 秒后恢复）
  - 同时触发轻量 Toast 提示"已复制到剪贴板"
- **自动复制**（可选设置）：转录完成后自动将新内容复制到剪贴板

#### 3.3.3 文本历史与会话管理（v1.1+）

- **历史记录**：每次转录完成后自动保存到本地历史（SQLite）
- **历史面板**：侧边栏展示历史记录列表，包含时间戳、转录模式、文本预览
- **搜索**：支持全文搜索历史记录
- **会话概念**：用户可创建/切换会话，每个会话维护独立的文本上下文
- **数据保留**：默认保留最近 30 天历史，可配置保留策略

### 3.4 全局快捷键

#### 3.4.1 默认快捷键

| 功能 | Windows/Linux | macOS |
|------|---------------|-------|
| 开始/停止录音 | `Ctrl+Alt+Space` | `Cmd+Option+Space` |
| 唤起/隐藏主窗口 | `Ctrl+Alt+V` | `Cmd+Option+V` |
| 一键复制并粘贴到前台应用 | `Ctrl+Alt+B` | `Cmd+Option+B` |

#### 3.4.2 快捷键要求

- 所有快捷键支持用户在设置中自定义
- 全局热键注册需处理与其他应用的冲突（检测冲突并提示用户更换）
- 快捷键在托盘后台运行时依然生效（进程全局监听）

### 3.5 系统级特性

#### 3.5.1 开机自启动

- 设置面板中提供开关选项
- 实现方式：
  - Windows：注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
  - macOS：LaunchAgent plist (`~/Library/LaunchAgents/`)
  - Linux：XDG Autostart `.desktop` 文件 (`~/.config/autostart/`)

#### 3.5.2 系统托盘

- **托盘图标**：
  - 默认状态：应用图标（彩色）
  - 录音中：图标叠加红色圆点徽标
  - 识别中：图标叠加橙色旋转指示器
- **左键单击**：唤起/隐藏主窗口（toggle）
- **右键菜单**：打开主界面、开始/停止录音、最近转录（子菜单）、设置、关于、退出
- **托盘 Tooltip**：显示应用名称 + 当前状态

#### 3.5.3 窗口管理

- **关闭按钮行为**：点击窗口关闭按钮（X）不退出应用，而是隐藏到托盘
- **最小化按钮**：最小化到任务栏，同时保留托盘图标
- **窗口置顶**（可选）：设置中可选择是否将主窗口置顶
- **窗口记忆**：记住上次窗口位置和大小，下次启动时恢复

---

## 4. 非功能性需求

### 4.1 性能指标

| 指标 | 目标值 | 测量方法 |
|------|--------|----------|
| 应用冷启动时间 | < 1.5s | 从进程启动到主窗口首次渲染完成 |
| 托盘唤起延迟 | < 200ms | 从点击托盘图标到窗口完全显示 |
| 内存占用（空闲） | < 80MB | 应用启动后静置 1 分钟，无录音 |
| 内存占用（录音中） | < 150MB | 实时 ASR 模式，1 分钟录音 |
| CPU 占用（空闲） | < 1% | 同上 |
| CPU 占用（本地 ASR） | < 50% 多核 | qwen-asr base 模型，现代 4 核 CPU |
| 音频延迟（采集到发送） | < 100ms | 实时模式，从麦克风采集到 WebSocket 发送 |
| 本地 WAV 文件大小 | ~5.5MB/分钟 | 16kHz, 16-bit, mono PCM |

### 4.2 兼容性矩阵

| 平台 | 最低版本 | 架构 |
|------|---------|------|
| Windows | Windows 10 21H2+ | x86_64, aarch64 |
| macOS | macOS 13 Ventura+ | x86_64, aarch64 (Apple Silicon) |
| Linux | Ubuntu 22.04+ / Fedora 38+ | x86_64 |

### 4.3 安全性

| 安全项 | 措施 |
|--------|------|
| API Key 存储 | AES-256-GCM 加密，密钥派生自机器唯一标识（machine-id + 随机 salt） |
| 网络通信 | 所有 ASR API 调用强制 HTTPS/WSS（TLS 1.2+） |
| 本地数据 | 录音临时文件和转写历史存储在应用私有数据目录 |
| 依赖审计 | 定期 `cargo audit` 检查依赖漏洞 |
| 日志脱敏 | 日志输出前过滤 API Key、Token 等敏感字段（替换为 `****`） |

### 4.4 可访问性（v1.2+）

- 支持系统字体缩放
- 支持高对比度模式（跟随系统设置）
- 主要功能可通过键盘完成操作（Tab 导航 + Enter 确认）

---

## 5. UI/UX 设计规范

### 5.1 窗口规格

| 属性 | 值 |
|------|-----|
| 默认窗口尺寸 | 480 × 600 px |
| 最小窗口尺寸 | 360 × 400 px |
| 最大窗口尺寸 | 800 × 1200 px |
| 字体 | 系统默认（中文：Microsoft YaHei / PingFang SC / Noto Sans CJK） |
| 字体大小 | 正文 14px，标题 18px，状态栏 12px |
| 圆角 | 窗口 12px，按钮 8px，输入框 6px |

### 5.2 主界面布局

```
┌─────────────────────────────────────┐
│  🎙 VoxInk              ⚙ 设置 │  ← Header
├─────────────────────────────────────┤
│                                     │
│        ┌─────────────────┐          │
│        │                 │          │
│        │   🎤 开始录音   │          │  ← 录音按钮（大，主操作区）
│        │                 │          │
│        └─────────────────┘          │
│                                     │
│   转录模式: [实时 ═══ 离线]          │  ← Streaming/Offline 切换
│                                     │
│   状态: ● 录音中  00:42             │  ← 状态指示
│   [████████████░░░░] 音量条         │  ← 音频电平（v1.1）
│                                     │
├─────────────────────────────────────┤
│                                     │
│   ┌─────────────────────────────┐   │
│   │                             │   │
│   │   这里是转写文本...          │   │
│   │   用户可以手动编辑...        │   │  ← 文本编辑区（占据主要空间）
│   │                             │   │
│   │                             │   │
│   └─────────────────────────────┘   │
│                                     │
├─────────────────────────────────────┤
│  字数: 128 │          📋 一键复制  │  ← Footer
└─────────────────────────────────────┘
```

### 5.3 主题与色彩

- **默认主题**：浅色主题（Light Mode）
- v1.1+ 支持深色主题（Dark Mode），跟随系统设置或手动切换
- **强调色**：`#4A90D9`（蓝色系）
- **录音中色**：`#E74C3C`（红色）
- **识别中色**：`#F39C12`（橙色）
- **就绪色**：`#27AE60`（绿色）

### 5.4 设置面板结构

设置面板以独立模态窗口或侧边抽屉形式展示，包含以下分区：

1. **ASR 服务配置**：后端选择、API Key、Endpoint URL、连接测试按钮
2. **录音设置**：默认转录模式、自动复制、提示音、最长录音时长
3. **通用设置**：开机自启、启动最小化、窗口置顶、主题、语言
4. **快捷键设置**（v1.1+）：当前绑定显示 + 重新录制
5. **关于**：版本号、协议、导出诊断、检查更新

---

## 6. ASR 后端插件架构

### 6.1 设计目标

VoxInk 的 ASR 能力通过 **Trait 抽象** 实现后端可插拔，支持：

- 云服务 ASR（阿里云百炼、讯飞、Azure Speech 等）
- 本地引擎 ASR（qwen-asr，CPU-only 纯 Rust 实现）
- 自定义 ASR（用户自建服务，配置 WebSocket URL）

### 6.2 核心 Trait 定义

以下接口是 ASR 子系统的核心契约，Agent 在实现时必须严格遵循：

```rust
/// ASR 后端统一接口
/// 
/// 所有 ASR 后端（云服务、本地引擎、自定义服务）必须实现此 trait。
/// 
/// Trait 要求 Send + Sync（可在线程间安全传递）+
/// 'static（可放入 tokio::spawn）。
#[async_trait]
pub trait AsrBackend: Send + Sync + 'static {
    /// 后端唯一标识符
    /// 示例: "aliyun_bailian_streaming", "qwen_asr_local"
    fn backend_id(&self) -> &str;

    /// 用户可见的后端名称
    /// 示例: "阿里云百炼（实时）", "本地 qwen-asr"
    fn display_name(&self) -> &str;

    /// 本后端是否支持实时流式识别
    fn supports_streaming(&self) -> bool;

    /// 本后端是否支持离线整段识别
    fn supports_offline(&self) -> bool;

    /// 验证配置是否有效（如测试 API Key 连通性）
    /// 返回 Ok(()) 表示验证通过
    /// 返回 Err(AsrError) 包含具体错误信息
    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError>;

    /// 实时流式识别
    /// 
    /// # 参数
    /// - `config`: ASR 配置（API Key、Endpoint 等）
    /// - `audio_rx`: 音频 chunk 接收通道
    ///   - 每个 chunk: 16kHz, 16-bit, 单声道 PCM bytes
    ///   - 通道关闭表示录音结束，应发送结束信号并等待最终结果
    /// - `result_tx`: 识别结果发送通道
    ///   - 实时发送 partial/final 结果
    ///   - 发送完 final 结果后可以 drop
    async fn transcribe_streaming(
        &self,
        config: &AsrConfig,
        audio_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        result_tx: tokio::sync::mpsc::Sender<StreamingResult>,
    ) -> Result<(), AsrError>;

    /// 离线整段识别
    /// 
    /// # 参数
    /// - `config`: ASR 配置
    /// - `audio_data`: 完整的 WAV 文件字节数据
    /// 
    /// # 返回
    /// - `Ok(String)`: 完整的转写文本
    async fn transcribe_offline(
        &self,
        config: &AsrConfig,
        audio_data: Vec<u8>,
    ) -> Result<String, AsrError>;
}

/// 流式识别中间结果
#[derive(Debug, Clone)]
pub struct StreamingResult {
    /// 本次增量文本（仅新增部分）
    pub delta_text: String,
    /// 是否为句子结束的最终结果
    /// true: 文本已稳定，应转为正常样式
    /// false: 文本为中间结果，应以斜体/浅色显示
    pub is_final: bool,
    /// 时间戳
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
```

### 6.3 错误类型定义

```rust
/// ASR 相关错误类型
/// 
/// Agent 注意：所有后端实现必须使用此错误类型，
/// 不得使用 anyhow 或字符串错误。
#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("网络连接失败: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("WebSocket 连接失败: {0}")]
    WebSocketError(String),

    #[error("API 鉴权失败，请检查 API Key")]
    AuthError,

    #[error("API 配额已用尽: {0}")]
    QuotaExceeded(String),

    #[error("音频格式不支持: {0}")]
    UnsupportedFormat(String),

    #[error("转写超时")]
    Timeout,

    #[error("未识别到语音内容")]
    EmptyResult,

    #[error("本地模型未找到: {0}")]
    ModelNotFound(String),

    #[error("本地推理失败: {0}")]
    InferenceError(String),

    #[error("配置无效: {0}")]
    InvalidConfig(String),

    #[error("录音数据为空")]
    EmptyAudio,

    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),
}
```

### 6.4 后端注册表

```rust
/// ASR 后端工厂注册表
/// 
/// 使用工厂模式，每个后端通过闭包创建实例。
/// 运行时根据配置文件中的 backend_id 获取对应后端。
pub struct AsrBackendRegistry {
    backends: HashMap<String, Box<dyn Fn() -> Box<dyn AsrBackend> + Send + Sync>>,
}
```

内置后端注册表：

| 后端 ID | 名称 | 类型 | 支持流式 | 支持离线 | 默认启用 |
|---------|------|------|---------|---------|---------|
| `aliyun_bailian_streaming` | 阿里云百炼（实时） | 云服务 | ✅ | ❌ | ✅ |
| `aliyun_bailian_offline` | 阿里云百炼（离线） | 云服务 | ❌ | ✅ | ✅ |
| `generic_ws` | 通用 WebSocket | 云服务 | ✅ | ❌ | ✅ |
| `qwen_asr_local` | 本地 qwen-asr | 本地 | ❌ | ✅ | ❌（需下载模型） |

### 6.5 配置结构

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    /// 当前使用的后端 ID
    pub backend_id: String,
    /// API Key（云服务）
    pub api_key: String,
    /// API Endpoint
    pub api_endpoint: String,
    /// 本地引擎模型路径（qwen-asr）
    pub local_model_path: Option<String>,
    /// qwen-asr 模型规格（"base", "small", "medium"）
    pub local_model_size: Option<String>,
    /// 语言代码（"zh", "en", "auto"）
    pub language: String,
}
```

---

## 7. 配置与安全管理

### 7.1 配置文件位置

| 平台 | 路径 |
|------|------|
| Windows | `%APPDATA%\VoxInk\config.json` |
| macOS | `~/Library/Application Support/VoxInk/config.json` |
| Linux | `~/.config/VoxInk/config.json` |

### 7.2 配置文件结构

```json
{
  "version": 1,
  "general": {
    "language": "zh-CN",
    "theme": "system",
    "launch_at_startup": false,
    "start_minimized": true,
    "window_on_top": false,
    "audio_feedback": true
  },
  "asr": {
    "backend_id": "aliyun_bailian_streaming",
    "default_mode": "streaming",           // "streaming" | "offline"（与 TranscriptionMode 枚举对应）
    "api_endpoint": "https://dashscope.aliyuncs.com/api/v1/...",
    "language": "zh",
    "max_recording_seconds": 600,
    "local_model_size": "base"
  },
  "shortcuts": {
    "toggle_recording": "Ctrl+Alt+Space",
    "toggle_window": "Ctrl+Alt+V",
    "copy_and_paste": "Ctrl+Alt+B"
  },
  "text": {
    "auto_copy": false,
    "append_mode": true,
    "history_retention_days": 30
  },
  "window": {
    "x": null,
    "y": null,
    "width": 480,
    "height": 600
  }
}
```

### 7.3 敏感字段加密方案

- **加密算法**：AES-256-GCM
- **密钥派生**：基于机器唯一标识符（machine-id）+ 固定 salt 通过 HKDF 派生 256-bit 密钥
- **加密字段**：`asr.api_key`
- **存储格式**：`base64(nonce || ciphertext || tag)`，12 字节 nonce + 密文 + 16 字节 tag
- **安全设计**：机器绑定的加密意味着更换设备/重装系统后需重新输入 API Key。这是有意为之。

---

## 8. 开发里程碑（Agent 执行单元）

> **🤖 这是 AI Agent 的核心执行章节。每个 Milestone 是独立的开发单元，Agent 应严格按照顺序执行，完成一个再进行下一个。**

### 里程碑概览

| # | 名称 | 预计工期 | 核心交付 | 依赖 |
|---|------|---------|---------|------|
| M1 | 项目初始化与基础 UI | 3-5 天 | GPUI 窗口 + 静态布局 | — |
| M2 | 状态管理与交互 | 3-4 天 | 按钮状态机 + 剪贴板 + 配置读写 | M1 |
| M3 | 本地录音引擎 | 4-6 天 | cpal 录音 + WAV 存储 | M2 |
| M4 | 离线 ASR 对接 | 3-5 天 | HTTP 上传 + 转写结果展示 | M3 |
| M5 | 系统托盘与自启动 | 3-4 天 | 托盘集成 + 开机自启 | M2 |
| M6 | 实时 ASR 对接 | 5-7 天 | WebSocket 流式 + 增量更新 | M3 |
| M7 | ASR 后端插件化 | 4-6 天 | Trait 抽象 + 多后端支持 | M4, M6 |
| M8 | 本地 ASR 集成 | 5-8 天 | qwen-asr 推理 + 模型管理 | M7 |
| M9 | 全局快捷键 | 3-5 天 | 全局热键 + 自定义绑定 UI | M5 |
| M10 | 文本历史与会话 | 4-6 天 | SQLite 存储 + 历史面板 | M2 |
| M11 | 设置面板完善 | 3-4 天 | 完整设置 UI + 主题切换 | M5 |
| M12 | 测试、打包与发布 | 5-8 天 | CI/CD + 安装包 + 文档 | M1-M11 |

---

### 🎯 Milestone 1: 项目初始化与基础 GPUI 界面搭建

**工期**：3-5 天 | **优先级**：P0 | **依赖**：无

#### 🤖 Agent 任务清单

##### 任务 1.1: 初始化 Cargo 项目

📦 **添加到 `Cargo.toml` 的依赖**：

```toml
[package]
name = "voxink"
version = "0.1.0"
edition = "2024"

[dependencies]
# GUI 框架
gpui = { git = "https://github.com/zed-industries/zed", package = "gpui" }

# 异步运行时
tokio = { version = "1", features = ["full"] }

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# 日志
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# 错误处理
thiserror = "2"
anyhow = "1"

# 时间
chrono = { version = "0.4", features = ["serde"] }
```

##### 任务 1.2: 搭建项目目录结构

按照附录 A 创建目录结构（初期只需创建 `src/`、`docs/`、`assets/` 目录和基本文件）。

##### 任务 1.3: 实现应用入口

`src/main.rs` 中完成：
- 初始化 `tracing_subscriber`（日志输出到控制台，默认 INFO 级别）
- 创建 Tokio runtime
- 启动 GPUI 应用，创建主窗口（480×600）

##### 任务 1.4: 定义基础 AppState

`src/state.rs`：

```rust
/// 录音状态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    /// 空闲，等待用户操作
    Idle,
    /// 正在录音
    Recording,
    /// 正在处理（上传转写 / 本地推理）
    Processing,
}

/// 转录处理模式（用户在主界面切换）
/// 
/// 注意：此枚举描述的是"如何处理音频数据"，而非"由谁识别"。
/// ASR 后端的云端/本地选择由 `AsrConfig.backend_id` 决定。
/// 本地后端（qwen-asr）当前仅支持 Offline 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptionMode {
    /// 实时流式：音频分帧实时发送到 ASR 后端，识别结果即时返回
    Streaming,
    /// 离线整段：录音完成后将完整音频一次性发送给 ASR 后端进行转写
    Offline,
}

/// 应用全局状态
#[derive(Debug, Clone)]
pub struct AppState {
    /// 当前录音状态
    pub recording_state: RecordingState,
    /// 用户选择的转录模式
    pub transcription_mode: TranscriptionMode,
    /// 文本编辑器中的文本内容
    pub text_content: String,
    /// 录音时长（秒），仅在 Recording / Processing 时有意义
    pub recording_duration_secs: u32,
}
```

##### 任务 1.5: 搭建 UI 布局骨架

`src/app.rs` 中实现 GPUI View：
- Header 区域：应用标题 `🎙 VoxInk` + 设置按钮（齿轮图标占位）
- 控制区域：大录音按钮（居中，显示"🎤 开始录音"）+ 模式 Toggle + 状态文本
- 文本编辑区域：多行文本输入框（使用 GPUI Editor）
- Footer 区域：字数统计（左侧）+ "📋 一键复制"按钮（右侧）

⚠️ **避坑提示**：
- GPUI 目前处于快速迭代期，API 可能变动。优先参考 GPUI 官方 examples 中的 `text_input` 和 `editor` 示例
- GPUI 的 `WindowOptions` 中设置 `window_bounds` 控制窗口大小
- 所有 UI 布局使用 GPUI 的 `div()` + `flex()` 组合，不要使用绝对定位

##### 任务 1.6: 实现按钮点击事件（占位）

- 录音按钮点击 → `tracing::info!("录音按钮被点击")`
- 复制按钮点击 → `tracing::info!("复制按钮被点击")`

#### 🛑 Agent 检查点

完成以下检查后再向用户汇报 M1 完成：

```bash
# 1. 编译通过（必须零 warning）
cargo check

# 2. Clippy 通过
cargo clippy -- -D warnings

# 3. 运行验证：主窗口能正常渲染
cargo run
```

#### 验收标准

- [ ] `cargo run` 可在 Windows/macOS/Linux 任一平台成功编译并渲染出窗口
- [ ] 窗口包含 Header、录音按钮、模式 Toggle、文本编辑区、复制按钮四个区域
- [ ] 点击录音按钮和复制按钮在控制台有对应日志输出
- [ ] TextEditor 支持键盘输入、选择、删除等基本操作
- [ ] 应用窗口可正常关闭，进程退出
- [ ] 日志正常输出到控制台（INFO 级别）

#### 关键文件

| 文件 | 说明 |
|------|------|
| `Cargo.toml` | 项目依赖配置 |
| `src/main.rs` | 应用入口点 |
| `src/app.rs` | GPUI App + View 实现 |
| `src/state.rs` | AppState + RecordingState 定义 |

---

### 🎯 Milestone 2: 状态管理与基础交互

**工期**：3-4 天 | **优先级**：P0 | **依赖**：M1

#### 🤖 Agent 任务清单

##### 任务 2.1: 实现录音按钮状态机

- IDLE → 点击 → RECORDING
- RECORDING → 点击 → PROCESSING
- PROCESSING → 完成/错误 → IDLE
- 按钮文字和颜色随状态变化：
  - IDLE: 绿色背景, "🎤 开始录音"
  - RECORDING: 红色背景 + 脉冲动画, "⏹ 停止录音"
  - PROCESSING: 橙色背景, "⏳ 处理中..."（不可点击）

##### 任务 2.2: 剪贴板集成

- 引入 `arboard` crate（3.4+）
- 实现 `copy_to_clipboard(text: &str) -> Result<(), AppError>`
- "一键复制"按钮将 TextEditor 内容写入系统剪贴板
- 复制成功后：按钮文字短暂变为 "✓ 已复制"（1.5 秒后恢复）+ Toast 提示

##### 任务 2.3: 配置管理模块

`src/config.rs`：

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoxInkConfig {
    pub version: u32,
    pub general: GeneralConfig,
    pub asr: AsrConfig,
    pub shortcuts: ShortcutConfig,
    pub text: TextConfig,
    #[serde(skip)]  // 不持久化，运行时计算
    pub config_path: PathBuf,
}

impl VoxInkConfig {
    /// 从默认路径加载配置，若文件不存在则返回默认配置
    pub fn load() -> Result<Self, ConfigError> { /* ... */ }
    /// 保存配置到文件
    pub fn save(&self) -> Result<(), ConfigError> { /* ... */ }
}
```

- 引入 `directories` crate 确定各平台配置目录
- 实现 `Default` trait ⇒ 合理的默认值
- 应用启动时自动加载配置，退出时自动保存
- 配置文件版本号管理（`version: 1`），未来升级时做迁移

##### 任务 2.4: API Key 加密存储

- 📦 依赖 `ring` crate（0.17+）用于 AES-256-GCM
- 实现 `encrypt_api_key(plaintext: &str) -> Result<String>` → 返回 base64 密文
- 实现 `decrypt_api_key(ciphertext: &str) -> Result<String>` → 返回明文
- 密钥派生：machine-id + salt → HKDF → 256-bit key
- 加密字段在 JSON 中标识为 `"encrypted": true` 便于识别

##### 任务 2.5: UI 状态联动

- `AppState` 变更自动反映到 UI（按钮颜色、状态文本）
- 使用 GPUI 的 `Model<T>` 和 `cx.notify()` 机制触发重渲染

#### 验收标准

- [ ] 录音按钮点击后颜色/文字正确切换（IDLE→RECORDING→IDLE）
- [ ] 复制按钮能将文本编辑器内容写入剪贴板，反馈正确
- [ ] 配置文件在应用启动时正确加载，退出时正确保存
- [ ] API Key 以密文形式存储，明文不出现在配置文件中
- [ ] 重启应用后配置正确恢复

---

### 🎯 Milestone 3: 本地录音引擎

**工期**：4-6 天 | **优先级**：P0 | **依赖**：M2

#### 🤖 Agent 任务清单

##### 任务 3.1: 音频设备探测

`src/audio/capture.rs`：
- 使用 `cpal` 枚举可用录音设备
- 自动选择系统默认输入设备
- 设备不可用时返回明确的错误类型，UI 显示友好提示

##### 任务 3.2: 环形缓冲区

`src/audio/buffer.rs`：
- 使用 `ringbuf` crate 实现单生产者（音频回调）单消费者（Tokio 任务）的无锁环形缓冲区
- 缓冲区大小：建议 64KB（约 2 秒 16kHz/16bit/mono 音频）
- `push(samples: &[f32])` — 音频回调中调用
- `pop() -> Vec<f32>` — Tokio 任务中调用

⚠️ **避坑提示**：音频回调中**绝对不能**做任何阻塞操作——仅调用 `ringbuf.push()`。重采样、文件写入等操作全部在 Tokio 任务中完成。

##### 任务 3.3: 音频采集

- 配置 `cpal::Stream` 参数（优先选择 16kHz / mono / f32）
- 支持开始/停止录音 → 对应 Stream 的 `play()` / `pause()` / `drop()`
- 启动时打印实际音频配置到日志（采样率、通道数、格式）

##### 任务 3.4: 重采样管线

`src/audio/resample.rs`：
- 📦 引入 `rubato` crate（0.15+）
- 实现 `Resampler` 结构体：输入任意采样率 → 输出 16kHz
- 通道转换：多声道 → 单声道（取平均 `(L+R)/2`）
- 格式转换：f32 → i16 PCM（`(sample * 32767.0) as i16`）
- Tokio 任务中从 ringbuf 读取 → 重采样 → 写入输出缓冲区

##### 任务 3.5: WAV 文件写入

`src/audio/writer.rs`：
- 📦 引入 `hound` crate（3.5+）
- 创建临时 WAV 文件：`{temp_dir}/voxink_recording_{YYYYMMDD}_{HHMMSS}.wav`
- 规格：16kHz, 16-bit, 单声道 PCM
- 录音过程中持续写入（流式写入，不是全部完成后一次性写入）

##### 任务 3.6: 录音 UI 状态

- 实时显示 `MM:SS` 格式的录制时长
- 使用 `tokio::time::interval` 每 1 秒更新一次显示
- 超过 `max_recording_seconds` 自动停止录音

#### 🛑 Agent 检查点

```bash
# 使用 Audacity 或 ffprobe 验证输出文件格式
ffprobe /tmp/voxink_recording_*.wav
# 期望输出: pcm_s16le, 16000 Hz, mono
```

#### 验收标准

- [ ] 点击"开始录音"后正确采集麦克风音频数据
- [ ] 录音过程中 UI 实时显示 MM:SS 计时
- [ ] 点击"停止录音"后生成可播放的 .wav 文件（16kHz, 16-bit, mono PCM）
- [ ] 无麦克风时应用给出明确提示，不崩溃
- [ ] 录音 60 秒无内存泄漏（内存占用稳定不持续增长）
- [ ] 超时自动停止录音并提示

---

### 🎯 Milestone 4: 离线 ASR 对接（阿里云百炼）

**工期**：3-5 天 | **优先级**：P1 | **依赖**：M3

#### 🤖 Agent 任务清单

##### 任务 4.1: HTTP 客户端封装

`src/asr/client.rs`：
- 基于 `reqwest::Client` 创建全局 HTTP 客户端
- 配置：超时 120s、自动重定向、gzip 压缩、TLS 1.2+
- 封装阿里云百炼 API 鉴权（Header: `Authorization: Bearer <api_key>`）

##### 任务 4.2: 离线 ASR API 对接

`src/asr/backends/bailian_offline.rs`：
- 实现 `BailianOfflineBackend` 结构体
- `transcribe_offline()` 方法：
  1. 读取 WAV 文件为 `Vec<u8>`
  2. 构造 multipart/form-data 请求
  3. POST 到百炼离线 ASR endpoint
  4. 解析 JSON 响应，提取转写文本
- 接口地址：`POST https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription`

##### 任务 4.3: 异步任务管理

- 使用 `tokio::spawn` 在后台执行上传+识别
- 使用 `tokio::sync::oneshot` 将结果传回 UI
- `cx.spawn()` 在 GPUI 上下文中更新 UI

##### 任务 4.4: Loading 状态

- 上传期间 UI 显示 "正在转录..." + 旋转动画
- 上传进度指示（文件较大时显示百分比）

##### 任务 4.5: 错误处理

对以下错误场景提供用户友好的提示：
- 网络超时 → "网络连接超时，请检查网络"
- API Key 无效 → "API Key 无效，请检查设置"
- API 配额用尽 → "API 配额已用尽，请升级套餐"
- 返回空结果 → "未识别到语音内容，请重试"

#### 🔧 人工操作

Agent 在开始 M4 前需确认：
1. 用户已注册阿里云百炼并获取 API Key
2. API Key 已手动填入配置文件 `asr.api_key` 字段
3. Agent 应使用解密后的 API Key 发送请求，而非加密密文

#### 验收标准

- [ ] 录音结束后自动触发上传 + 转写
- [ ] 转写期间 UI 正确显示 Loading 状态
- [ ] 转写完成后文本正确显示在编辑器中（追加模式）
- [ ] API Key 无效时显示明确错误提示
- [ ] 网络断开时不会崩溃，显示错误信息并保留音频文件
- [ ] 5 分钟录音（约 1.6MB WAV）在 30 秒内完成上传+识别

---

### 🎯 Milestone 5: 系统托盘与开机自启动

**工期**：3-4 天 | **优先级**：P1 | **依赖**：M2

#### 🤖 Agent 任务清单

##### 任务 5.1: 系统托盘集成

`src/tray.rs`：
- 📦 引入 `tray-icon` crate（0.19+）
- 创建托盘图标（应用 logo 的 16×16、32×32 版本）
- 左键单击：toggle 主窗口显示/隐藏
- 右键菜单：打开主界面、开始/停止录音、设置、退出
- 托盘 Tooltip：显示 "VoxInk — 就绪" / "VoxInk — 录音中 00:15"

##### 任务 5.2: 窗口生命周期管理

- 窗口关闭按钮（X）行为改为隐藏到托盘（不是退出进程）
- 托盘"退出"选项完全退出应用（`std::process::exit(0)`）
- `assets/` 目录下放置应用图标和托盘图标

⚠️ **避坑提示**：
- macOS 上 GPUI 使用 `NSApplication` 事件循环，`tray-icon` 也依赖 Cocoa 主线程。确保在主线程上初始化托盘。
- GPUI 的 `App::run()` 是阻塞的，需要在 GPUI 启动**之前**初始化托盘或使用 `cx.on_app_activated()` 回调。
- 如果 `tray-icon` 与 GPUI 不兼容，考虑使用 `tao` crate（GPUI 依赖的窗口库）的托盘 API。

##### 任务 5.3: 开机自启动

`src/autolaunch.rs`：
- 📦 引入 `auto-launch` crate（0.6+）
- 实现 `set_autolaunch(enabled: bool)` 方法
- 配置面板中加入开关 UI

##### 任务 5.4: 启动行为

- 实现"启动时最小化到托盘"选项
- 首次启动时显示主窗口

#### 验收标准

- [ ] 应用启动后托盘出现图标
- [ ] 关闭窗口后托盘仍在，进程未退出
- [ ] 左键单击托盘图标可以切换窗口显示/隐藏
- [ ] 右键菜单包含"打开主界面"和"退出"选项，功能正常
- [ ] "开机自启动"开关可正常启用/禁用
- [ ] 各平台事件循环无 panic

---

### 🎯 Milestone 6: 实时 ASR 对接（阿里云百炼）

**工期**：5-7 天 | **优先级**：P1 | **依赖**：M3

#### 🤖 Agent 任务清单

##### 任务 6.1: WebSocket 客户端封装

`src/asr/websocket.rs`：
- 基于 `tokio-tungstenite` 实现异步 WebSocket 客户端
- 支持连接建立、鉴权、心跳保活（ping/pong，间隔 30s）
- 自动重连：最多 3 次，指数退避（1s, 2s, 4s）
- 重连期间继续本地录音缓存（数据不丢失）

##### 任务 6.2: 实时 ASR 协议实现

`src/asr/backends/bailian_streaming.rs`：
- 按照百炼实时 ASR 协议构造握手请求
- 音频分帧发送：每帧 200ms（6400 bytes @ 16kHz/16bit/mono）
- 使用 `tokio::time::interval(Duration::from_millis(200))` 定时发送
- 接收并解析服务端返回的中间结果和最终结果

##### 任务 6.3: 流式音频管道

`src/audio/chunk_sender.rs`：
- 使用 `tokio::sync::mpsc::channel::<Vec<u8>>(64)` 连接音频处理和 WebSocket 发送
- 音频采集 → ringbuf → 重采样 → mpsc sender → WebSocket send
- 接收线程 → 解析 JSON → mpsc sender → UI update

##### 任务 6.4: 增量更新 UI

- `is_final == false`：未稳定文本以浅灰色/斜体渲染
- `is_final == true`：转为正常样式，追加到稳定文本区
- 用户可在识别过程中自由编辑已稳定文本
- 伪代码模式：
  ```rust
  // 接收到 StreamingResult
  if result.is_final {
      // 将稳定文本从 pending 移到 confirmed
      self.confirmed_text.push_str(&result.delta_text);
      self.pending_text.clear();
  } else {
      // 更新 pending 文本
      self.pending_text = result.delta_text;
  }
  cx.notify(); // 触发重渲染
  ```

##### 任务 6.5: 异常场景处理

- WebSocket 断开 → 自动重连（重连期间数据缓存）
- 重连全部失败 → 回退离线模式
- 鉴权失败 → 停止录音，提示更新 API Key

#### 验收标准

- [ ] 开启实时模式后，说话时文本即时出现在编辑器中
- [ ] 中间结果与最终结果有视觉区分
- [ ] WebSocket 断连后能自动重连并继续识别
- [ ] 停止录音后最终结果正确
- [ ] 识别过程中用户可手动编辑已稳定文本
- [ ] 网络断开后回退离线模式，数据不丢失

---

### 🎯 Milestone 7: ASR 后端插件化

**工期**：4-6 天 | **优先级**：P1 | **依赖**：M4, M6

#### 🤖 Agent 任务清单

##### 任务 7.1: 定义完整 ASR Trait

`src/asr/traits.rs` — 按 6.2 节的定义实现 `AsrBackend` trait

##### 任务 7.2: 实现错误类型

`src/asr/error.rs` — 按 6.3 节的定义实现 `AsrError` 枚举

##### 任务 7.3: 实现后端注册表

`src/asr/registry.rs` — 按 6.4 节的定义实现 `AsrBackendRegistry`

##### 任务 7.4: 重构现有代码

- 将 `bailian_offline` 改为实现 `AsrBackend` trait
- 将 `bailian_streaming` 改为实现 `AsrBackend` trait
- 应用层代码仅依赖 trait，不直接 import 具体后端模块
- 后端通过 `backend_id` 动态获取

##### 任务 7.5: 通用 WebSocket 后端

`src/asr/backends/generic_ws.rs`：
- 实现 `GenericWsBackend`，用户可配置自定义 WS URL
- 支持自定义鉴权 Header

##### 任务 7.6: 连接测试功能

- `validate_config()` 的具体实现
- 设置面板中的"测试连接"按钮

#### 验收标准

- [ ] 可在配置文件中切换后端（如从百炼切换到通用 WebSocket）
- [ ] 添加新后端只需实现 `AsrBackend` trait + 注册，无需修改核心代码
- [ ] 后端切换后功能正常

---

### 🎯 Milestone 8: 本地 ASR 集成 (qwen-asr)

**工期**：5-8 天 | **优先级**：P2 | **依赖**：M7

#### 🤖 Agent 任务清单

##### 任务 8.1: 引入 qwen-asr 依赖

📦 **添加到 `Cargo.toml`**：

```toml
# 使用 feature gate 控制编译
[features]
default = []
local-asr = ["qwen-asr"]

[dependencies]
qwen-asr = { git = "https://github.com/huanglizhuo/QwenASR", optional = true }
```

##### 任务 8.2: 实现 AsrBackend trait

`src/asr/backends/qwen_asr.rs` （通过 `#[cfg(feature = "local-asr")]` 条件编译）：
- `transcribe_offline()`: WAV → qwen-asr 输入格式 → 推理 → 文本
- `supports_streaming()` → `false`（qwen-asr 仅离线）
- `supports_offline()` → `true`
- `validate_config()` → 检查模型文件存在且完整

##### 任务 8.3: 模型管理模块

`src/model_manager.rs`：
- 模型下载（HTTP GET，带进度回调）
- 断点续传（`Range` header）
- SHA256 完整性校验
- 模型存储：`{app data dir}/models/qwen-asr/{model_size}/`
- 支持 base / small / medium 三种规格

##### 任务 8.4: 推理线程管理

- 使用 `tokio::task::spawn_blocking` 执行推理
- 设置 `tokio` runtime 的 `max_blocking_threads` = 4
- 模型惰性加载（首次使用时加载，`OnceCell` 复用）
- 推理期间通过 channel 发送心跳防止 UI 假死

##### 任务 8.5: 下载引导 UI

- 首次切换到本地 ASR 时弹出下载引导对话框
- 显示模型大小、下载进度条、预计剩余时间
- 下载完成后自动切换

#### 🔧 人工操作

Agent 在开始 M8 前需确认：
1. `qwen-asr` crate 与 Rust Edition 2024 兼容
2. qwen-asr 的模型文件托管位置（URL）
3. 许可证兼容性确认（qwen-asr + Qwen3-ASR 模型 vs Apache 2.0）

#### 验收标准

- [ ] 不联网情况下可使用 qwen-asr 完成离线转写
- [ ] base 模型在 4 核 PC 上转写 1 分钟音频 < 60 秒
- [ ] 中文 WER < 15%（安静环境，标准普通话）
- [ ] 模型下载流程完整可用（含断点续传 + 校验）
- [ ] 模型加载后内存增长 < 500MB
- [ ] 连续 10 次转写无内存泄漏

---

### 🎯 Milestone 9: 全局快捷键

**工期**：3-5 天 | **优先级**：P2 | **依赖**：M5

#### 🤖 Agent 任务清单

##### 任务 9.1: 跨平台热键抽象

`src/hotkey/mod.rs`：
```rust
pub trait HotkeyHandler: Send + Sync {
    fn on_recording_toggle(&self);
    fn on_window_toggle(&self);
    fn on_copy_and_paste(&self);
}

pub struct GlobalHotkeyManager { /* ... */ }
```

##### 任务 9.2: 平台实现

- `src/hotkey/windows.rs`：`RegisterHotKey` + `WM_HOTKEY` 消息循环
- `src/hotkey/macos.rs`：`CGEvent` tap 或 `NSEvent` monitor
- `src/hotkey/linux.rs`：X11 `XGrabKey` / Wayland portal

##### 任务 9.3: 热键自定义 UI

- 设置面板中显示当前绑定
- "重新录制"按钮：点击后监听下次按键组合
- 冲突检测并提示

##### 任务 9.4: 一键复制并粘贴

- 复制文本到剪贴板 + 模拟 `Ctrl+V` / `Cmd+V` 粘贴到前台应用
- ⚠️ 粘贴模拟各平台实现不同，需要分平台处理

#### 验收标准

- [ ] 在任意应用前台时，触发录音热键能开始/停止录音
- [ ] 触发唤起热键能显示/隐藏 VoxInk 窗口
- [ ] 一键复制并粘贴能将文本粘贴到前台应用
- [ ] 快捷键可在设置中自定义
- [ ] 冲突热键注册时给出提示

---

### 🎯 Milestone 10: 文本历史与会话管理

**工期**：4-6 天 | **优先级**：P2 | **依赖**：M2

#### 🤖 Agent 任务清单

##### 任务 10.1: SQLite 数据库

`src/history/db.rs`：

```sql
-- 会话表
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 转录历史表
CREATE TABLE IF NOT EXISTS transcriptions (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    mode TEXT NOT NULL,  -- "streaming" | "offline" | "local"
    duration_secs INTEGER NOT NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- 全文搜索索引
CREATE VIRTUAL TABLE IF NOT EXISTS transcriptions_fts
USING fts5(text, content=transcriptions, content_rowid=rowid);
```

##### 任务 10.2: 历史面板 UI

`src/history/panel.rs`：
- 侧边栏抽屉展示历史列表
- 每条显示：时间戳 + 模式图标 + 文本前 50 字预览
- 点击条目载入编辑器
- 支持删除单条或全部清空
- 搜索框 + 高亮结果

##### 任务 10.3: 会话管理

`src/session.rs`：
- 创建/切换/删除命名会话
- 默认为 "默认会话"
- 切换时编辑器内容切换
- 新建录音追加到当前会话

##### 任务 10.4: 数据保留

- 按 `text.history_retention_days` 自动清理过期记录
- 手动导出为 JSON 文件

#### 验收标准

- [ ] 转录完成后自动保存到历史
- [ ] 历史列表可查看、搜索、点击载入
- [ ] 支持创建/切换/删除会话
- [ ] 导出功能生成有效 JSON 文件

---

### 🎯 Milestone 11: 设置面板完善与主题

**工期**：3-4 天 | **优先级**：P2 | **依赖**：M5

#### 🤖 Agent 任务清单

##### 任务 11.1: 完整设置面板 UI

`src/settings/panel.rs`：按 5.4 节设计实现

##### 任务 11.2: 主题系统

`src/theme.rs`：
- 定义 `ThemeColors` 结构体（背景、前景、强调色、各状态色等）
- light / dark 两套主题预设
- 跟随系统设置（通过 `dark-light` crate 或平台 API）
- 主题变量通过 GPUI 的 `cx.set_global()` 共享

##### 任务 11.3: i18n 基础

`src/i18n/`：
- 定义翻译键值（中文 / English 初始版本）
- 使用 `rust-i18n` crate 或简单的 HashMap 方案
- 设置面板中切换语言后 UI 即时更新

##### 任务 11.4: 关于面板

- 版本号、构建时间、Git commit hash（`env!("CARGO_PKG_VERSION")` 等）
- 开源协议链接
- "导出诊断信息"按钮

#### 验收标准

- [ ] 设置面板各项配置功能完整可用
- [ ] 深色/浅色主题切换流畅
- [ ] 中英文界面切换正常
- [ ] "导出诊断信息"生成完整 ZIP 文件

---

### 🎯 Milestone 12: 测试、打包与发布

**工期**：5-8 天 | **优先级**：P1 | **依赖**：M1-M11

#### 🤖 Agent 任务清单

##### 任务 12.1: 单元测试

- 配置加解密测试
- 音频重采样正确性测试（使用已知输入/输出）
- ASR 后端 trait mock 测试
- 状态机非法转换测试

##### 任务 12.2: 集成测试

- 模拟完整录音→转写→复制流程
- 配置持久化 + 重启恢复测试

##### 任务 12.3: CI/CD

`.github/workflows/ci.yml`：
```yaml
# 矩阵构建: Windows x86_64, macOS x86_64, macOS aarch64, Linux x86_64
# 步骤: cargo fmt --check → cargo clippy → cargo test → cargo build --release
```

##### 任务 12.4: 应用打包

- Windows: NSIS 安装包
- macOS: .app bundle + DMG
- Linux: AppImage / .deb

##### 任务 12.5: 文档

- `docs/USER_GUIDE.md`
- `docs/DEVELOPER.md`
- `CHANGELOG.md`

#### 验收标准

- [ ] `cargo test` 所有测试通过，覆盖率 > 60%
- [ ] `cargo clippy` 无 warning
- [ ] CI 流水线在三平台上成功构建
- [ ] 生成的安装包可正常安装和启动
- [ ] 用户文档覆盖所有功能的操作说明

---

## 9. 测试策略

### 9.1 测试金字塔

```
         ┌─────────┐
         │  E2E    │  少量（完整录音→转写→复制流程）
         │  Tests  │
        ┌┴─────────┴┐
        │ Integration│  中等数量（ASR mock, 配置持久化）
        │   Tests    │
       ┌┴─────────────┴┐
       │   Unit Tests   │  大量（重采样, 状态机, 加密）
       └───────────────┘
```

### 9.2 关键测试场景

| 场景 | 测试类型 | 优先级 |
|------|---------|-----|
| 无麦克风设备时启动 | 集成 | P0 |
| 网络断开时离线转写 | 集成 | P0 |
| API Key 无效 | 集成 | P0 |
| WebSocket 断开重连 | 集成 | P0 |
| 配置加解密正确性 | 单元 | P0 |
| 音频重采样精度 | 单元 | P0 |
| 状态机非法转换 | 单元 | P1 |
| 多平台托盘行为 | E2E | P1 |
| 长录音（10 分钟）稳定性 | E2E | P1 |

---

## 10. 打包与分发

### 10.1 发布渠道

| 渠道 | 平台 | 说明 |
|------|------|------|
| GitHub Releases | 全部 | 主要发布渠道 |
| Homebrew Cask | macOS | `brew install voxink` |
| Winget | Windows | `winget install VoxInk` |
| AUR | Linux (Arch) | `yay -S voxink` |

### 10.2 版本管理

语义化版本号 `MAJOR.MINOR.PATCH`：

- **MAJOR**：架构大改或不兼容的 API 变更
- **MINOR**：新功能（如新增 ASR 后端、本地 ASR 上线）
- **PATCH**：Bug 修复、性能优化

---

## 11. 关键技术难点与解决方案

> **🤖 AI Agent 避坑指南**：以下每个难点都是实际开发中高频踩坑点，Agent 必须仔细阅读。

### 11.1 GPUI 文本输入框

**难点**：GPUI 生态仍在快速发展，官方多行可编辑文本输入框的 API 可能不够成熟。

**解决方案**：
- 参考 GPUI 官方 `examples/` 中的 `text_input` 和 `editor` 示例
- 关注 `gpui::Editor` 和 `gpui::TextElement` 的最新 API
- 备选方案：使用 `gpui-component` 库中的 `TextInput` 组件
- 降级方案：若 GPUI 原生编辑器不满足需求，可自定义 `View` 实现基本文本编辑（光标、选择、输入法支持）

### 11.2 异步与 UI 线程隔离

**难点**：音频 I/O 和网络请求必须在后台执行，不能阻塞 GPUI 渲染线程。

**解决方案**：
- 音频采集：`cpal` 回调中**仅做** `ringbuf.push()`，不进行任何阻塞操作
- 重采样：在独立的 Tokio 任务中从 ringbuf 读取并处理
- 网络请求：在 Tokio 任务中执行，结果通过 `cx.spawn()` 回 UI
- 推荐模式：
  ```rust
  cx.spawn(|mut cx| async move {
      let result = tokio::task::spawn_blocking(|| { /* ... */ }).await;
      cx.update(|cx| { /* update UI state */ })?;
  }).detach();
  ```

### 11.3 系统托盘与 GPUI 事件循环兼容

**难点**：GPUI 有自己的事件循环，`tray-icon` 等库可能依赖平台原生事件循环。

**解决方案**：
- macOS：GPUI 内部已处理 NSApplication 事件循环，`tray-icon` 需在主线程上初始化
- Windows：使用 `tray-icon` 的 Win32 后端，与 GPUI 的 Windows 事件循环一般兼容
- Linux：依赖 GTK/X11 的托盘实现可能与 GPUI 有冲突，需测试验证
- 若冲突无法解决，考虑使用 GPUI 的原始窗口 API 自行实现最小化托盘模拟

### 11.4 音频采样率匹配

**难点**：ASR 服务要求 16kHz/16bit/mono PCM，系统麦克风可能是 44.1kHz/48kHz 多声道。

**解决方案**：
- 使用 `cpal` 查询设备支持的配置，优先选择最接近目标的原生配置
- 使用 `rubato` crate 进行高质量 sinc 重采样
- 通道转换：多声道→单声道取平均
- 位深度：f32 → i16（`(sample * 32767.0) as i16`）
- **在录制开始时打印实际音频配置和重采样参数到日志**

### 11.5 实时 ASR 音频分帧策略

**难点**：实时 ASR 要求持续发送固定时长的音频帧，发送过快浪费带宽，过慢影响实时性。

**解决方案**：
- 推荐帧长：200ms（6400 bytes @ 16kHz/16bit/mono）
- 使用 `tokio::time::interval` 定时从 ringbuf 中取出 200ms 音频数据发送
- 音频通道和网络通道之间使用 `tokio::sync::mpsc::channel`（缓冲区 64）

### 11.6 全局热键的跨平台实现

**难点**：全局热键 API 在各平台差异巨大，且需要进程全局监听。

**解决方案**：
- 优先使用 `global-hotkey` crate（若成熟可用），否则分平台实现
- Windows：`RegisterHotKey` + `WM_HOTKEY` 消息循环
- macOS：`CGEvent` tap 或 `NSEvent.addGlobalMonitorForEventsMatchingMask`
- Linux X11：`XGrabKey` + X11 事件循环
- Linux Wayland：需通过 compositor 协议（复杂度高），可降级为 CLI 参数方案

### 11.7 macOS 代码签名与公证

**难点**：macOS Gatekeeper 要求应用经过代码签名和公证。

**解决方案**：
- 需要 Apple Developer Program 会员
- CI 中使用 `codesign` 签名 + `xcrun notarytool` 提交公证
- 若无法获得开发者证书，提供 Homebrew 编译安装方式作为替代

### 11.8 qwen-asr 本地推理集成

**难点**：将纯 Rust、CPU-only 推理引擎嵌入异步架构，需处理模型生命周期和线程隔离。

**解决方案**：
- 模型生命周期：`OnceCell` 实现惰性加载和全局复用
- 阻塞推理隔离：通过 `tokio::task::spawn_blocking` 提交到专用线程池
- `max_blocking_threads` 至少为 4
- 推理期间通过 channel 发送心跳防止 UI 假死
- 首次启动运行 micro-benchmark 评估 CPU 性能，推荐合适的模型规格

---

## 12. 术语表

| 术语 | 英文 | 说明 |
|------|------|------|
| ASR | Automatic Speech Recognition | 自动语音识别 |
| PCM | Pulse-Code Modulation | 脉冲编码调制，原始未压缩音频 |
| WAV | Waveform Audio File Format | 音频文件格式 |
| VAD | Voice Activity Detection | 语音活动检测 |
| Ring Buffer | Ring Buffer | 环形缓冲区，无锁循环队列 |
| Resampling | Resampling | 音频采样率转换 |
| MPSC | Multiple Producer Single Consumer | 多生产者单消费者通道 |
| GPUI | GPUI | Zed 编辑器的 Rust UI 框架 |
| WER | Word Error Rate | 词错率，ASR 准确度指标 |
| qwen-asr | qwen-asr | 纯 Rust CPU-only Qwen3-ASR 本地推理引擎 |
| spawn_blocking | spawn_blocking | Tokio 将阻塞任务提交到专用线程池的方法 |
| HKDF | HMAC-based Key Derivation Function | 密钥派生函数 |

---

## 13. 附录

### 附录 A：项目目录结构（强制约定）

```
VoxInk/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── CHANGELOG.md
├── .gitignore
├── .github/
│   └── workflows/
│       ├── ci.yml              # CI 流水线
│       └── release.yml         # Release 构建流水线
├── docs/
│   ├── PRD.md                  # 本文件
│   ├── USER_GUIDE.md           # 用户使用手册
│   └── DEVELOPER.md            # 开发者文档
├── assets/
│   ├── icon.png                # 应用图标 (1024x1024)
│   ├── icon.ico                # Windows 图标
│   ├── icon.icns               # macOS 图标
│   ├── tray_icon.png           # 托盘图标 (32x32)
│   ├── tray_icon_recording.png # 托盘图标-录音中
│   └── sounds/                 # 提示音文件
│       ├── recording_start.wav
│       └── recording_stop.wav
├── src/
│   ├── main.rs                 # 入口点 (GPUI App 启动 + Tokio runtime)
│   ├── app.rs                  # GPUI Window View 实现（主界面）
│   ├── state.rs                # AppState, RecordingState, TranscriptionMode
│   ├── config.rs               # VoxInkConfig 定义 + 读写 + 加密
│   ├── error.rs                # AppError 统一错误类型
│   ├── theme.rs                # 主题系统（ThemeColors, light/dark）
│   ├── i18n/                   # 多语言翻译
│   │   ├── mod.rs
│   │   ├── zh_CN.rs
│   │   └── en_US.rs
│   ├── audio/                  # 音频子系统
│   │   ├── mod.rs
│   │   ├── capture.rs          # cpal 音频采集
│   │   ├── buffer.rs           # RingBuf 环形缓冲区
│   │   ├── resample.rs         # rubato 重采样
│   │   └── writer.rs           # hound WAV 文件写入
│   ├── asr/                    # ASR 后端子系统
│   │   ├── mod.rs
│   │   ├── traits.rs           # AsrBackend trait 定义
│   │   ├── error.rs            # AsrError 错误类型
│   │   ├── registry.rs         # AsrBackendRegistry 注册表
│   │   ├── client.rs           # HTTP/WS 客户端封装
│   │   ├── websocket.rs        # WebSocket 客户端（实时 ASR）
│   │   └── backends/
│   │       ├── mod.rs
│   │       ├── bailian_offline.rs   # 阿里云百炼-离线 (M4)
│   │       ├── bailian_streaming.rs # 阿里云百炼-实时 (M6)
│   │       ├── generic_ws.rs        # 通用 WebSocket (M7)
│   │       └── qwen_asr.rs          # 本地 qwen-asr (M8)
│   ├── tray.rs                 # 系统托盘 (M5)
│   ├── autolaunch.rs           # 开机自启动 (M5)
│   ├── hotkey/                 # 全局快捷键 (M9)
│   │   ├── mod.rs
│   │   ├── windows.rs
│   │   ├── macos.rs
│   │   └── linux.rs
│   ├── history/                # 文本历史 (M10)
│   │   ├── mod.rs
│   │   ├── db.rs               # SQLite 操作
│   │   └── panel.rs            # 历史面板 UI
│   ├── session.rs              # 会话管理 (M10)
│   ├── settings/               # 设置面板 (M11)
│   │   ├── mod.rs
│   │   └── panel.rs
│   ├── model_manager.rs        # 模型下载管理 (M8)
│   └── diagnostics.rs          # 诊断信息导出 (M11)
└── tests/
    ├── integration/
    │   ├── audio_tests.rs
    │   ├── config_tests.rs
    │   └── asr_tests.rs
    └── e2e/
        └── full_flow_tests.rs
```

### 附录 B：阿里云百炼 ASR API 参考

- **WebSocket 实时 ASR**：`wss://dashscope.aliyuncs.com/api-ws/v1/inference`
  - 协议：二进制帧 + JSON 控制帧
  - 要求：16kHz, 16-bit, 单声道 PCM
  - 鉴权：Header `Authorization: Bearer <api_key>`
- **HTTP 离线 ASR**：`POST https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription`
  - 格式：multipart/form-data（音频文件）+ JSON 参数
  - 支持格式：WAV, MP3, FLAC
  - 最大文件：500MB
- 详细文档：https://help.aliyun.com/document_detail/dashscope/

### 附录 C：qwen-asr（本地 ASR 引擎）技术约束

| 项目 | 内容 |
|------|------|
| **仓库** | https://github.com/huanglizhuo/QwenASR |
| **模型** | Qwen3-ASR（阿里巴巴通义千问团队） |
| **实现语言** | 纯 Rust，无 C/C++ FFI 依赖 |
| **推理设备** | CPU-only，不支持 GPU |
| **支持模式** | 仅离线整段识别（不支持流式） |
| **音频输入** | 16kHz, 16-bit, 单声道 PCM WAV |
| **模型规格** | base (~200MB) / small (~500MB) / medium (~1GB) |
| **Rust Edition** | 需确认与 Edition 2024 兼容 |
| **Send + Sync** | 模型实例必须满足（`AsrBackend` trait bound） |

### 附录 D：GPUI 参考资源

- [GPUI 官方仓库](https://github.com/zed-industries/zed)
- [GPUI 示例代码](https://github.com/zed-industries/zed/tree/main/crates/gpui/examples)
- [gpui-component 组件库](https://longbridge.github.io/gpui-component)
- [cpal 音频采集示例](https://github.com/RustAudio/cpal/tree/master/examples)
- [rubato 重采样库](https://github.com/HEnquist/rubato)

---

> **文档版本**：v3.0 — AI Agent 可执行版
> **最后更新**：2026-06-11
> **作者**：VoxInk 产品团队
> **协议**：Apache License 2.0
