# CLAUDE.md

VoxInk —— Rust + GPUI 的语音转 Prompt 桌面应用（Windows 优先）。本文件记录在本项目中已固化的开发习惯与约定，供后续开发保持一致。**这些约定是硬性要求，优先于默认行为。**

## 1. 技术栈与硬约束

- **语言/框架**：Rust edition 2024 + GPUI（`zed-industries/zed`）+ gpui-component（`longbridge/gpui-component`），git 依赖。
- **纯 Rust，零 C/系统工具链**：这是项目第一原则。
  - TLS 一律用 **rustls**（reqwest `rustls-tls`、tungstenite `rustls-tls-webpki-roots`），**不要引入 OpenSSL/ring/native-tls**。
  - 加密用纯 Rust：AES-256-GCM + HKDF-SHA256（`aes-gcm`/`hkdf`/`sha2`），**不要用 ring**。
  - SQLite 用 `rusqlite` 的 `bundled` 特性（编译期自带 FTS5 + trigram），**不要依赖系统 sqlite**。
  - 任何需要 BLAS/C 链接的依赖一律拒绝（本地 ASR 引擎 `qwen-asr` 已因此被永久移除，勿重新引入，勿引用 `ModelNotFound`/`InferenceError`/`local_model_*`）。
- **异步**：tokio 多线程运行时。GPUI 执行器 ≠ tokio——需要 reactor 的网络调用（reqwest/tungstenite）必须在 tokio 上跑：runtime handle 存为 gpui Global（`GlobalTokioHandle`），用 `handle.spawn(...)` + `oneshot` 把结果送回 gpui 前台。

## 2. 开发流程与门禁

- **质量门禁（每次改完必须全绿，提交前必跑）**：
  ```
  cargo check
  cargo clippy -- -D warnings    # 必须零警告
  cargo test
  ```
  - 门禁用**默认 targets**的 clippy。`--all-targets` 会触发一个**既有**的 `items_after_test_module`（diagnostics.rs），那不是新引入的问题。
  - 坑：`cargo clippy` **不会**重建 `target/debug/VoxInk.exe`（只产出 check 制品）。要真机测试改动，先 `cargo build`/`cargo run` 再启动 exe，否则跑的是旧 exe。
- **里程碑节奏**：按 `docs/PRD.md`（M1–M12，M8 为永久空缺号，不重排）逐里程碑开发。一个里程碑 = 一个提交单元；**每个里程碑结束后停下、等用户人工验证，不要自动进入下一个**。
- **PRD 是契约**：§2「核心契约」类型权威。实现若偏离 PRD（如线程模型、API 端点、schema），**先改 PRD 再写码**，保持文档与代码同步。
- **提交规范**：commit message 用中文 `type(scope): 描述`（见 git log）。只有用户明确要求时才提交/推送。

## 3. GPUI / gpui-component 关键规则（踩过坑，务必遵守）

- **顶层视图必须包 `gpui_component::Root`**：`cx.new(|cx| Root::new(view, window, cx))`。不包能编译但渲染 Input/主题组件时运行期 panic（表现为 Windows `STATUS_STACK_BUFFER_OVERRUN`）。
- **覆盖层不会被 Root 自动渲染**：`Root::render` 只画被包视图 + tooltip/native-menu。Sheet/Dialog/Notification(toast) 层必须由顶层视图（`VoxInk::render`）自己调 `Root::render_sheet_layer/render_dialog_layer/render_notification_layer(window, cx)` 并 `.children(...)` 挂上，否则 `push_notification`/`open_dialog` 只改状态不绘制（toast 静默失效）。
- **双重借用 panic**（"cannot update X while it is already being updated"，表现为 STATUS_STACK_BUFFER_OVERRUN）：
  - `WindowHandle<Root>::update` 会**租借 Root**；闭包内再调 `push_notification`（内部又 update Root）→ panic。需要 `&mut Window + &mut App` 时改用 `AnyWindowHandle::update`（不租借 Root）：`let any = *window;`（WindowHandle Deref 到 AnyWindowHandle，二者 Copy），再在其中 `view.update(...)`。tray/hotkey 调 `toggle_recording` 等会发 toast 的方法都走这条路径。
  - **子视图在父视图 update 中不可 read/update 父视图**。子视图需要父状态时，让父在自己的方法里把状态 push 给子（镜像字段），不要让子 `parent.read(cx)`。
