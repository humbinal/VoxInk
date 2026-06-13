# 📄 VoxInk — 产品需求文档 (PRD) — AI Agent 可执行版

> **声落成墨，让 AI 提示词快人一步。**
> *Speak your prompts, ink your thoughts.*

---

## 🤖 AI Agent 执行指南（请先阅读本节）

本文档既是产品需求文档，也是 AI 编程助手（如 Claude Code、Cursor、Copilot 等）的**可执行开发指南**。

### 本文档的设计哲学（必读）

本文档对内容做了**严格的分层**，不同层级有不同的权威性，Agent 必须区别对待：

| 层级                        | 内容                                                       | 权威性                                                                 |
|---------------------------|----------------------------------------------------------|---------------------------------------------------------------------|
| **Tier 1 — 核心契约**        | 跨模块的枚举、trait 方法集与语义、错误分类、配置 schema、数据库 schema、后端能力矩阵 | **权威**。其"形状与语义"是设计决策，必须遵守；见 [§2 核心契约](#2-核心契约-core-contracts) |
| **Tier 2 — 行为需求**        | 每个功能"做什么、满足什么约束、验收标准"                              | **权威**。描述意图，不描述实现                                              |
| **Tier 3 — 示意 / 实现建议** | 伪代码、调用模式提示、推荐参数值                                    | **非权威**。仅帮助理解，**不保证 API 准确**                                |

### ⚠️ 关键规则：权威层级与代码冲突处理

本文档**刻意不包含可直接复制的实现代码**（除核心契约的类型定义外）。原因：Rust 生态（尤其 `gpui` 处于 `latest git` 快速迭代期）与外部 crate 的 API 会随时间变化，固化的实现代码会迅速过时，诱导 Agent 对抗编译器、固化错误模式。

因此：

1. **契约类型定义（Tier 1）是权威的"形状与语义"**，但其中绑定的外部类型（如 `tokio::mpsc`、`chrono::DateTime`）是**当前选定的实现绑定**——以这些类型为默认，但若当前库版本的确切类型名/路径不同，以**当前库的真实 API 为准**，保持语义不变即可。
2. **所有非契约代码（Tier 3）均为示意伪代码**，不保证可编译、不保证 API 名称正确。
3. **当任何示意代码与真实编译器/库行为冲突时——以库为准**，按当前库文档实现，并在汇报中记录差异。**绝不为了迁就文档里的过时写法而扭曲代码或反复对抗编译器。**
4. **依赖版本以 [§1.4 技术栈表](#14-核心技术栈) 为唯一来源**，且优先解析"当前可用的最新兼容版本"，而非死守文档中的数字。版本号仅表达"不低于此大版本"的下限意图。

### 如何使用本文档

1. **按里程碑顺序执行**：每个 Milestone 是独立的开发单元，完成一个再进入下一个。
2. **先读契约再写代码**：动到跨模块类型时，先回到 [§2 核心契约](#2-核心契约-core-contracts) 确认权威定义。
3. **先读技术约束**：每个 Milestone 开头列出前置依赖、关键约束和避坑提示。
4. **严格按验收标准自检**：每个 Milestone 末尾有可勾选的验收清单，所有项通过后才能标记完成。
5. **遵循命名和目录规范**：附录 A 的项目目录结构是强制约定。
6. **每个 Milestone 结束时停手**：等待人工验证通过后再进入下一阶段。

### AI Agent 行为规范

- **一个 Milestone 一个 PR**：每个 Milestone 的变更作为一个独立提交单元。
- **先编译通过再写新代码**：每完成一个 Rust 源文件后运行 `cargo check`。
- **Clippy 零 Warning**：`cargo clippy -- -D warnings` 必须通过。
- **不要过度设计**：实现 PRD 描述的功能即可，不添加未被要求的功能；也不要为"解耦"而引入文档未要求的抽象层。
- **先读相关代码再修改**：修改文件前务必先用 Read 工具确认最新内容。
- **遇到阻塞必须停下来**：依赖 crate 版本不兼容、API 行为与预期不符、或需要人工决策（如申请 API Key）时，必须明确报告并等待指示。
- **遇到契约与实现冲突时**：按上面"权威层级"规则处理并记录。

### 文档中使用的标记说明

| 标记               | 含义                                          |
|------------------|---------------------------------------------|
| 🤖 **Agent 任务**  | 需要 AI Agent 直接执行的开发任务                        |
| 📐 **契约引用**      | 指向 [§2 核心契约](#2-核心契约-core-contracts) 的权威定义 |
| ⚠️ **避坑提示**      | 已知的技术难点或容易出错的地方，Agent 应特别注意                  |
| 🛑 **Agent 检查点** | Agent 必须在此处停下来自检并汇报结果                        |
| 🔧 **人工操作**      | 需要人工完成的操作（如申请 API Key、下载模型等）                 |
| 🧩 **示意**        | 非权威的伪代码 / 实现建议，以当前库为准                        |

---

## 目录

1. [项目概述](#1-项目概述)
2. [核心契约 (Core Contracts)](#2-核心契约-core-contracts)
3. [系统架构设计](#3-系统架构设计)
4. [详细功能需求](#4-详细功能需求)
5. [非功能性需求](#5-非功能性需求)
6. [UI/UX 设计规范](#6-uiux-设计规范)
7. [ASR 后端插件架构](#7-asr-后端插件架构)
8. [配置与安全管理](#8-配置与安全管理)
9. [开发里程碑（Agent 执行单元）](#9-开发里程碑agent-执行单元)
10. [测试策略](#10-测试策略)
11. [打包与分发](#11-打包与分发)
12. [关键技术难点与解决方案](#12-关键技术难点与解决方案)
13. [术语表](#13-术语表)
14. [附录](#14-附录)

---

## 1. 项目概述

### 1.1 项目背景

在日常使用大语言模型（LLM）时，复杂的提示词（Prompt）往往需要耗费大量的键盘输入与修改时间。**VoxInk**
是一款专为大模型高频使用者设计的开源、轻量级桌面语音提示词辅助工具，帮助用户将脑海中的灵感快速"落笔成墨"，无缝输出至大模型对话框中。

### 1.2 产品定位

- **目标用户**：LLM 高频使用者（开发者、研究员、作家、产品经理等），需要频繁输入复杂提示词的用户
- **核心价值**：以语音替代键盘输入，大幅降低提示词编写的时间成本与认知负担
- **产品形态**：桌面常驻助手应用（Windows / macOS / Linux），系统托盘常驻，随叫随到

### 1.3 核心设计原则

| 原则         | 说明                                         |
|------------|--------------------------------------------|
| **轻量优先**   | 极低的系统资源占用（内存 < 100MB 空闲），启动迅速（< 1s）        |
| **隐私优先**   | 敏感数据（API Key）本地加密存储；支持纯本地 ASR，音频不上传云端      |
| **插件化可扩展** | ASR 后端采用 trait 抽象，支持多种 ASR 服务商及本地引擎自由切换    |
| **渐进式体验**  | 默认开箱即用，高级功能通过设置逐步发现，降低初次使用门槛               |
| **跨平台一致**  | Windows / macOS / Linux 三平台核心体验一致，平台差异妥善处理 |

### 1.4 核心技术栈

> **本表是项目依赖的唯一权威来源（single source of truth）。** 各 Milestone 不再重复罗列版本号；需要某个 crate 时，回到本表确认。
>
> ⚠️ **版本号语义**：表中版本表达"建议的大版本下限"，**不是精确锁定**。Agent 应通过 `cargo add <crate>` 解析当前可用的最新兼容版本，而非硬编码文档中的数字。若某下限版本不存在或与 Edition 2024 不兼容，**停下来报告**，不要猜测。

| 层级 / 领域        | 技术选型                            | 版本下限（建议）             | 选型理由与说明                                  |
|:---------------|:--------------------------------|:--------------------|:-----------------------------------------|
| **GUI 渲染框架**   | `gpui`                          | latest git          | Zed 团队的高性能 UI 框架，GPU 级渲染，极速冷启动           |
| **UI 组件**      | `gpui-component`                | latest git          | GPUI 生态组件库                               |
| **基础语言与环境**    | Rust                            | Edition 2024, MSRV 1.85+ | 现代 Rust 语法，开启最新特性                        |
| **异步与并发**      | `tokio`                         | 1.x (multi-thread)  | 事实标准的异步运行时，调度网络与耗时任务                     |
| **音频采集控制**     | `cpal`                          | 最新稳定版               | 跨平台音频底层访问，获取麦克风流                         |
| **音频处理管线**     | `hound` + `rubato`              | 最新稳定版               | WAV 读写与高质量 Sinc 音频重采样                    |
| **无锁环形缓冲**     | `ringbuf`                       | 最新稳定版               | 音频回调线程与 Tokio 工作线程间的无锁数据传递               |
| **静音检测 (VAD)** | `webrtc-vad`（可选）                | 最新稳定版               | 极低 CPU 占用的端点检测，实现自动停止录音（可选特性）            |
| **网络层通讯**      | `reqwest` + `tokio-tungstenite` | 最新稳定版               | HTTP 与 WebSocket，负责与云端 ASR 交互            |
| **系统托盘集成**     | `tray-icon`                     | 最新稳定版               | Tauri 团队维护的跨平台托盘                         |
| **全局快捷键**      | `global-hotkey`                 | 最新稳定版               | Tauri 团队维护，跨平台系统级热键                      |
| **剪贴板控制**      | `arboard`                       | 最新稳定版               | 全平台剪贴板读写                                 |
| **应用自启动**      | `auto-launch`                   | 最新稳定版               | 管理注册表/LaunchAgent 实现开机自启                 |
| **配置存储**       | `serde` + `toml` + `directories`| 最新稳定版               | 使用带注释、可读性更高的 TOML 持久化用户配置                |
| **安全与加密**      | `aes-gcm` + `hkdf`              | 最新稳定版               | **纯 Rust 实现**的 API Key 加密，无 C 编译链，跨平台无痛  |
| **本地 ASR**     | `qwen-asr`                      | latest git          | CPU-only、纯 Rust 实现的 Qwen3-ASR 本地语音识别引擎  |
| **本地历史数据库**    | `rusqlite`                      | 最新稳定版               | 开启 `bundled` + `fts5` feature，支持全文检索     |
| **错误处理规范**     | `thiserror` + `anyhow`          | 最新稳定版               | `thiserror` 处理模块级强类型错误，`anyhow` 处理顶层业务传播 |
| **多语言本地化**     | `rust-i18n`                     | 最新稳定版               | 编译期嵌入多语言词典                               |
| **日期时间**       | `chrono`                        | 最新稳定版               | 时间戳处理（features = ["serde"]）              |

> ⚠️ **加密库唯一选择**：本项目使用 `aes-gcm` + `hkdf`（纯 Rust）。**不要引入 `ring` 或其他需要 C 工具链的加密库**——这与"跨平台无痛编译"的设计原则冲突。

---

## 2. 核心契约 (Core Contracts)

> 🟦 **本章是 Tier 1 权威内容。** 这里定义的类型是跨模块、跨里程碑的**接缝（seam）**，是不同 Milestone 之间集成的共同约定。它们表达的是**产品与架构决策**，与具体 Rust/crate 版本无关，因此被赋予最高权威性。
>
> **使用约定（务必理解）：**
> - 下列定义的**字段集合、方法集合、变体集合、它们的语义**是权威的，Agent 实现时必须保持一致。
> - 定义中出现的**外部 crate 类型**（如 `tokio::sync::mpsc`、`chrono::DateTime`）是**当前选定的实现绑定**。以它们为默认；若当前库版本的确切类型路径/名称不同，以真实库为准、保持语义不变。
> - 这些类型应在指定模块中**只定义一次**，其他模块通过引用使用，**禁止各 Milestone 各自重新发明**。
> - 设计有意保持**最小化**：不要为"解耦外部 crate"而额外包装类型（违反"不过度设计"）。唯一的例外见 [§2.4 设计决策：错误分类与传输层解耦](#24-设计决策错误分类与传输层解耦)。

### 2.1 应用状态契约 — `src/state.rs`

```rust
/// 录音状态机。状态转移见 §4.1.2。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    /// 空闲，等待用户操作
    Idle,
    /// 正在录音
    Recording,
    /// 正在处理（上传转写 / 本地推理）
    Processing,
}

/// 转录处理模式（用户在主界面切换）。
///
/// 语义：此枚举描述"如何处理音频数据"，而非"由谁识别"。
/// "由谁识别"（云端/本地）由 AsrConfig.backend_id 决定（见 §2.5）。
/// 本地后端（qwen-asr）当前仅支持 Offline 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptionMode {
    /// 实时流式：音频分帧实时发送，识别结果增量返回
    Streaming,
    /// 离线整段：录音完成后一次性发送完整音频转写
    Offline,
}
```

> 📐 **应用全局状态（`AppState`）的字段集合**是契约，但其在 GPUI 中的承载方式（`Model<T>` / `Entity` / 其他）随 GPUI 版本而定，属实现细节，不在此固化。**字段语义**如下，Agent 须保证它们存在并被正确维护：
>
> | 字段                       | 类型                  | 语义                              |
> |--------------------------|---------------------|---------------------------------|
> | `recording_state`        | `RecordingState`    | 当前录音状态                          |
> | `transcription_mode`     | `TranscriptionMode` | 用户选择的转录模式                       |
> | `text_content`           | `String`            | 文本编辑器中已确认（稳定）的文本               |
> | `pending_text`           | `String`            | 流式识别中未稳定的尾部文本（视觉区分，见 §4.2.1） |
> | `recording_duration_secs`| `u32`               | 当前录音时长（秒），仅 Recording/Processing 有意义 |

### 2.2 ASR 后端契约 — `src/asr/traits.rs`

ASR 子系统通过 trait 抽象实现后端可插拔。下面的**方法集合与语义**是权威契约；所有后端（云服务、本地引擎、自定义服务）必须实现。

```rust
/// ASR 后端统一接口。
///
/// 约束：Send + Sync（线程间安全传递）+ 'static（可放入 tokio::spawn）。
/// trait 是否需要 #[async_trait] 宏，取决于当前 Rust 版本对 trait 中
/// async fn 的支持情况——以当前工具链为准（这是实现细节，非契约）。
pub trait AsrBackend: Send + Sync + 'static {
    /// 后端唯一标识符，对应 AsrConfig.backend_id。
    /// 示例: "aliyun_bailian_streaming", "qwen_asr_local"
    fn backend_id(&self) -> &str;

    /// 用户可见的后端名称。示例: "阿里云百炼（实时）", "本地 qwen-asr"
    fn display_name(&self) -> &str;

    /// 本后端是否支持实时流式识别
    fn supports_streaming(&self) -> bool;

    /// 本后端是否支持离线整段识别
    fn supports_offline(&self) -> bool;

    /// 验证配置是否有效（如测试 API Key 连通性）。
    /// Ok(()) 表示通过；Err(AsrError) 含具体错误。
    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError>;

    /// 实时流式识别。
    /// - audio_rx：音频 chunk 接收通道，每个 chunk 为 16kHz/16-bit/单声道 PCM 字节；
    ///   通道关闭表示录音结束，后端应发送结束信号并等待最终结果。
    /// - result_tx：识别结果发送通道，实时发送 partial/final；发完 final 后可 drop。
    async fn transcribe_streaming(
        &self,
        config: &AsrConfig,
        audio_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        result_tx: tokio::sync::mpsc::Sender<StreamingResult>,
    ) -> Result<(), AsrError>;

    /// 离线整段识别。
    /// - audio_data：完整 WAV 文件字节。
    /// - 返回完整转写文本。
    async fn transcribe_offline(
        &self,
        config: &AsrConfig,
        audio_data: Vec<u8>,
    ) -> Result<String, AsrError>;
}

/// 流式识别的单次增量结果。
#[derive(Debug, Clone)]
pub struct StreamingResult {
    /// 本次增量文本（仅新增部分）
    pub delta_text: String,
    /// 是否为句子结束的最终结果。
    /// true：文本已稳定，应转为正常样式并固化到 text_content；
    /// false：中间结果，应以斜体/浅色显示在 pending_text。
    pub is_final: bool,
    /// 结果时间戳
    pub timestamp: chrono::DateTime<chrono::Utc>,
}
```

### 2.3 ASR 错误分类契约 — `src/asr/error.rs`

下面的**错误变体集合（分类法 taxonomy）**是契约。所有后端实现必须使用此错误类型，不得对外暴露 `anyhow` 或裸字符串错误。

```rust
#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    /// 网络/传输层失败。携带可读描述字符串（见 §2.4 决策说明）。
    #[error("网络连接失败: {0}")]
    NetworkError(String),

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

    #[error("录音数据为空")]
    EmptyAudio,

    #[error("本地模型未找到: {0}")]
    ModelNotFound(String),

    #[error("本地推理失败: {0}")]
    InferenceError(String),

    #[error("配置无效: {0}")]
    InvalidConfig(String),

    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),
}
```

### 2.4 设计决策：错误分类与传输层解耦

> 这是本契约中唯一一处刻意的"解耦"，因为它有明确的架构收益，**不属于过度设计**。

`NetworkError` 携带 `String` 而**不是** `#[from] reqwest::Error`。原因：

- 若错误类型 `#[from] reqwest::Error`，则**整个 ASR 子系统（包括纯本地的 `qwen-asr` 后端）都会被动耦合到 reqwest**，即便本地后端根本不发 HTTP。
- 因此**错误分类法是契约，传输库是实现细节**。各后端在内部捕获自己的传输错误（reqwest / tokio-tungstenite / 本地推理），转换为合适的 `AsrError` 变体（如 `e.to_string()` 装入 `NetworkError`）。
- 同理，`WebSocketError`、`InferenceError` 也携带字符串而非具体库的错误类型。

### 2.5 ASR 配置契约 — `src/asr/config.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    /// 当前使用的后端 ID（对应 §7.4 注册表）
    pub backend_id: String,
    /// API Key（云服务）。持久化时加密，见 §8.3。
    pub api_key: String,
    /// API Endpoint
    pub api_endpoint: String,
    /// 本地引擎模型路径（qwen-asr），云后端为 None
    pub local_model_path: Option<String>,
    /// qwen-asr 模型规格（见 §4.3.3）
    pub local_model_size: Option<String>,
    /// 语言代码（"zh" / "en" / "auto"）
    pub language: String,
}
```

> 📝 **M4 落地扩展（OSS 字段）**：`aliyun_bailian_filetrans`（大文件异步）只接受公网 URL，需先把本地 WAV
> 上传到用户 OSS。为此 `AsrConfig` 新增四个 OSS 字段（语义不变，仅追加）：`oss_endpoint`、`oss_bucket`、
> `oss_access_key_id`、`oss_access_key_secret`。M4 阶段由环境变量填充（`OSS_ENDPOINT` / `OSS_BUCKET` /
> `OSS_ACCESS_KEY_ID` / `OSS_ACCESS_KEY_SECRET`）；M11 设置面板上线后改由加密配置提供（secret 需加密落盘）。

### 2.6 后端注册表与内置后端契约

后端通过工厂模式注册，运行时按 `backend_id` 获取实例。注册表的**接口语义**是契约（按 id 注册工厂、按 id 取实例、枚举所有后端及其能力）；其内部数据结构（`HashMap` 等）是实现细节。

**内置后端清单（契约）：**

| 后端 ID                      | 名称              | 类型  | 支持流式 | 支持离线 | 默认启用     | 引入里程碑 |
|----------------------------|-----------------|-----|------|------|----------|-------|
| `aliyun_bailian_streaming` | 阿里云百炼（实时）       | 云服务 | ✅    | ❌    | ✅        | M6    |
| `aliyun_bailian_offline`   | 阿里云百炼（离线·同步）    | 云服务 | ❌    | ✅    | ✅        | M4    |
| `aliyun_bailian_filetrans` | 阿里云百炼（离线·大文件）  | 云服务 | ❌    | ✅    | ✅        | M4    |
| `generic_ws`               | 通用 WebSocket    | 云服务 | ✅    | ❌    | ✅        | M7    |
| `qwen_asr_local`           | 本地 qwen-asr     | 本地  | ❌    | ✅    | ❌（需下载模型） | M8    |

> 📝 **M4 落地修订（离线后端拆为同步/大文件两种）**：阿里云百炼无"上传本地字节即同步返回"的单一离线接口。
> - `aliyun_bailian_offline`：**Qwen3-ASR-Flash** 同步接口，base64 内联本地音频，≤10MB（约 3-4 分钟）。
> - `aliyun_bailian_filetrans`：**qwen3-asr-flash-filetrans** 异步接口，仅接受公网 URL、不支持本地上传，故需先把 WAV 上传到用户 OSS（私有）→ 预签名 URL → 提交任务 → 轮询 → 取结果。支持超大/超长音频。
> - 应用层按音频大小路由：≤7MB（原始）走同步，否则走大文件。OSS 凭证经 `AsrConfig` 的 oss_* 字段提供（§2.5 扩展）。

### 2.7 持久化配置 Schema 契约

配置以 **TOML** 格式存储（见 §8）。下面是**字段结构与语义契约**，TOML 仅为示意格式。

```toml
version = 1

[general]
language = "zh-CN"            # 界面语言
theme = "system"             # "light" | "dark" | "system"
launch_at_startup = false
start_minimized = true
window_on_top = false
audio_feedback = true

[asr]
backend_id = "aliyun_bailian_streaming"
default_mode = "streaming"   # "streaming" | "offline"，对应 TranscriptionMode
api_endpoint = "https://dashscope.aliyuncs.com/api/v1/..."
api_key = "<encrypted>"      # 加密存储，见 §8.3
language = "zh"              # "zh" | "en" | "auto"
max_recording_seconds = 600
local_model_size = "base"    # "base" | "small" | "medium"

[shortcuts]
toggle_recording = "Ctrl+Alt+Space"
toggle_window = "Ctrl+Alt+V"
copy_and_paste = "Ctrl+Alt+B"

[text]
auto_copy = false
append_mode = true
history_retention_days = 30

[window]
x = 0                        # 上次窗口位置/尺寸；未设置时由应用决定默认
y = 0
width = 480
height = 600
```

### 2.8 历史数据库 Schema 契约 — `src/history/db.rs`

SQLite DDL 在跨版本上是稳定的，因此**表结构、字段、关系是契约**。

```sql
-- 会话表
CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- 转录历史表
CREATE TABLE IF NOT EXISTS transcriptions (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL,
    mode          TEXT NOT NULL,          -- "streaming" | "offline" | "local"
    duration_secs INTEGER NOT NULL,
    text          TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- 全文搜索索引（FTS5）
CREATE VIRTUAL TABLE IF NOT EXISTS transcriptions_fts
    USING fts5(text, content=transcriptions, content_rowid=rowid);
```

---

## 3. 系统架构设计

### 3.1 整体架构（四层模型）

```
┌──────────────────────────────────────────────────────────┐
│                     Presentation Layer                    │
│   Window(GPUI) · System Tray · Settings Dialog · Hotkey   │
├──────────────────────────────────────────────────────────┤
│                     Application Layer                     │
│   Audio Capture Controller · State Manager                │
│   (AppState, RecordingState, Config)                      │
├──────────────────────────────────────────────────────────┤
│                       Service Layer                       │
│   ASR Pipeline (Buffer → Resample → Encode → Backend)     │
│   Clipboard & Text Service (Copy, History, Session)       │
├──────────────────────────────────────────────────────────┤
│                   Infrastructure Layer                    │
│   ASR Backends (Plugin Trait) · Config Store · Logging    │
└──────────────────────────────────────────────────────────┘
```

### 3.2 数据流架构

```
Microphone ──[PCM]──▶ Audio Capture ──[f32]──▶ Ring Buffer
                                                  │
                                ┌─────────────────┴─────────────────┐
                                ▼                                   ▼
                         [Offline Mode]                      [Streaming Mode]
                                │                                   │
                          Temp WAV file                    Resample → 16kHz mono
                                │                                   │
                          Upload (HTTP POST)              WebSocket Send / Recv
                                │                                   │
                                ▼                                   ▼
                        Transcript Response                Partial / Final Result
                                │                                   │
                                └───────────────┬───────────────────┘
                                                ▼
                                  GPUI State Update（回主线程）
                                                ▼
                                  Text Editor（可编辑）
                                                ▼
                                  Copy to Clipboard / 用户手动编辑
```

### 3.3 线程模型（关键架构约束）

```
┌──────────────────────────────────────────────────────────┐
│                     Main Thread (GPUI)                     │
│  窗口渲染 & 事件循环；UI 状态变更；禁止阻塞（目标 < 16ms / 60fps）│
└───────────────────────────┬──────────────────────────────┘
                            │ 回主线程的异步更新机制 / MPSC channel
                            ▼
┌──────────────────────────────────────────────────────────┐
│                   Tokio Runtime Threads                    │
│   Audio I/O (cpal) · Network (reqwest/WS) · 本地 ASR 推理   │
└──────────────────────────────────────────────────────────┘
```

⚠️ **关键约束（Agent 必须遵守）**：

1. 音频采集回调**必须非阻塞**：回调中**仅做**数据拷贝到环形缓冲区（`ringbuf`），**不做**任何重采样、网络操作或文件 I/O。
2. 所有耗时操作（重采样、编码、网络发送、本地推理）在后台线程中执行。
3. UI 更新统一通过 GPUI 的"回主线程"机制（异步上下文回调 / 单消费者 MPSC channel）投递，**不在 GPUI 主线程上 `.await` 阻塞操作**。
4. 具体的"回主线程"API 名称随 GPUI 版本而定——以当前 GPUI 文档为准。

> 📝 **实现说明（M3 落地修订）**：约束 2 原文为"在 Tokio 工作线程中执行"。实际中，长生命周期的音频消费者（读环形缓冲→重采样→写 WAV）采用**专用 `std::thread`** 更自然——它全程占用一个线程，与 `tokio::task::spawn_blocking` 等价但更易用 `JoinHandle` 同步收尾，且避免占用 Tokio 阻塞线程池。短时/可调度的异步任务（网络、并发上传等）仍走 Tokio。此处"Tokio 工作线程"应理解为"任一非 UI、非音频回调的后台线程"，故正文已改为"后台线程"。cpal 的 `Stream` 句柄为 `!Send`，保留在主线程持有（回调在 cpal 自有音频线程触发），不跨线程移动。

---

## 4. 详细功能需求

### 4.1 音频录制控制

#### 4.1.1 设备管理

- **默认设备自动选择**：启动时自动探测并选择系统默认录音设备。
- **设备热插拔**：监听系统音频设备变更，默认设备切换时自动跟随（无需重启）。
- **设备列表展示**（v1.1+）：设置面板可查看可用设备列表，允许手动选择非默认设备。
- **权限处理**：系统未授予麦克风权限时（macOS 需 Info.plist 声明 `NSMicrophoneUsageDescription`），弹出友好提示引导用户开启。

#### 4.1.2 录音操作与状态机

- **开始录音**：点击主界面录音按钮或触发全局快捷键。
- **停止录音**：再次点击按钮 / 触发快捷键 / 点击托盘菜单"停止录音"。
- **状态机**（📐 状态枚举见 [§2.1](#21-应用状态契约--srcstaters)）：

```
  ┌──────────┐   点击开始    ┌──────────┐   点击停止    ┌──────────┐
  │   Idle   │──────────────▶│Recording │──────────────▶│Processing│
  │  (绿色)   │◀──────────────│  (红色)   │              │  (橙色)   │
  └──────────┘   取消/错误    └──────────┘              └─────┬─────┘
       ▲                                                     │
       └───────────────── 处理完成 / 错误 ─────────────────────┘
```

- **最长录音时长**：单次最长 600 秒（可在配置中调整），超时自动停止并提示。
- **静音检测**（v1.2+）：可选启用 VAD，检测到连续静音 N 秒后自动停止录音。

#### 4.1.3 录音状态反馈

- **视觉反馈**：录音按钮变红 + 外圈脉冲动画；状态栏显示 `MM:SS`；托盘图标叠加录音中标识（红点）。
- **音频电平指示**（v1.1+）：实时显示麦克风输入音量电平条。

### 4.2 语音转录模式

> **设计说明**：转录模式（实时/离线）与 ASR 后端（云端/本地）是两个**正交维度**（📐 见 [§2.1](#21-应用状态契约--srcstaters) 的 `TranscriptionMode` 语义）。
> - **转录模式**决定音频如何处理：流式发送 vs 结束后整段上传。
> - **ASR 后端**决定由谁识别：云服务 vs 本地引擎（`AsrConfig.backend_id`）。
> - 本地 qwen-asr 当前仅支持离线；选择本地后端时实时模式自动不可用。

#### 4.2.1 实时流式转录 (Streaming ASR)

通过 WebSocket 连接 ASR 服务，边说边识别，文本即时呈现。

- **流程**：
    1. 用户点击"开始录音"，建立与 ASR 后端的 WebSocket 连接。
    2. 音频采集线程持续写入环形缓冲区。
    3. 后台任务从缓冲区读取 chunk → 重采样至 16kHz/16bit/单声道 PCM → 通过 WebSocket 发送。
    4. 接收服务端实时返回的中间结果（partial）。
    5. 增量更新到文本编辑器。
    6. 用户停止录音 → 发送结束信号 → 接收最终结果（final）→ 替换对应段落的中间结果。
- **增量更新策略**（📐 数据载体见 [§2.2](#22-asr-后端契约--srcasrtraitsrs) 的 `StreamingResult`）：
    - 以 `is_final` 判断文本是否可固化为稳定结果。
    - 未稳定的尾部文本（`pending_text`）以斜体/浅色呈现，稳定后转为正常样式并并入 `text_content`。
    - 用户可在识别过程中随时手动编辑已稳定文本。
- **异常处理**：
    - WebSocket 断开：自动重连（最多 3 次，指数退避），重连期间继续本地录音（数据不丢失）。
    - 重连失败：回退为离线模式，录音停止后将缓冲数据作为离线请求发送。
    - 鉴权失败：立即停止录音，弹出重新配置 API Key 的提示。

#### 4.2.2 离线整段转录 (Offline ASR)

录音时仅做本地缓存，结束后一次性上传整段音频进行高精度识别。

- **流程**：
    1. 用户点击"开始录音"。
    2. 音频数据写入本地临时 WAV 文件（路径：`{OS temp dir}/voxink_recording_{timestamp}.wav`）。
    3. 用户点击"停止录音" → 关闭 WAV → UI 显示"正在识别中..."和 Loading 动画。
    4. 通过 HTTP POST 上传 WAV 至 ASR 后端。
    5. 接收完整转写结果 → 更新到文本编辑器。
    6. 删除临时 WAV（或按设置保留以便复查）。
- **上传进度**：大文件上传显示进度条。
- **超时处理**：HTTP 请求超时 120 秒（可配置），超时后提示"转写超时，请检查网络或缩短录音时长"。

#### 4.2.3 本地 ASR 后端 (qwen-asr) — 完全离线运行

**这不是独立转录模式，而是"离线转录"模式的一种后端实现。** 选择 ASR 后端为"本地 qwen-asr"时，离线流程完全在本地运行推理，音频**不离开本机**。

##### 引擎选型与约束

- **唯一引擎**：`qwen-asr`（https://github.com/huanglizhuo/QwenASR）
    - 基于 **Qwen3-ASR** 模型的 CPU-only、纯 Rust 实现。
    - 无需 GPU、无 FFI/C 依赖；中文识别精度优秀，支持多语言混合。

##### 技术约束

| 约束项       | 要求                     | 说明                                        |
|-----------|------------------------|-------------------------------------------|
| **推理设备**  | CPU-only               | 不支持 GPU 加速                                |
| **运行时依赖** | 纯 Rust                 | 无 C/C++ FFI 依赖                            |
| **模型下载**  | 首次启动引导下载               | 模型存储在 `{app data dir}/models/qwen-asr/`   |
| **音频输入**  | 16kHz, 16-bit, 单声道 PCM | 与音频采集管线一致                                 |
| **推理延迟**  | 目标 < 3× 实时             | 取决于 CPU 性能                                |
| **内存占用**  | 模型加载后额外 < 500MB        | 取决于模型规格                                   |
| **线程安全**  | `Send + Sync`          | 必须满足 `AsrBackend` trait bound（📐 [§2.2](#22-asr-后端契约--srcasrtraitsrs)） |

##### 模型规格

| 规格     | 文件大小   | 推理速度 | WER     | 推荐场景       |
|--------|--------|------|---------|------------|
| base   | ~200MB | 最快   | ~10-15% | 日常快速转写     |
| small  | ~500MB | 中等   | ~8-12%  | 需要较高精度     |
| medium | ~1GB   | 最慢   | ~5-10%  | 专业场景，高精度需求 |

### 4.3 文本编辑与交互

#### 4.3.1 文本编辑器

- **多行编辑**：支持任意输入、修改、删除、选择、复制、粘贴。
- **Undo/Redo**：支持撤销/重做（Ctrl+Z / Ctrl+Shift+Z，macOS 对应 Cmd 组合）。
- **追加 vs 覆盖**：
    - 实时模式：识别结果增量追加到已有文本末尾（可在设置中切换为"每次录音覆盖"）。
    - 离线模式：识别完成后**追加**到已有文本末尾。
- **字数统计**：底部状态栏实时显示字数/字符数。

#### 4.3.2 一键复制

- **复制按钮**：位于编辑器右下角，点击将全部文本写入系统剪贴板。
- **复制反馈**：按钮短暂变为"✓ 已复制"（1.5 秒后恢复）+ 轻量 Toast 提示。
- **自动复制**（可选设置）：转录完成后自动复制新内容到剪贴板。

#### 4.3.3 文本历史与会话管理（v1.1+）

- **历史记录**：每次转录完成自动保存到本地（SQLite，📐 schema 见 [§2.8](#28-历史数据库-schema-契约--srchistorydbrs)）。
- **历史面板**：侧边栏展示历史列表（时间戳、模式、预览）。
- **搜索**：全文搜索历史记录（FTS5）。
- **会话概念**：可创建/切换会话，每个会话维护独立文本上下文。
- **数据保留**：默认保留 30 天，可配置策略。

### 4.4 全局快捷键

#### 4.4.1 默认快捷键

| 功能           | Windows/Linux    | macOS              |
|--------------|------------------|--------------------|
| 开始/停止录音      | `Ctrl+Alt+Space` | `Cmd+Option+Space` |
| 唤起/隐藏主窗口     | `Ctrl+Alt+V`     | `Cmd+Option+V`     |
| 一键复制并粘贴到前台应用 | `Ctrl+Alt+B`     | `Cmd+Option+B`     |

#### 4.4.2 快捷键要求

- 所有快捷键支持用户在设置中自定义。
- 注册需处理与其他应用的冲突（检测冲突并提示更换）。
- 托盘后台运行时依然生效（进程全局监听）。

### 4.5 系统级特性

#### 4.5.1 开机自启动

- 设置面板提供开关。
- 实现方式：Windows 注册表 `HKCU\...\Run`；macOS LaunchAgent plist；Linux XDG Autostart `.desktop`。

#### 4.5.2 系统托盘

- **托盘图标**：默认（彩色）/ 录音中（叠加红点）/ 识别中（叠加橙色指示器）。
- **左键单击**：toggle 主窗口显示/隐藏。
- **右键菜单**：打开主界面、开始/停止录音、最近转录（子菜单）、设置、关于、退出。
- **Tooltip**：应用名称 + 当前状态。

#### 4.5.3 窗口管理

- **关闭按钮（X）**：隐藏到托盘，不退出应用。
- **最小化**：最小化到任务栏，保留托盘图标。
- **窗口置顶**（可选）。
- **窗口记忆**：记住上次位置和大小，下次启动恢复。

---

## 5. 非功能性需求

### 5.1 性能指标

| 指标             | 目标值       | 测量方法                        |
|----------------|-----------|-----------------------------|
| 应用冷启动时间        | < 1.5s    | 从进程启动到主窗口首次渲染完成             |
| 托盘唤起延迟         | < 200ms   | 从点击托盘图标到窗口完全显示              |
| 内存占用（空闲）       | < 80MB    | 启动后静置 1 分钟，无录音             |
| 内存占用（录音中）      | < 150MB   | 实时 ASR 模式，1 分钟录音            |
| CPU 占用（空闲）     | < 1%      | 同上                          |
| CPU 占用（本地 ASR） | < 50% 多核  | qwen-asr base 模型，现代 4 核 CPU |
| 音频延迟（采集到发送）    | < 100ms   | 实时模式，麦克风采集到 WebSocket 发送   |
| 本地 WAV 文件大小    | ~1.9MB/分钟 | 16kHz, 16-bit, mono PCM     |

### 5.2 兼容性矩阵

| 平台      | 最低版本                       | 架构                              |
|---------|----------------------------|---------------------------------|
| Windows | Windows 10 21H2+           | x86_64, aarch64                 |
| macOS   | macOS 13 Ventura+          | x86_64, aarch64 (Apple Silicon) |
| Linux   | Ubuntu 22.04+ / Fedora 38+ | x86_64                          |

### 5.3 安全性

| 安全项        | 措施                                               |
|------------|--------------------------------------------------|
| API Key 存储 | AES-256-GCM 加密，密钥派生自机器唯一标识（machine-id + 随机 salt） |
| 网络通信       | 所有 ASR API 调用强制 HTTPS/WSS（TLS 1.2+）              |
| 本地数据       | 录音临时文件和转写历史存储在应用私有数据目录                           |
| 依赖审计       | 定期 `cargo audit` 检查依赖漏洞                          |
| 日志脱敏       | 日志输出前过滤 API Key、Token 等敏感字段（替换为 `****`）          |

### 5.4 可访问性（v1.2+）

- 支持系统字体缩放；支持高对比度模式（跟随系统）；主要功能可键盘操作（Tab 导航 + Enter 确认）。

---

## 6. UI/UX 设计规范

### 6.1 窗口规格

| 属性     | 值                                                      |
|--------|--------------------------------------------------------|
| 默认窗口尺寸 | 480 × 600 px                                           |
| 最小窗口尺寸 | 360 × 400 px                                           |
| 最大窗口尺寸 | 800 × 1200 px                                          |
| 字体     | 系统默认（中文：Microsoft YaHei / PingFang SC / Noto Sans CJK） |
| 字体大小   | 正文 14px，标题 18px，状态栏 12px                               |
| 圆角     | 窗口 12px，按钮 8px，输入框 6px                                 |

### 6.2 主界面布局

```
┌─────────────────────────────────────┐
│  🎙 VoxInk                    ⚙ 设置 │  ← Header
├─────────────────────────────────────┤
│        ┌─────────────────┐          │
│        │   🎤 开始录音    │          │  ← 录音按钮（主操作区）
│        └─────────────────┘          │
│   转录模式: [实时 ═══ 离线]          │  ← Streaming/Offline 切换
│   状态: ● 录音中  00:42             │  ← 状态指示
│   [████████████░░░░] 音量条         │  ← 音频电平（v1.1）
├─────────────────────────────────────┤
│   ┌─────────────────────────────┐   │
│   │   这里是转写文本...          │   │
│   │   用户可以手动编辑...        │   │  ← 文本编辑区（主要空间）
│   └─────────────────────────────┘   │
├─────────────────────────────────────┤
│  字数: 128 │          📋 一键复制   │  ← Footer
└─────────────────────────────────────┘
```

### 6.3 主题与色彩

- **默认主题**：浅色（Light Mode）。v1.1+ 支持深色（跟随系统或手动切换）。
- **强调色** `#4A90D9` · **录音中** `#E74C3C` · **识别中** `#F39C12` · **就绪** `#27AE60`

### 6.4 设置面板结构

独立模态窗口或侧边抽屉，包含：

1. **ASR 服务配置**：后端选择、API Key、Endpoint URL、连接测试按钮。
2. **录音设置**：默认转录模式、自动复制、提示音、最长录音时长。
3. **通用设置**：开机自启、启动最小化、窗口置顶、主题、语言。
4. **快捷键设置**（v1.1+）：当前绑定显示 + 重新录制。
5. **关于**：版本号、协议、导出诊断、检查更新。

---

## 7. ASR 后端插件架构

> 本章描述插件架构的**设计意图与边界**。所有权威类型定义集中在 [§2 核心契约](#2-核心契约-core-contracts)，本章不再重复，只做引用与说明。

### 7.1 设计目标

ASR 能力通过 trait 抽象实现后端可插拔，支持：

- 云服务 ASR（阿里云百炼、讯飞、Azure Speech 等）
- 本地引擎 ASR（qwen-asr，CPU-only 纯 Rust）
- 自定义 ASR（用户自建服务，配置 WebSocket URL）

### 7.2 核心契约引用

| 契约              | 位置                                                          |
|-----------------|-------------------------------------------------------------|
| `AsrBackend` trait | 📐 [§2.2](#22-asr-后端契约--srcasrtraitsrs)                   |
| `StreamingResult`  | 📐 [§2.2](#22-asr-后端契约--srcasrtraitsrs)                   |
| `AsrError`         | 📐 [§2.3](#23-asr-错误分类契约--srcasrerrorrs)               |
| `AsrConfig`        | 📐 [§2.5](#25-asr-配置契约--srcasrconfigrs)                   |
| 后端注册表与内置后端清单    | 📐 [§2.6](#26-后端注册表与内置后端契约)                              |

### 7.3 后端注册表设计要点

- **工厂模式**：注册表保存 `backend_id → 创建闭包` 的映射；运行时按配置的 `backend_id` 取实例。
- **应用层只依赖 trait**，不直接 import 具体后端模块。
- **新增后端 = 实现 trait + 在注册表登记**，无需修改核心代码（开闭原则）。

---

## 8. 配置与安全管理

### 8.1 配置文件位置

| 平台      | 路径                                                 |
|---------|----------------------------------------------------|
| Windows | `%APPDATA%\VoxInk\config.toml`                     |
| macOS   | `~/Library/Application Support/VoxInk/config.toml` |
| Linux   | `~/.config/VoxInk/config.toml`                     |

> 路径通过 `directories` crate 解析，**不要硬编码**。

### 8.2 配置文件结构

📐 见 [§2.7 持久化配置 Schema 契约](#27-持久化配置-schema-契约)。配置使用 **TOML** 格式（带注释、可读性好）。配置文件含 `version` 字段，未来升级时据此做迁移。

### 8.3 敏感字段加密方案

- **加密算法**：AES-256-GCM（使用 `aes-gcm` crate，纯 Rust）。
- **密钥派生**：机器唯一标识符（machine-id）+ 随机 salt 通过 HKDF（`hkdf` crate）派生 256-bit 密钥。
- **加密字段**：`asr.api_key`。
- **存储格式**：`base64(nonce || ciphertext || tag)`（12 字节 nonce + 密文 + 16 字节 tag）。
- **安全设计**：机器绑定加密意味着更换设备/重装系统后需重新输入 API Key——这是有意为之。

---

## 9. 开发里程碑（Agent 执行单元）

> **🤖 这是 AI Agent 的核心执行章节。每个 Milestone 是独立开发单元，严格按顺序执行，完成一个再进行下一个。**
>
> **本章只描述"做什么、满足什么约束、如何验收"（Tier 2）。** 跨模块类型一律引用 [§2 核心契约](#2-核心契约-core-contracts)。本章不再内联实现代码——具体写法以当前库为准（见执行指南"权威层级"规则）。

### 里程碑概览

> ⚠️ **本版排序的重要变化**：核心契约（`AsrBackend` trait / `AsrError` / `AsrConfig` / 注册表骨架）在 **M4 一开始即定义并落地**，M4/M6 直接面向 trait 实现。这样 M7 只需"补充 generic_ws 后端 + 连接测试 UI + 完善注册表"，**避免了"先具体实现、再大重构"的 churn**（编程智能体最不擅长大范围重构）。

| #   | 名称                | 预计工期  | 核心交付                       | 依赖     |
|-----|-------------------|-------|----------------------------|--------|
| M1  | 项目初始化与基础 UI       | 3-5 天 | GPUI 窗口 + 静态布局             | —      |
| M2  | 状态管理与交互           | 3-4 天 | 按钮状态机 + 剪贴板 + 配置读写 + 加密    | M1     |
| M3  | 本地录音引擎            | 4-6 天 | cpal 录音 + 重采样 + WAV 存储     | M2     |
| M4  | ASR 契约 + 离线 ASR 对接 | 4-6 天 | trait/error/registry + 百炼离线 | M3     |
| M5  | 系统托盘与自启动          | 3-4 天 | 托盘集成 + 开机自启                | M2     |
| M6  | 实时 ASR 对接         | 5-7 天 | WebSocket 流式 + 增量更新（面向 trait）| M4     |
| M7  | 插件化完善            | 3-5 天 | generic_ws 后端 + 连接测试 UI    | M4, M6 |
| M8  | 本地 ASR 集成         | 5-8 天 | qwen-asr 推理 + 模型管理         | M7     |
| M9  | 全局快捷键             | 3-5 天 | 全局热键 + 自定义绑定 UI            | M5     |
| M10 | 文本历史与会话           | 4-6 天 | SQLite 存储 + 历史面板           | M2     |
| M11 | 设置面板完善            | 3-4 天 | 完整设置 UI + 主题切换             | M5     |
| M12 | 测试、打包与发布          | 5-8 天 | CI/CD + 安装包 + 文档           | M1-M11 |

---

### 🎯 Milestone 1: 项目初始化与基础 GPUI 界面搭建

**工期**：3-5 天 | **优先级**：P0 | **依赖**：无

#### 🤖 Agent 任务清单

**任务 1.1: 初始化 Cargo 项目**
- 创建 Edition 2024 项目（`name = "voxink"`）。
- 添加本阶段所需依赖：`gpui`、`tokio`、`serde` / `serde` derive、`tracing` + `tracing-subscriber`、`thiserror`、`anyhow`、`chrono`。版本以 [§1.4 技术栈表](#14-核心技术栈) 为准（解析最新兼容版本）。

**任务 1.2: 搭建项目目录结构**
- 按附录 A 创建目录（初期只需 `src/`、`docs/`、`assets/` 及基本文件）。

**任务 1.3: 实现应用入口（`src/main.rs`）**
- 初始化 `tracing_subscriber`（控制台输出，默认 INFO）。
- 创建 Tokio runtime。
- 启动 GPUI 应用，创建主窗口（480×600）。

**任务 1.4: 定义基础状态（`src/state.rs`）**
- 📐 按 [§2.1 应用状态契约](#21-应用状态契约--srcstaters) 定义 `RecordingState`、`TranscriptionMode` 及 `AppState` 字段。

**任务 1.5: 搭建 UI 布局骨架（`src/app.rs`）**
- 按 [§6.2 主界面布局](#62-主界面布局) 实现 Header / 控制区（大录音按钮 + 模式 Toggle + 状态文本）/ 文本编辑区 / Footer（字数 + 复制按钮）。

⚠️ **避坑提示**：
- GPUI 处于快速迭代期，API 可能变动。**先参考当前 GPUI 官方 examples**（尤其 `text_input` / `editor`），再动手。
- 布局使用 GPUI 的 flex 组合，不要用绝对定位。
- 多行可编辑文本框可优先尝试 `gpui-component` 的输入组件（见 [§12.1](#121-gpui-文本输入框)）。

**任务 1.6: 按钮点击事件（占位）**
- 录音/复制按钮点击 → 打印 `tracing::info!` 日志。

#### 🛑 Agent 检查点
```bash
cargo check
cargo clippy -- -D warnings
cargo run
```

#### 验收标准
- [ ] `cargo run` 在任一目标平台成功编译并渲染出窗口
- [ ] 窗口包含 Header、录音按钮、模式 Toggle、文本编辑区、复制按钮
- [ ] 点击录音/复制按钮有日志输出
- [ ] 文本编辑区支持键盘输入、选择、删除等基本操作
- [ ] 窗口可正常关闭，进程退出
- [ ] 日志正常输出到控制台（INFO 级别）

#### 关键文件
`Cargo.toml` · `src/main.rs` · `src/app.rs` · `src/state.rs`

---

### 🎯 Milestone 2: 状态管理与基础交互

**工期**：3-4 天 | **优先级**：P0 | **依赖**：M1

#### 🤖 Agent 任务清单

**任务 2.1: 录音按钮状态机**
- 📐 状态枚举见 [§2.1](#21-应用状态契约--srcstaters)；状态转移见 [§4.1.2](#412-录音操作与状态机)。
- 按钮文字/颜色随状态变化：Idle（绿，"🎤 开始录音"）/ Recording（红 + 脉冲，"⏹ 停止录音"）/ Processing（橙，"⏳ 处理中..."，不可点击）。

**任务 2.2: 剪贴板集成**
- 使用 `arboard`，封装"复制文本到系统剪贴板"能力。
- 复制成功后按钮短暂变为"✓ 已复制"（1.5 秒后恢复）+ Toast 提示。

**任务 2.3: 配置管理模块（`src/config.rs`）**
- 📐 配置 schema 见 [§2.7](#27-持久化配置-schema-契约)；格式为 TOML（`toml` + `serde`）。
- 使用 `directories` 解析各平台配置目录（见 [§8.1](#81-配置文件位置)）。
- 实现"加载（不存在则返回默认值）"与"保存"；实现合理的默认配置。
- 应用启动时加载，退出时保存；保留 `version` 字段以便未来迁移。

**任务 2.4: API Key 加密存储**
- ⚠️ **加密库唯一选择**：`aes-gcm` + `hkdf`（纯 Rust）。**不要使用 `ring`**（见 [§1.4](#14-核心技术栈) 末尾说明）。
- 实现"加密 API Key → base64 密文"与"解密"。
- 密钥派生：machine-id + 随机 salt → HKDF → 256-bit key（见 [§8.3](#83-敏感字段加密方案)）。
- 配置文件中仅出现密文，明文不落盘。

**任务 2.5: UI 状态联动**
- `AppState` 变更自动反映到 UI（按钮颜色、状态文本）。
- 🧩 **示意**：通过 GPUI 的状态承载与"通知重渲染"机制实现——具体 API（`Model`/`Entity`/`notify` 等名称）以当前 GPUI 版本为准。

#### 验收标准
- [ ] 录音按钮点击后颜色/文字正确切换（Idle→Recording→Idle）
- [ ] 复制按钮能将编辑器内容写入剪贴板，反馈正确
- [ ] 配置在启动时正确加载、退出时正确保存
- [ ] API Key 以密文存储，明文不出现在配置文件中
- [ ] 重启应用后配置正确恢复

---

### 🎯 Milestone 3: 本地录音引擎

**工期**：4-6 天 | **优先级**：P0 | **依赖**：M2

#### 🤖 Agent 任务清单

**任务 3.1: 音频设备探测（`src/audio/capture.rs`）**
- 用 `cpal` 枚举录音设备，自动选择系统默认输入设备。
- 设备不可用时返回明确错误类型，UI 显示友好提示。

**任务 3.2: 环形缓冲区（`src/audio/buffer.rs`）**
- 用 `ringbuf` 实现单生产者（音频回调）单消费者（Tokio 任务）的无锁环形缓冲区。
- 容量建议约 2 秒 16kHz/16bit/mono 音频。
- ⚠️ **避坑**：音频回调中**绝不**做阻塞操作——仅写入缓冲区；重采样、文件写入全部在 Tokio 任务中完成。

**任务 3.3: 音频采集**
- 配置 cpal 流参数（优先 16kHz / mono / f32）。
- 支持开始/停止录音；启动时打印实际音频配置到日志（采样率、通道数、格式）。

**任务 3.4: 重采样管线（`src/audio/resample.rs`）**
- 用 `rubato` 实现"任意采样率 → 16kHz"。
- 多声道 → 单声道（取平均）。
- f32 → i16 PCM 转换。
- Tokio 任务从缓冲区读取 → 重采样 → 写入输出。

**任务 3.5: WAV 文件写入（`src/audio/writer.rs`）**
- 用 `hound` 创建临时 WAV：`{temp_dir}/voxink_recording_{YYYYMMDD}_{HHMMSS}.wav`，规格 16kHz/16-bit/mono PCM。
- 流式写入（边录边写）。

**任务 3.6: 录音 UI 状态**
- 实时显示 `MM:SS`（每秒更新）。
- 超过 `max_recording_seconds` 自动停止。

#### 🛑 Agent 检查点
```bash
# 验证输出文件格式（期望：pcm_s16le, 16000 Hz, mono）
ffprobe <生成的 wav 文件>
```

#### 验收标准
- [ ] 点击"开始录音"正确采集麦克风数据
- [ ] 录音中 UI 实时显示 MM:SS 计时
- [ ] 停止后生成可播放的 .wav（16kHz/16-bit/mono PCM）
- [ ] 无麦克风时给出明确提示，不崩溃
- [ ] 录音 60 秒无内存泄漏（占用稳定）
- [ ] 超时自动停止并提示

---

### 🎯 Milestone 4: ASR 契约落地 + 离线 ASR 对接（阿里云百炼）

**工期**：4-6 天 | **优先级**：P1 | **依赖**：M3

> 本里程碑**先落地 ASR 核心契约**，再以契约实现首个后端。这样后续 M6/M7/M8 都面向同一 trait，无需大重构。

#### 🤖 Agent 任务清单

**任务 4.1: 落地 ASR 核心契约**
- 📐 按 [§2.2](#22-asr-后端契约--srcasrtraitsrs) 实现 `AsrBackend` trait + `StreamingResult`（`src/asr/traits.rs`）。
- 📐 按 [§2.3](#23-asr-错误分类契约--srcasrerrorrs) 实现 `AsrError`（`src/asr/error.rs`）。
- 📐 按 [§2.5](#25-asr-配置契约--srcasrconfigrs) 实现 `AsrConfig`（`src/asr/config.rs`）。
- 📐 按 [§2.6](#26-后端注册表与内置后端契约) 实现注册表骨架（`src/asr/registry.rs`）。

**任务 4.2: HTTP 客户端封装（`src/asr/client.rs`）**
- 基于 `reqwest::Client` 创建共享 HTTP 客户端，配置超时 120s、TLS 1.2+。
- 封装百炼 API 鉴权（Header `Authorization: Bearer <api_key>`）。

**任务 4.3: 百炼离线后端（`src/asr/backends/bailian_offline.rs`）**
- 实现 `AsrBackend`（`supports_offline() == true`，`supports_streaming() == false`）。
- `transcribe_offline()`：读取 WAV 字节 → base64 内联 → POST 到 Qwen3-ASR-Flash 同步接口 → 从 `choices[0].message.content` 取文本。
- 接口参考见 [附录 B](#附录-b阿里云百炼-asr-api-参考)（**已于 M4 修订**：原 multipart/file_urls 接口不适用，改用 Qwen3-ASR-Flash 同步 base64 接口；§2.7 单一 `api_endpoint` 默认是流式 wss URL，离线后端使用自身 HTTPS 默认端点，仅在用户显式配置 https 端点时覆盖）。
- ⚠️ 后端内部把 `reqwest` 错误转换为 `AsrError`（见 [§2.4 解耦决策](#24-设计决策错误分类与传输层解耦)）。

**任务 4.4: 异步任务与 UI 联动**
- 后台执行上传+识别，结果回传 UI 更新（不阻塞主线程）。
- 🧩 具体回主线程的 API 以当前 GPUI 为准。

**任务 4.5: Loading 状态与错误处理**
- 上传期间显示"正在转录..."+ 动画；大文件显示进度。
- 错误友好提示（映射 `AsrError` 变体）：网络超时、API Key 无效、配额用尽、空结果。

#### 🔧 人工操作
1. 用户已注册阿里云百炼并获取 API Key。
2. API Key 已填入配置（以加密形式存储）。
3. Agent 发请求时使用**解密后**的明文 Key。

> 📝 **M4 落地补充**：设置面板（§6.4）在 **M11** 才上线，M4 阶段无 UI 录入 API Key，而配置中的 `api_key` 为机器绑定加密、无法手工编辑。为使 M4 可被验证，离线流程在 `asr.api_key` 为空时**回退读取环境变量 `DASHSCOPE_API_KEY`**（明文不落盘）。验证方式：设置该环境变量后运行（如 PowerShell `$env:DASHSCOPE_API_KEY="sk-..."; cargo run`）。M11 设置面板上线后改为从加密配置读取，env 仅作开发期兜底。

#### 验收标准
- [ ] `AsrBackend` / `AsrError` / `AsrConfig` / 注册表按 §2 契约落地，编译通过
- [ ] 录音结束后自动触发上传 + 转写
- [ ] 转写期间正确显示 Loading
- [ ] 转写完成后文本追加到编辑器
- [ ] API Key 无效时显示明确错误
- [ ] 网络断开不崩溃，显示错误并保留音频文件

---

### 🎯 Milestone 5: 系统托盘与开机自启动

**工期**：3-4 天 | **优先级**：P1 | **依赖**：M2

#### 🤖 Agent 任务清单

**任务 5.1: 系统托盘集成（`src/tray.rs`）**
- 用 `tray-icon` 创建托盘图标（16×16 / 32×32）。
- 左键单击：toggle 主窗口；右键菜单：打开主界面、开始/停止录音、设置、退出。
- Tooltip 显示状态（"就绪" / "录音中 00:15"）。

**任务 5.2: 窗口生命周期管理**
- 关闭按钮（X）改为隐藏到托盘；托盘"退出"完全退出应用。
- `assets/` 下放置应用与托盘图标。

> 📝 **M5 落地状态（2026-06-13，部分推迟）**：托盘图标当前为**程序化生成的占位蓝点**；下列两项经用户同意**推迟**：
> 1. 用品牌图标（现有 `assets/icon.ico` 或后续提供的 `assets/tray_icon.png`）替换占位图。
> 2. §4.5.2 的**状态化托盘图标**（默认彩色 / 录音中叠红点 / 识别中叠橙色）与 Tooltip 动态状态（"录音中 00:15"）。
>
> 待用户提供设计好的 `tray_icon.png` 后接入；接入点在 `src/tray.rs` 的 `tray_icon_image()` 及 `GlobalTray`（持有 `TrayIcon`，可 `set_icon`/`set_tooltip` 按状态更新）。

⚠️ **避坑提示**（见 [§12.3](#123-系统托盘与-gpui-事件循环兼容)）：
- macOS：`tray-icon` 依赖 Cocoa 主线程，确保在主线程初始化。
- GPUI 的运行入口是阻塞的，需在合适时机初始化托盘——以当前 GPUI 提供的回调/钩子为准。

**任务 5.3: 开机自启动（`src/autolaunch.rs`）**
- 用 `auto-launch` 实现"启用/禁用开机自启"，配置面板加开关。

**任务 5.4: 启动行为**
- 实现"启动时最小化到托盘"；首次启动显示主窗口。

#### 验收标准
- [ ] 启动后托盘出现图标
- [ ] 关闭窗口后托盘仍在、进程未退出
- [ ] 左键单击托盘可切换窗口显示/隐藏
- [ ] 右键菜单"打开主界面"和"退出"功能正常
- [ ] "开机自启动"开关可正常启用/禁用
- [ ] 各平台事件循环无 panic

---

### 🎯 Milestone 6: 实时 ASR 对接（阿里云百炼）

**工期**：5-7 天 | **优先级**：P1 | **依赖**：M4

> 本里程碑面向 M4 已落地的 `AsrBackend` trait 实现流式后端，无需改动核心契约。

#### 🤖 Agent 任务清单

**任务 6.1: WebSocket 客户端封装（`src/asr/websocket.rs`）**
- 基于 `tokio-tungstenite` 实现异步 WS 客户端：连接、鉴权、心跳保活（ping/pong，约 30s）。
- 自动重连：最多 3 次，指数退避（约 1s/2s/4s）；重连期间继续本地缓存音频（数据不丢失）。

**任务 6.2: 百炼流式后端（`src/asr/backends/bailian_streaming.rs`）**
- 实现 `AsrBackend`（`supports_streaming() == true`）。
- 按百炼实时 ASR 协议握手；音频分帧发送（推荐帧长约 200ms，见 [§12.5](#125-实时-asr-音频分帧策略)）。
- 解析服务端中间结果/最终结果，封装为 `StreamingResult`（📐 [§2.2](#22-asr-后端契约--srcasrtraitsrs)）。

**任务 6.3: 流式音频管道（`src/audio/chunk_sender.rs`）**
- 用有界 MPSC 通道连接音频处理与 WS 发送：采集 → 缓冲区 → 重采样 → 发送通道 → WebSocket。
- 接收侧：解析 → 结果通道 → UI 更新。

**任务 6.4: 增量更新 UI**
- 按 `is_final` 区分稳定/未稳定文本（📐 [§2.1](#21-应用状态契约--srcstaters) 的 `text_content` / `pending_text`，语义见 [§4.2.1](#421-实时流式转录-streaming-asr)）。
- 🧩 **示意逻辑**（非 API 准确，以当前 GPUI 为准）：
  ```
  收到 StreamingResult：
    若 is_final：把 pending_text 并入 text_content，清空 pending_text
    否则：用 delta_text 更新 pending_text
    触发重渲染
  ```

**任务 6.5: 异常场景处理**
- WS 断开 → 自动重连（数据缓存）；重连全部失败 → 回退离线模式；鉴权失败 → 停止并提示更新 Key。

#### 验收标准
- [ ] 开启实时模式后，说话时文本即时出现
- [ ] 中间结果与最终结果有视觉区分
- [ ] WS 断连后能自动重连并继续识别
- [ ] 停止录音后最终结果正确
- [ ] 识别过程中用户可手动编辑已稳定文本
- [ ] 网络断开后回退离线模式，数据不丢失

---

### 🎯 Milestone 7: 插件化完善

**工期**：3-5 天 | **优先级**：P1 | **依赖**：M4, M6

> 核心契约已在 M4 落地，M4/M6 后端已面向 trait。本里程碑只做"补齐与完善"，**不做大重构**。

#### 🤖 Agent 任务清单

**任务 7.1: 校验解耦**
- 确认应用层仅依赖 `AsrBackend` trait，不直接 import 具体后端；后端通过 `backend_id` 动态获取。

**任务 7.2: 通用 WebSocket 后端（`src/asr/backends/generic_ws.rs`）**
- 实现 `GenericWsBackend`，用户可配置自定义 WS URL 与鉴权 Header。
- 📝 **M7 落地约定**：`api_endpoint` 填 `ws(s)://` URL，`api_key` 非空则作 `Authorization: Bearer`。约定协议：客户端发**二进制 PCM 帧**（16kHz/16bit/mono）；服务端回文本帧，JSON `{"text","is_final"}` 优先，否则纯文本作中间结果；停止时客户端发 Close，服务端给最终结果后关闭。无真实自建服务时端到端不可测，仅保证可注册/可切换。

**任务 7.3: 注册表完善**
- 注册全部内置后端（📐 [§2.6](#26-后端注册表与内置后端契约) 清单）；提供按 id 取实例、枚举后端及能力。
- 📝 **M7 落地**：已注册 `aliyun_bailian_offline`/`aliyun_bailian_filetrans`/`aliyun_bailian_streaming`/`generic_ws`（`qwen_asr_local` 属 M8，届时注册）。应用层用 `resolve_backend_id(config, want_streaming, audio_len)` 按**配置 backend_id + 能力**选后端：配置项支持该模式则用之（百炼离线大文件透明转 filetrans），否则用合理默认。应用层仅依赖 trait + 注册表，新增后端无需改核心（开闭原则）。

**任务 7.4: 连接测试功能**
- 实现各后端 `validate_config()`；设置面板加"测试连接"按钮。
- ⚠️ **里程碑顺序冲突（已记录）**：设置面板是 **M11**（§6.4），M7 阶段尚无面板可挂"测试连接"按钮。M7 实现：流式/`generic_ws` 的 `validate_config()` 做**真实 WS 握手测试**（401/403→AuthError）；离线后端暂为 api_key 非空校验。**临时**把主界面「⚙ 设置」按钮用作"测试连接"（测当前 `backend_id`，Toast 反馈），M11 设置面板上线后移到面板内。

#### 验收标准
- [ ] 可在配置中切换后端（如百炼 → 通用 WebSocket）
- [ ] 新增后端只需实现 trait + 注册，无需改核心代码
- [ ] 后端切换后功能正常
- [ ] "测试连接"能正确反馈成功/失败

---

### 🎯 Milestone 8: 本地 ASR 集成 (qwen-asr)

**工期**：5-8 天 | **优先级**：P2 | **依赖**：M7

#### 🤖 Agent 任务清单

**任务 8.1: 引入 qwen-asr 依赖（feature gate）**
- 通过 Cargo feature（如 `local-asr`）将 `qwen-asr` 设为可选依赖，控制条件编译。

**任务 8.2: 实现本地后端（`src/asr/backends/qwen_asr.rs`，`#[cfg(feature = "local-asr")]`）**
- 实现 `AsrBackend`：`transcribe_offline()`（WAV → 推理 → 文本）；`supports_streaming() == false`；`supports_offline() == true`；`validate_config()` 检查模型文件存在且完整。

**任务 8.3: 模型管理模块（`src/model_manager.rs`）**
- 模型下载（HTTP GET，进度回调）；断点续传（Range header）；SHA256 校验。
- 存储：`{app data dir}/models/qwen-asr/{model_size}/`；支持 base/small/medium。

**任务 8.4: 推理线程管理**
- 用 `spawn_blocking` 执行推理；模型惰性加载并复用；推理期间通过 channel 发心跳防 UI 假死。

**任务 8.5: 下载引导 UI**
- 首次切换本地 ASR 时弹出下载引导（大小、进度、预计剩余）；完成后自动切换。

#### 🔧 人工操作
1. 确认 `qwen-asr` 与 Edition 2024 兼容。
2. 确认模型托管 URL。
3. 确认许可证兼容性（qwen-asr + Qwen3-ASR 模型 vs Apache 2.0）。

#### 验收标准
- [ ] 不联网可用 qwen-asr 完成离线转写
- [ ] base 模型在 4 核 PC 上转写 1 分钟音频 < 60 秒
- [ ] 中文 WER < 15%（安静环境，标准普通话）
- [ ] 模型下载流程完整（断点续传 + 校验）
- [ ] 模型加载后内存增长 < 500MB
- [ ] 连续 10 次转写无内存泄漏

---

### 🎯 Milestone 9: 全局快捷键

**工期**：3-5 天 | **优先级**：P2 | **依赖**：M5

#### 🤖 Agent 任务清单

**任务 9.1: 跨平台热键抽象（`src/hotkey/mod.rs`）**
- 优先使用 `global-hotkey` crate；定义"录音切换 / 窗口切换 / 复制并粘贴"三类回调的处理接口。

**任务 9.2: 平台差异处理**
- 若 `global-hotkey` 不能满足，分平台降级（Windows `RegisterHotKey`；macOS `NSEvent` 全局监听；Linux X11 `XGrabKey`，Wayland 走 portal 或降级）。见 [§12.6](#126-全局热键的跨平台实现)。

**任务 9.3: 热键自定义 UI**
- 显示当前绑定；"重新录制"监听下次组合键；冲突检测并提示。

**任务 9.4: 一键复制并粘贴**
- 复制到剪贴板 + 模拟粘贴到前台应用。⚠️ 粘贴模拟各平台不同，分平台处理。

#### 验收标准
- [ ] 任意前台应用下，录音热键能开始/停止录音
- [ ] 唤起热键能显示/隐藏窗口
- [ ] 一键复制并粘贴能粘贴到前台应用
- [ ] 快捷键可在设置中自定义
- [ ] 冲突热键注册时给出提示

---

### 🎯 Milestone 10: 文本历史与会话管理

**工期**：4-6 天 | **优先级**：P2 | **依赖**：M2

#### 🤖 Agent 任务清单

**任务 10.1: SQLite 数据库（`src/history/db.rs`）**
- 📐 按 [§2.8 历史数据库 Schema 契约](#28-历史数据库-schema-契约--srchistorydbrs) 建表（含 FTS5）。
- 使用 `rusqlite`（启用 `bundled` + `fts5`）。

**任务 10.2: 历史面板 UI（`src/history/panel.rs`）**
- 侧边抽屉展示历史列表（时间戳 + 模式图标 + 前 50 字预览）；点击载入编辑器；删除单条/清空；搜索 + 高亮。

**任务 10.3: 会话管理（`src/session.rs`）**
- 创建/切换/删除命名会话；默认"默认会话"；切换时编辑器内容切换；新录音追加到当前会话。

**任务 10.4: 数据保留**
- 按 `history_retention_days` 自动清理过期记录；支持导出为 JSON。

#### 验收标准
- [ ] 转录完成后自动保存到历史
- [ ] 历史列表可查看、搜索、点击载入
- [ ] 支持创建/切换/删除会话
- [ ] 导出生成有效 JSON 文件

---

### 🎯 Milestone 11: 设置面板完善与主题

**工期**：3-4 天 | **优先级**：P2 | **依赖**：M5

#### 🤖 Agent 任务清单

**任务 11.1: 完整设置面板 UI（`src/settings/panel.rs`）**
- 按 [§6.4 设置面板结构](#64-设置面板结构) 实现。

**任务 11.2: 主题系统（`src/theme.rs`）**
- 定义主题色集合（背景/前景/强调/各状态色）；light/dark 预设；可跟随系统。
- 🧩 主题共享与系统主题探测的具体实现以当前 GPUI / 平台 API 为准。

**任务 11.3: i18n 基础（`src/i18n/`）**
- 用 `rust-i18n` 定义中/英翻译键；设置中切换语言后 UI 即时更新。

**任务 11.4: 关于面板**
- 版本号、构建时间、Git commit hash（`env!` 宏注入）；协议链接；"导出诊断信息"按钮。

#### 验收标准
- [ ] 设置面板各项配置功能完整可用
- [ ] 深色/浅色主题切换流畅
- [ ] 中英文界面切换正常
- [ ] "导出诊断信息"生成完整文件

---

### 🎯 Milestone 12: 测试、打包与发布

**工期**：5-8 天 | **优先级**：P1 | **依赖**：M1-M11

#### 🤖 Agent 任务清单

**任务 12.1: 单元测试**
- 配置加解密；音频重采样正确性（已知输入/输出）；ASR 后端 trait mock；状态机非法转换。

**任务 12.2: 集成测试**
- 模拟完整录音→转写→复制流程；配置持久化 + 重启恢复。

**任务 12.3: CI/CD（`.github/workflows/ci.yml`）**
- 矩阵构建（Windows x86_64 / macOS x86_64 / macOS aarch64 / Linux x86_64）。
- 步骤：`cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test` → `cargo build --release`。

**任务 12.4: 应用打包**
- Windows NSIS；macOS .app + DMG；Linux AppImage / .deb。

**任务 12.5: 文档**
- `docs/USER_GUIDE.md` · `docs/DEVELOPER.md` · `CHANGELOG.md`。

#### 验收标准
- [ ] `cargo test` 全部通过，覆盖率 > 60%
- [ ] `cargo clippy` 无 warning
- [ ] CI 在三平台成功构建
- [ ] 安装包可正常安装和启动
- [ ] 用户文档覆盖所有功能操作说明

---

## 10. 测试策略

### 10.1 测试金字塔

```
         ┌─────────┐
         │  E2E    │  少量（完整录音→转写→复制）
        ┌┴─────────┴┐
        │ Integration│  中等（ASR mock, 配置持久化）
       ┌┴─────────────┴┐
       │   Unit Tests   │  大量（重采样, 状态机, 加密）
       └───────────────┘
```

### 10.2 关键测试场景

| 场景             | 测试类型 | 优先级 |
|----------------|------|-----|
| 无麦克风设备时启动      | 集成   | P0  |
| 网络断开时离线转写      | 集成   | P0  |
| API Key 无效     | 集成   | P0  |
| WebSocket 断开重连 | 集成   | P0  |
| 配置加解密正确性       | 单元   | P0  |
| 音频重采样精度        | 单元   | P0  |
| 状态机非法转换        | 单元   | P1  |
| 多平台托盘行为        | E2E  | P1  |
| 长录音（10 分钟）稳定性  | E2E  | P1  |

---

## 11. 打包与分发

### 11.1 发布渠道

| 渠道              | 平台           | 说明                      |
|-----------------|--------------|-------------------------|
| GitHub Releases | 全部           | 主要发布渠道                  |
| Homebrew Cask   | macOS        | `brew install voxink`   |
| Winget          | Windows      | `winget install VoxInk` |
| AUR             | Linux (Arch) | `yay -S voxink`         |

### 11.2 版本管理

语义化版本号 `MAJOR.MINOR.PATCH`：

- **MAJOR**：架构大改或不兼容的 API 变更
- **MINOR**：新功能（如新增 ASR 后端、本地 ASR 上线）
- **PATCH**：Bug 修复、性能优化

---

## 12. 关键技术难点与解决方案

> **🤖 AI Agent 避坑指南**：以下是实际开发的高频踩坑点。本章给出的是**应对策略与方向**，不是可复制代码——具体写法以当前库为准。

### 12.1 GPUI 文本输入框

**难点**：GPUI 生态快速发展，官方多行可编辑文本框 API 可能不够成熟或频繁变动。

**策略**：
- 先参考当前 GPUI 官方 `examples/`（`text_input` / `editor`）确认现状。
- 优先尝试 `gpui-component` 的输入组件。
- 降级方案：若原生编辑器不满足，自定义 View 实现基本编辑（光标、选择、输入法支持）。

### 12.2 异步与 UI 线程隔离

**难点**：音频 I/O 和网络请求必须在后台执行，不能阻塞 GPUI 渲染线程。

**策略**：
- 音频采集回调中**仅做**缓冲区写入，不做任何阻塞操作。
- 重采样在独立 Tokio 任务中处理；网络请求在 Tokio 任务中执行，结果通过 GPUI 的"回主线程"机制更新 UI。
- CPU 密集型阻塞任务（本地推理）用 `spawn_blocking` 提交到专用线程池。
- 🧩 回主线程的确切 API 名称随 GPUI 版本而定。

### 12.3 系统托盘与 GPUI 事件循环兼容

**难点**：GPUI 有自己的事件循环，`tray-icon` 等库可能依赖平台原生事件循环。

**策略**：
- macOS：`tray-icon` 需在主线程初始化。
- Windows：`tray-icon` 的 Win32 后端一般与 GPUI 兼容。
- Linux：GTK/X11 托盘可能与 GPUI 冲突，需实测验证。
- 若冲突无法解决，考虑用 GPUI 原始窗口 API 自行实现最小化托盘模拟。

### 12.4 音频采样率匹配

**难点**：ASR 要求 16kHz/16bit/mono，系统麦克风常为 44.1kHz/48kHz 多声道。

**策略**：
- 用 `cpal` 查询设备支持配置，优先选最接近目标的原生配置。
- 用 `rubato` 高质量 sinc 重采样；多声道→单声道取平均；位深 f32→i16。
- **录制开始时把实际音频配置与重采样参数打印到日志。**

### 12.5 实时 ASR 音频分帧策略

**难点**：实时 ASR 需持续发送固定时长帧，过快浪费带宽，过慢影响实时性。

**策略**：
- 推荐帧长约 200ms。
- 定时从缓冲区取出该时长音频发送。
- 音频与网络阶段之间用有界 MPSC 通道衔接。

### 12.6 全局热键的跨平台实现

**难点**：全局热键 API 各平台差异巨大，需进程全局监听。

**策略**：
- 优先 `global-hotkey` crate；否则分平台实现。
- Windows `RegisterHotKey` + `WM_HOTKEY`；macOS `NSEvent` 全局监听 / `CGEvent` tap；Linux X11 `XGrabKey`；Wayland 经 compositor 协议（复杂，可降级）。

### 12.7 macOS 代码签名与公证

**难点**：macOS Gatekeeper 要求应用签名 + 公证。

**策略**：
- 需 Apple Developer Program 会员；CI 中 `codesign` 签名 + `notarytool` 公证。
- 无证书时提供 Homebrew 编译安装作为替代。

### 12.8 qwen-asr 本地推理集成

**难点**：将纯 Rust、CPU-only 推理引擎嵌入异步架构，需处理模型生命周期与线程隔离。

**策略**：
- 模型惰性加载并全局复用。
- 阻塞推理通过 `spawn_blocking` 提交专用线程池。
- 推理期间发心跳防 UI 假死。
- 首次启动可跑 micro-benchmark 评估 CPU 性能，推荐合适模型规格。

---

## 13. 术语表

| 术语             | 英文                                 | 说明                               |
|----------------|------------------------------------|----------------------------------|
| ASR            | Automatic Speech Recognition       | 自动语音识别                           |
| PCM            | Pulse-Code Modulation              | 脉冲编码调制，原始未压缩音频                   |
| WAV            | Waveform Audio File Format         | 音频文件格式                           |
| VAD            | Voice Activity Detection           | 语音活动检测                           |
| Ring Buffer    | Ring Buffer                        | 环形缓冲区，无锁循环队列                     |
| Resampling     | Resampling                         | 音频采样率转换                          |
| MPSC           | Multiple Producer Single Consumer  | 多生产者单消费者通道                       |
| GPUI           | GPUI                               | Zed 编辑器的 Rust UI 框架              |
| WER            | Word Error Rate                    | 词错率，ASR 准确度指标                    |
| qwen-asr       | qwen-asr                           | 纯 Rust CPU-only Qwen3-ASR 本地推理引擎 |
| spawn_blocking | spawn_blocking                     | Tokio 将阻塞任务提交到专用线程池的方法           |
| HKDF           | HMAC-based Key Derivation Function | 密钥派生函数                           |
| Contract       | Contract                           | 跨模块共同约定的类型/接口（本文 §2）             |

---

## 14. 附录

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
│   ├── state.rs                # 📐 RecordingState, TranscriptionMode, AppState（§2.1）
│   ├── config.rs               # VoxInkConfig 定义 + 读写 + 加密（§2.7, §8）
│   ├── error.rs                # AppError 统一错误类型
│   ├── theme.rs                # 主题系统
│   ├── i18n/                   # 多语言翻译
│   │   ├── mod.rs
│   │   ├── zh_CN.rs
│   │   └── en_US.rs
│   ├── audio/                  # 音频子系统
│   │   ├── mod.rs
│   │   ├── capture.rs          # cpal 音频采集
│   │   ├── buffer.rs           # ringbuf 环形缓冲区
│   │   ├── resample.rs         # rubato 重采样
│   │   ├── writer.rs           # hound WAV 文件写入
│   │   └── chunk_sender.rs     # 流式音频分帧发送 (M6)
│   ├── asr/                    # ASR 后端子系统
│   │   ├── mod.rs
│   │   ├── traits.rs           # 📐 AsrBackend trait（§2.2）
│   │   ├── error.rs            # 📐 AsrError（§2.3）
│   │   ├── config.rs           # 📐 AsrConfig（§2.5）
│   │   ├── registry.rs         # 📐 后端注册表（§2.6）
│   │   ├── client.rs           # HTTP 客户端封装
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
│   │   ├── db.rs               # 📐 SQLite schema（§2.8）
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

> ⚠️ 以下为**编写时的参考信息**，云服务接口可能调整。对接前以阿里云百炼官方最新文档为准。

- **WebSocket 实时 ASR**：`wss://dashscope.aliyuncs.com/api-ws/v1/inference`
    - 协议：二进制帧 + JSON 控制帧；要求 16kHz/16-bit/单声道 PCM；鉴权 Header `Authorization: Bearer <api_key>`。
- **HTTP 离线 ASR（M4 落地修订，2026-06）**：本应用采用 **Qwen3-ASR-Flash** 的 OpenAI 兼容同步接口。
    - 端点：`POST https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions`
    - 鉴权：Header `Authorization: Bearer <api_key>`，`Content-Type: application/json`。
    - 请求体：`{"model":"qwen3-asr-flash","messages":[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"data:audio/wav;base64,<BASE64>"}}]}],"stream":false}`（本地音频以 base64 data URL 内联，无需公网 URL）。
    - 响应：转写文本位于 `choices[0].message.content`。
    - 限制：单次音频 ≤ 10MB（base64 后），约 3-4 分钟；超长音频改用下面的大文件异步接口。
- **大文件离线 ASR（qwen3-asr-flash-filetrans，异步，M4 落地）**：仅接受**公网 URL**、不支持本地/base64 上传，故需先上传到用户 OSS。流程：
    1. **上传 OSS**（私有）：OSS V1 签名 PUT 到 `https://{bucket}.{endpoint}/{key}`；再生成**预签名 GET URL**（`OSSAccessKeyId`/`Expires`/`Signature`）供 DashScope 拉取。
    2. **提交任务**：`POST https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription`，Header `Authorization: Bearer`、`X-DashScope-Async: enable`；体 `{"model":"qwen3-asr-flash-filetrans","input":{"file_url":"<预签名URL>"},"parameters":{"channel_id":[0],"enable_itn":false}}` → 返回 `output.task_id`。
    3. **轮询**：`GET https://dashscope.aliyuncs.com/api/v1/tasks/{task_id}`（Bearer）→ `output.task_status` 为 `SUCCEEDED`/`FAILED`/`PENDING`/`RUNNING`；成功后取 `transcription_url`。
    4. **取结果**：GET `transcription_url`（24h 有效）→ JSON `{"transcripts":[{"text":"..."}]}`。
    - 支持超大/超长音频（文档称可达 12 小时）。OSS 对象不自动删除，建议在 OSS 配置生命周期规则定期清理。
- ⚠️ **原"HTTP 离线 ASR：POST /api/v1/services/audio/asr/transcription（multipart，500MB）"描述不准确**：该路径实际就是上面的**异步、需公网 URL** 接口（提交→轮询→取结果），**不接受本地文件字节的 multipart 同步上传**。因此 §2.2 的 `transcribe_offline(audio_data: Vec<u8>)` 由两个后端落地：短音频走同步 base64（`aliyun_bailian_offline`），大文件走"上传 OSS + 异步"（`aliyun_bailian_filetrans`）。
- **WebSocket 实时 ASR（M6 落地，已核对协议；2026-06 改用 Qwen-ASR）**：模型 **qwen3-asr-flash-realtime**，端点 `wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model=qwen3-asr-flash-realtime`，Header `Authorization: Bearer <key>`。**协议为 OpenAI-Realtime 风格**（不同于 Paraformer 的 run-task 协议）。
    - 客户端事件（JSON 文本帧）：连接后发 `session.update`（`session.input_audio_format="pcm"`、`sample_rate=16000`、`input_audio_transcription.language`、`turn_detection={type:"server_vad",threshold:0.0,silence_duration_ms:400}`）→ 持续发 `input_audio_buffer.append`（`audio` 为 **base64 PCM16**，16kHz/mono，每 100ms 一帧）→ 结束发 `session.finish`。
    - 服务端事件：`session.created`/`session.updated`（就绪）；`conversation.item.input_audio_transcription.text`（中间结果，`text`=已确认前缀 + `stash`=暂定尾部，合并为当前整句）；`conversation.item.input_audio_transcription.completed`（最终结果，`transcript`）；`error`（`error.code`/`error.message`）；`session.finished`（结束）。server_vad 自动断句，无需手动 commit。
    - 实现：握手 401/403 → AuthError 不重试；其它断开自动重连（≤3 次，1/2/4s 退避；音频在 MPSC 通道缓冲不丢）；全部失败回退离线（流式采集**同时写本地 WAV**，停止后离线转写）。
    - ⚠️ 注意：§2.7 默认 `api_endpoint` 仍是旧的 inference URL，对本模型不适用——后端默认使用上面的 realtime URL，仅当用户显式配置了含 `/realtime` 的端点才覆盖。
- 官方文档：录音文件识别（Qwen-ASR）https://help.aliyun.com/zh/model-studio/qwen-asr-api-reference ；实时识别（Qwen-ASR-Realtime，客户端/服务端事件）https://help.aliyun.com/zh/model-studio/qwen-asr-realtime-interaction-process 。

### 附录 C：qwen-asr（本地 ASR 引擎）技术约束

| 项目               | 内容                                             |
|------------------|------------------------------------------------|
| **仓库**           | https://github.com/huanglizhuo/QwenASR         |
| **模型**           | Qwen3-ASR（阿里巴巴通义千问团队）                          |
| **实现语言**         | 纯 Rust，无 C/C++ FFI 依赖                          |
| **推理设备**         | CPU-only，不支持 GPU                               |
| **支持模式**         | 仅离线整段识别（不支持流式）                                 |
| **音频输入**         | 16kHz, 16-bit, 单声道 PCM WAV                     |
| **模型规格**         | base (~200MB) / small (~500MB) / medium (~1GB) |
| **Rust Edition** | 需确认与 Edition 2024 兼容                           |
| **Send + Sync**  | 模型实例必须满足（`AsrBackend` trait bound）             |

### 附录 D：参考资源

- [GPUI 官方仓库](https://github.com/zed-industries/zed)
- [GPUI 示例代码](https://github.com/zed-industries/zed/tree/main/crates/gpui/examples)
- [gpui-component 组件库](https://longbridge.github.io/gpui-component)
- [cpal 音频采集示例](https://github.com/RustAudio/cpal/tree/master/examples)
- [rubato 重采样库](https://github.com/HEnquist/rubato)

---

> **文档版本**：v4.0 — AI Agent 可执行版（契约/实现分层重构）
> **最后更新**：2026-06-12
> **作者**：VoxInk 产品团队
> **协议**：Apache License 2.0
>
> **v4.0 主要变更**：
> 1. 新增 [§2 核心契约](#2-核心契约-core-contracts)，集中所有跨模块类型作为唯一权威来源（Tier 1）。
> 2. 剥离里程碑中的命令式实现代码，转为行为需求 + 契约引用（Tier 2/3 分层）。
> 3. 执行指南新增"权威层级与代码冲突处理"规则。
> 4. 依赖版本收敛为 §1.4 单一来源，去除硬钉与虚构版本号。
> 5. 修复内部矛盾：加密库统一 `aes-gcm`（移除 `ring`）；配置格式统一 TOML（修正 `config.json`/JSON 示例）；版本号矛盾。
> 6. `AsrError` 错误分类与传输库解耦（§2.4）。
> 7. 调整里程碑排序：ASR 契约在 M4 一次落地，消除 M7 大重构 churn。