- **全屏覆盖层/scrim 必须 `.occlude()`**，否则点击会穿透到下层主视图控件（设置面板「完成」按钮曾穿透触发录音）。
- **AsyncApp 返回值**：`AsyncApp::update`、同步 `Entity::update`/`Context::update` 直接返回 `R`（不是 Result）；只有 `WindowHandle::update` 和 `Entity::update_in` 返回 Result。对 AsyncApp 用 `let _ = cx.update(...)` 会触发 clippy `let_unit_value`，应作为裸语句调用。
- **读源码再写**：API 漂移快。本机 cargo home 是 `D:\Repository\cargo`（非 `~/.cargo`），源码 checkout 在 `D:\Repository\cargo\git\checkouts\...`，写 GPUI 代码前先读对应版本源码/examples（含 gpui-component `crates/ui`、`crates/story`）。

## 4. Windows 平台约定

- **隐藏到托盘是 Win32-only**：gpui 在 Windows 上 `Window::hide` 是 no-op、minimize 未公开。用 `raw-window-handle` 取 HWND，直接 `ShowWindow(hwnd, SW_HIDE/SW_SHOW)`。`on_window_should_close` 取消关闭并隐藏。
- **托盘/全局热键事件**靠 gpui 前台 `cx.spawn` 轮询循环（~100–150ms）读 `TrayIconEvent::receiver()`/`MenuEvent::receiver()`/`GlobalHotKeyEvent::receiver()`——Windows 上这些库的消息由 gpui 的 win32 消息循环泵送。
- **!Send 可以存进 gpui Global**：Global 无 Send bound。cpal Stream、TrayIcon、HotKeyManager 等 !Send 类型存 Global / 进视图字段都没问题（gpui 视图不要求 Send）。
- **日志默认过滤**保留 `gpui_windows::events=off,gpui_windows::window=off,gpui::window=off`：屏蔽 gpui Windows 后端的伪 ERROR（右键 GetLastError==0、teardown 无效句柄），别当真错误删掉。用 `RUST_LOG` 覆盖。
- 文本输入框右键菜单是 OS 原生 Win32 弹窗（NativeMenu），样式无法跟随主题、光标为 I-beam，均为上游行为，**不是 bug，别去"修"**。

## 5. UI 设计系统（"现代 / 简洁 / 小清新"）

- **品牌色 teal**（`hsl(172,58,43)`）定义在 `src/theme.rs`，用 `const fn hsl()` 直接构造 `Hsla{}` 字面量（gpui 的 `hsla()` 非 const）。
- **`theme::apply()` 顺序强制**：先 `Theme::change`/`sync_system_appearance`，**再** `apply_brand(cx)`——因为 `Theme::change` 会从基础配色重置 `colors`，品牌覆盖必须在其后重应用。注意 `Theme` 有同名 `list: ListSettings` 字段会遮蔽颜色 token，颜色要写 `t.colors.list`。
- **图标用线性 Icon，不用 emoji**：gpui-component `Icon`/`IconName`。无内置麦克风图标，故 `src/assets.rs` 的 `VoxInkAssets` 组合 AssetSource 提供自绘 `assets/icons/mic.svg`（`Icon::empty().path("icons/mic.svg")`），其余委托给 `gpui-component-assets`。
- **无边框窗口 + 自绘标题栏**：`WindowOptions.titlebar` 用 `appears_transparent: true` 且**保留 `title: Some("VoxInk")`**（否则任务栏/Alt-Tab 空、截图脚本按标题查不到窗）。标题栏手写（未用 gpui-component `TitleBar`）。**关键坑**：标了 `WindowControlArea::Drag` 的元素整片是 HTCAPTION，其**子元素**点击会被系统当拖窗吞掉——可点击控件（齿轮）必须与拖拽区是**兄弟**节点，不能是其子节点。窗口 min/max/close 按钮用 `div.window_control_area(...)` + `IconName::Window*`，**无需 on_click**（系统按 NC 命中码处理）。
- **设置面板 = 全屏 OVERLAY 子视图**（非 Dialog/Sheet，避免父子双重借用），左侧标签栏（`SettingsTab` enum）分类。滚动条必须用 `gpui_component::scroll::Scrollbar`（gpui 原生 overflow 只滚不画条）。
- **可视化验证**：用 **`ui-verify` skill**（`.claude/skills/ui-verify/`）。`ui-shot.ps1` 一次调用完成 杀进程→临时改 config（直接可见启动 + 目标主题，省去 force-show / 暗色还原的多步操作）→可选构建→启动→可选点击/滚动→截图→强杀→还原 config。须用 **Windows PowerShell 5.1**（`powershell.exe`，`System.Drawing` 不在 pwsh）。改了 Rust 代码加 `-Build`；暗色用 `-Theme dark`。截完用 Read 读回 PNG 检查。无麦克风只能验证静态 UI。

## 6. 模块架构速览

- `src/state.rs` —— RecordingState / TranscriptionMode / AppState（PRD §2.1 契约）。
- `src/config.rs` —— TOML 配置（`%APPDATA%\VoxInk\config.toml`），api_key/OSS secret 用 AES-256-GCM 按字段加密；旧字段靠 `serde(default)` 容错。
- `src/app.rs` —— 主视图 `VoxInk`，两栏布局（左侧栏 documents 列表 + 右侧 录制/编辑/底栏）。
- `src/asr/` —— ASR 抽象：`traits.rs`（`AsrBackend`，用 `#[async_trait]` 因原生 async-fn-in-trait 非 dyn-safe）+ `registry.rs` + `backends/`。**App 只依赖 trait + registry，不 import 具体 backend**。后端按能力（`supports_streaming`/`supports_offline`）选择；每模式独立配置（`streaming_backend`/`offline_backend` + 每后端 `BackendSettings`）。
- `src/audio/` —— cpal 采集 → ringbuf → 独立 std::thread（非 tokio）下混/重采样到 16kHz → hound WAV / chunk_sender 流式 PCM16 帧。
- `src/history/db.rs` —— 单表 `records`（文档模型）+ FTS5 trigram；`segments` 表归档录音（绝对路径，`ON DELETE CASCADE` + 须 `PRAGMA foreign_keys=ON`）。
- `src/{tray,hotkey,autolaunch,settings,theme,i18n,diagnostics,assets}.rs`。
- i18n：`rust-i18n`，`locales/app.yaml`（zh-CN 兜底 + en），`i18n::tr(key)`；与 gpui-component 内置字符串共享 locale。

## 7. ASR / 音频要点

- **云端百炼 Qwen3-ASR**：离线 `qwen3-asr-flash`（OpenAI 兼容 sync，base64，≤10MB）；大文件 `qwen3-asr-flash-filetrans`（async + 需先传用户 OSS 私有桶，OSS V1 HMAC-SHA1 签名）；实时 `qwen3-asr-flash-realtime`（OpenAI-Realtime 风格 WS，`input_audio_buffer.append` 发 base64 PCM16，server_vad 自动分段）。China 区 host（`dashscope.aliyuncs.com`），intl host 对 China key 返回 401。
- **自建服务** `qwen3_asr_selfhosted`：唯一同时 `supports_streaming + supports_offline`（一个 server 双模，大文件无需 OSS）。config `endpoint` = base URL，后端自行派生 HTTP/WS 路径。api_key 可选。
- **per-backend env 兜底**：单一真源 `app::api_key_env_var(backend_id)`（自建→`QWEN3_ASR_API_KEY`，其余→`DASHSCOPE_API_KEY`）。config 空时回退对应 env。新后端在此登记自己的 env 变量。
- **403 区分**：模型权限 403（`Model.AccessDenied`）映射为 `InvalidConfig`（提示开通模型），只有 401 才是 `AuthError`——别把权限错当成 key 无效误导用户。

## 8. 录音音频归档

- 默认持久化（`save_audio=true`），根目录 `%LOCALAPPDATA%\VoxInk\recordings\{record_id}\{时间戳}_{短id}.wav`（大媒体放本地数据目录，不放可漫游 config）。
- `segments.file_path` 存**绝对路径**：改音频路径只对新录音生效、旧文件留原处，无迁移逻辑。
- 转写成败都保留音频（失败 text 空，可重转写）。仅 WAV 格式。
- SQLite 删不了文件：删记录/过期清理时，DB 返回待删路径，由**应用层**删文件，再删 DB 行。启动时 `cleanup_audio_on_startup`（过期片段 + 孤儿 wav + 旧版临时 wav）。
