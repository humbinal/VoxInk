//! 主界面 View —— 双栏布局（2026-06-14 重设计）：左侧常驻识别记录栏 + 右侧当前记录编辑区。
//!
//! - 左栏：「＋ 新建」+ 搜索 + 按时间分组（今天/昨天/近 7 天/近 30 天）的记录列表；当前项高亮、可删除。
//! - 右栏：录音按钮（对当前记录续录追加）+ 模式切换 + 状态 + 文本编辑区 + 字数/复制。
//! - 每条记录是一个可编辑、可续录的文档（§2.8 单表 records）；启动默认打开最近一条。
//! - 录制中禁用「新建」与切换记录；正文手动编辑防抖自动保存。

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Local};
use gpui::{
    div, ease_in_out, prelude::*, px, rgb, white, Animation, AnimationExt, AnyElement, App,
    ClickEvent, Context, Entity, Focusable, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex, ActiveTheme, Root, WindowExt,
};

use crate::asr::traits::StreamingResult;
use crate::asr::{AsrConfig, AsrError, BackendRegistry};
use crate::audio::{AudioError, Recorder, StreamingCapture};
use crate::config::VoxInkConfig;
use crate::history::db::Record;
use crate::history::GlobalHistory;
use crate::i18n::tr;
use crate::settings::{SettingsEvent, SettingsView};
use crate::state::{AppState, RecordingState, TranscriptionMode};

/// 以全局形式承载持久化配置，便于跨 View 读写、退出时统一保存。
pub struct GlobalConfig(pub VoxInkConfig);

impl gpui::Global for GlobalConfig {}

/// Tokio 运行时句柄，供把网络任务派发到 Tokio 执行（reqwest 需要 reactor）。
pub struct GlobalTokioHandle(pub tokio::runtime::Handle);

impl gpui::Global for GlobalTokioHandle {}

/// 左栏宽度（px）。
const SIDEBAR_WIDTH: f32 = 230.0;
/// 手动编辑后自动保存的防抖时延。
const AUTOSAVE_DEBOUNCE_MS: u64 = 800;

/// VoxInk 主窗口视图。
pub struct VoxInk {
    /// 应用全局状态（§2.1）。
    state: AppState,
    /// 文本编辑器状态（gpui-component 多行输入）——显示/编辑当前记录正文。
    editor: Entity<InputState>,
    /// 左栏搜索框。
    search: Entity<InputState>,
    /// 左栏记录列表（按 updated_at 倒序；受搜索过滤）。
    records: Vec<Record>,
    /// 当前打开的记录 id（录音追加与编辑均作用于它）。
    current_record_id: String,
    /// 复制成功后的短暂反馈标记（1.5s 后复位）。
    copied: bool,
    /// 当前离线录音会话（None 表示未在录音）。
    recorder: Option<Recorder>,
    /// 当前实时流式会话（None 表示未在流式录音）。
    streaming: Option<StreamingSession>,
    /// 实时识别失败后是否已切换到"停止后离线转写"。
    streaming_fallback: bool,
    /// 停止流式时捕获的录音时长（秒），供异步完成时累加到记录时长。
    streaming_duration_secs: u32,
    /// 自动保存防抖代际计数（仅最新一次定时器生效）。
    autosave_gen: u64,
    /// 设置面板覆盖层视图（M11）。
    settings: Entity<SettingsView>,
    /// 是否显示设置面板覆盖层。
    show_settings: bool,
    /// 订阅句柄（编辑器/搜索/设置事件）保活。
    _subs: Vec<Subscription>,
}

/// 实时流式会话：持有流式采集句柄（cpal 流在主线程）。
struct StreamingSession {
    capture: StreamingCapture,
}

impl VoxInk {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder(tr("editor.placeholder"))
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("sidebar.search")));

        // 初始转录模式与主题取自持久化配置。
        let mut state = AppState::default();
        let mut theme_str = "system".to_string();
        if let Some(global) = cx.try_global::<GlobalConfig>() {
            state.transcription_mode = global.0.asr.default_mode;
            theme_str = global.0.general.theme.clone();
        }
        // 启动应用主题（M11 任务 11.2）。
        crate::theme::apply(&theme_str, window, cx);

        // 启动默认打开最近一条记录；库为空则新建一条空记录（§4.3.3）。
        let (current_record_id, initial_text, records) = match cx.try_global::<GlobalHistory>() {
            Some(global) => {
                let db = &global.0;
                let current = match db.most_recent() {
                    Ok(Some(r)) => Some(r),
                    _ => db.create_record().ok(),
                };
                let records = db.search_records("").unwrap_or_default();
                match current {
                    Some(r) => (r.id, r.text, records),
                    None => (String::new(), String::new(), records),
                }
            }
            None => (String::new(), String::new(), Vec::new()),
        };

        // 载入当前记录正文到编辑器。
        if !initial_text.is_empty() {
            editor.update(cx, |s, cx| s.set_value(initial_text, window, cx));
        }

        // 启动时聚焦编辑器，便于直接键盘输入。
        let focus_handle = editor.focus_handle(cx);
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });

        // 订阅：编辑器变更 → 防抖自动保存；搜索变更 → 重查列表。
        let editor_sub = cx.subscribe(&editor, |this, _ed, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_autosave(cx);
            }
        });
        let search_sub = cx.subscribe(&search, |this, _s, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh_records(cx);
            }
        });

        // 设置面板覆盖层 + 关闭事件订阅（M11）。
        let settings = cx.new(|scx| SettingsView::new(window, scx));
        let settings_sub = cx.subscribe(&settings, |this, _s, event: &SettingsEvent, cx| {
            match event {
                SettingsEvent::Closed => {
                    this.show_settings = false;
                    cx.notify();
                }
            }
        });

        Self {
            state,
            editor,
            search,
            records,
            current_record_id,
            copied: false,
            recorder: None,
            streaming: None,
            streaming_fallback: false,
            streaming_duration_secs: 0,
            autosave_gen: 0,
            settings,
            show_settings: false,
            _subs: vec![editor_sub, search_sub, settings_sub],
        }
    }

    fn is_idle(&self) -> bool {
        self.state.recording_state == RecordingState::Idle
    }

    // ───────────────────────────── 记录管理（M10 重设计）─────────────────────────────

    /// 按当前搜索词重查记录列表并刷新 UI。
    fn refresh_records(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        if let Some(global) = cx.try_global::<GlobalHistory>() {
            self.records = global.0.search_records(&query).unwrap_or_default();
        }
        cx.notify();
    }

    /// 把当前编辑器正文写回当前记录（保持其 mode/duration 不变）——用于手动编辑自动保存、切换前刷写。
    /// 正文无改动时直接跳过：既省一次写，也避免无谓地刷新 updated_at（纯选中不应触发写）。
    fn flush_editor_to_record(&mut self, cx: &Context<Self>) {
        if self.current_record_id.is_empty() {
            return;
        }
        let text = self.editor.read(cx).value().to_string();
        let id = self.current_record_id.clone();
        if let Some(global) = cx.try_global::<GlobalHistory>()
            && let Some(rec) = global.0.get_record(&id).ok().flatten()
        {
            if rec.text == text {
                return; // 无改动，不写
            }
            if let Err(e) = global.0.save_record(&id, &text, &rec.mode, rec.duration_secs) {
                tracing::error!("自动保存失败: {e:#}");
            }
        }
    }

    /// 录制完成后把编辑器正文写回当前记录，并累加本次时长、记录模式。
    fn persist_after_recording(&mut self, mode: &str, added_secs: u32, cx: &mut Context<Self>) {
        if self.current_record_id.is_empty() {
            return;
        }
        let text = self.editor.read(cx).value().to_string();
        let id = self.current_record_id.clone();
        if let Some(global) = cx.try_global::<GlobalHistory>() {
            let db = &global.0;
            let base = db
                .get_record(&id)
                .ok()
                .flatten()
                .map(|r| r.duration_secs)
                .unwrap_or(0);
            if let Err(e) = db.save_record(&id, &text, mode, base + added_secs) {
                tracing::error!("保存记录失败: {e:#}");
            }
        }
        self.refresh_records(cx);
    }

    /// 防抖自动保存：仅空闲时生效（录制中由完成时统一写回）。
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        if !self.is_idle() {
            return;
        }
        self.autosave_gen += 1;
        let generation = self.autosave_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(AUTOSAVE_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.autosave_gen == generation && this.is_idle() {
                    this.flush_editor_to_record(cx);
                    this.refresh_records(cx);
                }
            });
        })
        .detach();
    }

    /// 新建一条空记录并切为当前（录制中禁用）。
    fn on_new_record(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_idle() {
            window.push_notification("录制中，暂不能新建记录", cx);
            return;
        }
        self.flush_editor_to_record(cx);
        let new_rec = match cx.try_global::<GlobalHistory>() {
            Some(global) => match global.0.create_record() {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("新建记录失败: {e:#}");
                    window.push_notification("新建记录失败", cx);
                    return;
                }
            },
            None => return,
        };
        self.current_record_id = new_rec.id;
        self.editor.update(cx, |s, cx| s.set_value("", window, cx));
        self.refresh_records(cx);
        let focus_handle = self.editor.focus_handle(cx);
        focus_handle.focus(window, cx);
    }

    /// 选择并载入一条记录到编辑器（录制中禁用切换）。
    fn select_record(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.current_record_id {
            return;
        }
        if !self.is_idle() {
            window.push_notification("录制中，暂不能切换记录", cx);
            return;
        }
        self.flush_editor_to_record(cx);
        let rec = cx
            .try_global::<GlobalHistory>()
            .and_then(|g| g.0.get_record(&id).ok().flatten());
        if let Some(rec) = rec {
            self.current_record_id = rec.id;
            self.editor.update(cx, |s, cx| s.set_value(rec.text, window, cx));
        }
        self.refresh_records(cx);
    }

    /// 删除一条记录；若删的是当前记录，则切到最近一条（无则新建空记录）。
    fn delete_record(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_idle() {
            window.push_notification("录制中，暂不能删除记录", cx);
            return;
        }
        let deleting_current = id == self.current_record_id;
        if let Some(global) = cx.try_global::<GlobalHistory>()
            && let Err(e) = global.0.delete_record(&id)
        {
            tracing::error!("删除记录失败: {e:#}");
            window.push_notification("删除记录失败", cx);
            return;
        }

        if deleting_current {
            // 切到剩余最近一条；都没有了就新建一条空记录。
            let next = match cx.try_global::<GlobalHistory>() {
                Some(global) => {
                    let db = &global.0;
                    match db.most_recent() {
                        Ok(Some(r)) => Some(r),
                        _ => db.create_record().ok(),
                    }
                }
                None => None,
            };
            match next {
                Some(rec) => {
                    self.current_record_id = rec.id;
                    self.editor.update(cx, |s, cx| s.set_value(rec.text, window, cx));
                }
                None => {
                    self.current_record_id = String::new();
                    self.editor.update(cx, |s, cx| s.set_value("", window, cx));
                }
            }
        }
        self.refresh_records(cx);
    }

    fn max_recording_seconds(&self, cx: &Context<Self>) -> u32 {
        cx.try_global::<GlobalConfig>()
            .map(|g| g.0.asr.max_recording_seconds)
            .unwrap_or(600)
    }

    /// 开始录音：构建 Recorder，进入 Recording 状态，启动计时器。
    fn start_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match Recorder::start() {
            Ok(recorder) => {
                self.recorder = Some(recorder);
                self.state.recording_state = RecordingState::Recording;
                self.state.recording_duration_secs = 0;
                tracing::info!("开始录音");
                cx.notify();
                let max = self.max_recording_seconds(cx);
                self.spawn_timer(window, cx, max);
            }
            Err(e) => {
                tracing::error!("启动录音失败: {e}");
                let msg = match e {
                    AudioError::NoInputDevice => "未检测到麦克风，请检查录音设备",
                    _ => "无法开始录音，请重试",
                };
                window.push_notification(msg, cx);
            }
        }
    }

    /// 停止录音：收尾 WAV，进入识别阶段。`auto` 表示是否因超时自动触发。
    fn stop_recording(&mut self, window: &mut Window, cx: &mut Context<Self>, auto: bool) {
        let Some(recorder) = self.recorder.take() else {
            self.state.recording_state = RecordingState::Idle;
            self.state.recording_duration_secs = 0;
            cx.notify();
            return;
        };
        match recorder.stop() {
            Ok(outcome) => {
                if auto {
                    window.push_notification(
                        format!(
                            "已达最长录音时长，已自动停止（{}s）",
                            outcome.duration.as_secs()
                        ),
                        cx,
                    );
                }
                // 录音完成后自动触发离线转写（任务 4.4）。
                let duration_secs = outcome.duration.as_secs() as u32;
                self.start_transcription(window, cx, outcome.path, duration_secs);
            }
            Err(e) => {
                tracing::error!("停止录音失败: {e}");
                window.push_notification("停止录音时出错", cx);
                self.state.recording_state = RecordingState::Idle;
                self.state.recording_duration_secs = 0;
                cx.notify();
            }
        }
    }

    /// 进入 Processing 并在 Tokio 后台执行离线转写，结果回主线程追加到编辑器（任务 4.4/4.5）。
    fn start_transcription(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        wav_path: PathBuf,
        duration_secs: u32,
    ) {
        self.state.recording_state = RecordingState::Processing;
        cx.notify();
        window.push_notification("正在识别…", cx);

        let asr_config = runtime_asr_config(cx, false);
        let Some(global_handle) = cx.try_global::<GlobalTokioHandle>() else {
            tracing::error!("缺少 Tokio runtime 句柄，无法转写");
            self.state.recording_state = RecordingState::Idle;
            cx.notify();
            return;
        };
        let handle = global_handle.0.clone();

        cx.spawn_in(window, async move |this, cx| {
            // 网络请求在 Tokio 运行时执行（reqwest 需 reactor）；结果经 oneshot 回到 GPUI 前台。
            let (tx, rx) = tokio::sync::oneshot::channel();
            handle.spawn(async move {
                let result = run_offline_transcription(asr_config, wav_path).await;
                let _ = tx.send(result);
            });

            let outcome = rx.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.state.recording_state = RecordingState::Idle;
                this.state.recording_duration_secs = 0;
                match outcome {
                    Ok(Ok(text)) => {
                        append_text(&this.editor, &text, window, cx);
                        // 追加到当前记录并累加时长（任务 10.3）。
                        this.persist_after_recording("offline", duration_secs, cx);
                        window.push_notification("转写完成", cx);
                    }
                    Ok(Err(e)) => {
                        tracing::error!("离线转写失败: {e}");
                        window.push_notification(friendly_asr_error(&e), cx);
                    }
                    Err(_) => {
                        window.push_notification("转写任务已取消", cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }


    // ───────────────────────────── 实时流式（M6）─────────────────────────────

    /// 开始实时流式识别：流式采集 + WS 后端 + 增量结果回 UI。
    fn start_streaming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = runtime_asr_config(cx, true);
        if config.api_key.trim().is_empty() {
            window.push_notification(
                "未配置 API Key（设置环境变量 DASHSCOPE_API_KEY 或在设置中填写）",
                cx,
            );
            return;
        }
        let Some(handle) = cx.try_global::<GlobalTokioHandle>().map(|g| g.0.clone()) else {
            tracing::error!("缺少 Tokio runtime 句柄，无法实时识别");
            return;
        };

        let (audio_tx, audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<StreamingResult>(64);
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<Result<(), AsrError>>();

        let capture = match StreamingCapture::start(audio_tx) {
            Ok(capture) => capture,
            Err(e) => {
                tracing::error!("启动流式采集失败: {e}");
                let msg = match e {
                    AudioError::NoInputDevice => "未检测到麦克风，请检查录音设备",
                    _ => "无法开始录音，请重试",
                };
                window.push_notification(msg, cx);
                return;
            }
        };
        self.streaming = Some(StreamingSession { capture });
        self.streaming_fallback = false;
        self.streaming_duration_secs = 0;
        self.state.recording_state = RecordingState::Recording;
        self.state.recording_duration_secs = 0;
        self.state.pending_text.clear();
        cx.notify();
        window.push_notification("实时识别中…", cx);

        // 用配置选定的流式后端（用户在设置中选择）；只依赖 trait + 注册表。
        let streaming_backend_id = config.backend_id.clone();
        handle.spawn(async move {
            let registry = BackendRegistry::with_builtins();
            let result = match registry.get(&streaming_backend_id) {
                Some(backend) => backend.transcribe_streaming(&config, audio_rx, result_tx).await,
                None => Err(AsrError::InvalidConfig(format!(
                    "未找到后端: {streaming_backend_id}"
                ))),
            };
            let _ = done_tx.send(result);
        });

        let max = self.max_recording_seconds(cx);
        self.spawn_timer(window, cx, max);

        // 前台读取增量结果并更新 UI；通道关闭后取后端最终状态。
        cx.spawn_in(window, async move |this, cx| {
            while let Some(result) = result_rx.recv().await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.apply_streaming_result(result, window, cx);
                });
            }
            let done = done_rx
                .await
                .unwrap_or_else(|_| Err(AsrError::WebSocketError("任务通道中断".to_string())));
            let _ = this.update_in(cx, |this, window, cx| {
                this.on_streaming_backend_done(done, window, cx);
            });
        })
        .detach();
    }

    /// 停止实时流式：收尾 WAV → 已失败则离线转写，否则等待最终结果。
    fn stop_streaming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.streaming.take() else {
            self.finish_to_idle(cx);
            return;
        };
        match session.capture.stop() {
            Ok(outcome) => {
                // 捕获时长供异步完成时累加到记录。
                self.streaming_duration_secs = outcome.duration.as_secs() as u32;
                self.state.recording_state = RecordingState::Processing;
                self.state.recording_duration_secs = 0;
                cx.notify();
                if self.streaming_fallback {
                    window.push_notification("实时识别失败，正在离线转写…", cx);
                    let duration_secs = outcome.duration.as_secs() as u32;
                    self.start_transcription(window, cx, outcome.path, duration_secs);
                } else {
                    // 关闭音频通道触发后端 finish-task → 最终结果 → done（转 Idle）。
                    window.push_notification("正在生成最终结果…", cx);
                }
            }
            Err(e) => {
                tracing::error!("停止流式采集失败: {e}");
                window.push_notification("停止录音时出错", cx);
                self.finish_to_idle(cx);
            }
        }
    }

    /// 应用一条增量识别结果（§4.2.1）。
    fn apply_streaming_result(
        &mut self,
        result: StreamingResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if result.is_final {
            // 整句稳定：固化到编辑器（即追加到当前记录正文），清空 pending。
            append_text(&self.editor, &result.delta_text, window, cx);
            self.state.pending_text.clear();
        } else {
            // 未稳定：替换 pending（DashScope 发整句而非增量）。
            self.state.pending_text = result.delta_text;
        }
        cx.notify();
    }

    /// 后端结束处理：成功转 Idle 并把整段写回当前记录；鉴权失败提示；其它失败标记回退离线。
    fn on_streaming_backend_done(
        &mut self,
        done: Result<(), AsrError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 提交残留的未固化 pending。
        if !self.state.pending_text.is_empty() {
            let pending = std::mem::take(&mut self.state.pending_text);
            append_text(&self.editor, &pending, window, cx);
        }

        match done {
            Ok(()) => {
                // 把当前记录（含本次追加）写回 + 累加时长（任务 10.3）。
                let added = self.streaming_duration_secs;
                self.persist_after_recording("streaming", added, cx);
                window.push_notification("识别完成", cx);
                self.finish_to_idle(cx);
            }
            Err(AsrError::AuthError) => {
                if let Some(session) = self.streaming.take() {
                    let _ = session.capture.stop();
                }
                window.push_notification("API Key 无效，请检查后重试", cx);
                self.finish_to_idle(cx);
            }
            Err(e) => {
                tracing::warn!("实时识别失败: {e}");
                self.streaming_fallback = true;
                if self.streaming.is_some() {
                    // 用户仍在录音：保持录制，停止后用完整 WAV 离线转写。
                    window.push_notification("实时识别失败，已切换离线，停止后将转写", cx);
                    cx.notify();
                } else {
                    self.finish_to_idle(cx);
                }
            }
        }
    }

    /// 复位到 Idle 并清理流式状态。
    fn finish_to_idle(&mut self, cx: &mut Context<Self>) {
        self.state.recording_state = RecordingState::Idle;
        self.state.recording_duration_secs = 0;
        self.state.pending_text.clear();
        self.streaming = None;
        self.streaming_fallback = false;
        cx.notify();
    }

    /// 每秒递增录音时长并刷新 UI；达到上限时自动停止（任务 3.6）。
    fn spawn_timer(&self, window: &mut Window, cx: &mut Context<Self>, max_secs: u32) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(1))
                    .await;
                let stop = this
                    .update_in(cx, |this, window, cx| {
                        if this.state.recording_state != RecordingState::Recording {
                            return true;
                        }
                        this.state.recording_duration_secs += 1;
                        cx.notify();
                        if this.state.recording_duration_secs >= max_secs {
                            this.stop_capture(window, cx, true);
                            return true;
                        }
                        false
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    /// 录音时长格式化为 `MM:SS`。
    fn duration_label(&self) -> String {
        let secs = self.state.recording_duration_secs;
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }

    /// 当前状态的文字与指示色（§6.3）。
    fn status(&self) -> (SharedString, gpui::Rgba) {
        match self.state.recording_state {
            RecordingState::Idle => (tr("status.idle").into(), rgb(0x27AE60)),
            RecordingState::Recording => (tr("status.recording").into(), rgb(0xE74C3C)),
            RecordingState::Processing => (tr("status.processing").into(), rgb(0xF39C12)),
        }
    }

    /// 录音按钮点击：Idle 开始录音 / Recording 停止录音（§4.1.2）。
    fn on_toggle_recording(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_recording(window, cx);
    }

    /// 切换录音状态（供录音按钮与系统托盘菜单调用）。
    pub fn toggle_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.state.recording_state {
            RecordingState::Idle => self.start_capture(window, cx),
            RecordingState::Recording => self.stop_capture(window, cx, false),
            // Processing 不可点击/不可切换，理论上不会到这里。
            RecordingState::Processing => {}
        }
    }

    /// 一键复制并粘贴：复制全部文本到剪贴板 + 模拟粘贴到前台应用（M9 任务 9.4）。
    /// 供全局快捷键 `copy_and_paste` 调用；典型场景是焦点在其它应用、VoxInk 隐藏于托盘。
    pub fn copy_and_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        tracing::info!(chars = text.chars().count(), "复制并粘贴热键触发");
        if text.is_empty() {
            window.push_notification("没有可复制的内容", cx);
            return;
        }
        match copy_to_clipboard(&text) {
            Ok(()) => crate::hotkey::simulate_paste(),
            Err(e) => {
                tracing::error!("复制失败: {e:#}");
                window.push_notification("复制失败", cx);
            }
        }
    }

    /// 按当前模式开始：实时 → 流式；离线 → 录 WAV。
    fn start_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.transcription_mode == TranscriptionMode::Streaming {
            self.start_streaming(window, cx);
        } else {
            self.start_recording(window, cx);
        }
    }

    /// 停止：有流式会话 → 停流式；否则停离线录音。
    fn stop_capture(&mut self, window: &mut Window, cx: &mut Context<Self>, auto: bool) {
        if self.streaming.is_some() {
            self.stop_streaming(window, cx);
        } else {
            self.stop_recording(window, cx, auto);
        }
    }

    fn on_select_mode(&mut self, mode: TranscriptionMode, cx: &mut Context<Self>) {
        if self.state.transcription_mode == mode {
            return;
        }
        self.state.transcription_mode = mode;

        // 同步到全局配置（退出时落盘）。
        if cx.try_global::<GlobalConfig>().is_some() {
            let mut updated = cx.global::<GlobalConfig>().0.clone();
            updated.asr.default_mode = mode;
            cx.set_global(GlobalConfig(updated));
        }

        tracing::info!(?mode, "切换转录模式");
        cx.notify();
    }

    fn on_copy(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        tracing::info!(chars = text.chars().count(), "复制按钮被点击");

        if text.is_empty() {
            window.push_notification("没有可复制的内容", cx);
            return;
        }

        match copy_to_clipboard(&text) {
            Ok(()) => {
                self.copied = true;
                cx.notify();
                window.push_notification("已复制到剪贴板", cx);

                // 1.5 秒后复位"✓ 已复制"反馈。
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1500))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.copied = false;
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(e) => {
                tracing::error!("复制失败: {e:#}");
                window.push_notification("复制失败", cx);
            }
        }
    }

    /// 打开设置面板覆盖层（M11）：用当前配置填充输入框后显示。
    fn on_open_settings(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |panel, pcx| panel.load_from_config(window, pcx));
        self.show_settings = true;
        cx.notify();
    }

    /// 导出全部记录为 JSON（写入配置目录，Toast 路径；任务 10.4）。
    fn on_export(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<PathBuf> {
            let global = cx
                .try_global::<GlobalHistory>()
                .ok_or_else(|| anyhow::anyhow!("历史数据库不可用"))?;
            let json = global.0.export_json()?;
            let dir = crate::history::db::HistoryDb::default_path()?
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            let path = dir.join(format!(
                "history_export_{}.json",
                Local::now().format("%Y%m%d_%H%M%S")
            ));
            std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
            Ok(path)
        })();

        match result {
            Ok(path) => {
                tracing::info!("历史已导出: {}", path.display());
                window.push_notification(format!("已导出到 {}", path.display()), cx);
            }
            Err(e) => {
                tracing::error!("导出历史失败: {e:#}");
                window.push_notification("导出失败", cx);
            }
        }
    }

    // ───────────────────────────── 渲染：左栏 ─────────────────────────────

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                // 标题 + 设置
                h_flex()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("🎙 VoxInk"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("export")
                                    .ghost()
                                    .label("⬇")
                                    .on_click(cx.listener(Self::on_export)),
                            )
                            .child(
                                Button::new("settings")
                                    .ghost()
                                    .label("⚙")
                                    .on_click(cx.listener(Self::on_open_settings)),
                            ),
                    ),
            )
            .child(div().px_3().pb_2().child(self.render_new_button(cx)))
            .child(div().px_3().pb_2().child(Input::new(&self.search)))
            .child(self.render_record_list(cx))
    }

    /// 「＋ 新建」按钮；录制中禁用（变灰、不可点）。
    fn render_new_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let is_idle = self.is_idle();
        let mut btn = div()
            .id("new-record")
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(34.))
            .rounded(px(8.))
            .bg(cx.theme().accent)
            .text_color(cx.theme().accent_foreground)
            .child(tr("sidebar.new"));
        if is_idle {
            btn = btn
                .cursor_pointer()
                .hover(|s| s.opacity(0.9))
                .on_click(cx.listener(Self::on_new_record));
        } else {
            btn = btn.opacity(0.5);
        }
        btn.into_any_element()
    }

    fn render_record_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex()
            .id("record-list")
            .flex_1()
            .w_full()
            .gap_1()
            .px_2()
            .pb_2()
            .overflow_y_scroll();

        if self.records.is_empty() {
            return list.child(
                div()
                    .px_2()
                    .py_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sidebar.empty")),
            );
        }

        let current = self.current_record_id.clone();
        let is_idle = self.is_idle();
        let mut last_bucket = "";
        for rec in &self.records {
            let bucket = record_bucket(&rec.created_at);
            if bucket != last_bucket {
                last_bucket = bucket;
                list = list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(bucket)),
                );
            }
            list = list.child(self.render_record_item(rec, &current, is_idle, cx));
        }
        list
    }

    fn render_record_item(
        &self,
        rec: &Record,
        current: &str,
        is_idle: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_current = rec.id == *current;
        let select_id = rec.id.clone();
        let del_id = rec.id.clone();
        let title = if rec.title.trim().is_empty() {
            crate::history::db::NEW_RECORD_TITLE.to_string()
        } else {
            rec.title.clone()
        };

        let mut body = div()
            .id(elem_id("recbody", &rec.id))
            .flex_1()
            .overflow_hidden()
            .child(
                v_flex()
                    .w_full()
                    .gap_0p5()
                    // 标题单行截断（省略号），避免长文本换行撑高列表项。
                    .child(div().w_full().text_sm().truncate().child(title))
                    .child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(mode_icon(&rec.mode))
                            .child(time_label(&rec.created_at)),
                    ),
            );
        if is_idle {
            body = body
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_record(select_id.clone(), window, cx)
                }));
        }

        let mut row = h_flex()
            .id(elem_id("rec", &rec.id))
            .w_full()
            .items_center()
            .gap_1()
            .px_2()
            .py_1p5()
            .rounded(px(6.))
            .overflow_hidden()
            .child(body);

        if is_current {
            row = row.bg(cx.theme().list_active);
        } else if is_idle {
            row = row.hover(|s| s.bg(cx.theme().muted));
        }

        if is_idle {
            row = row.child(
                div()
                    .id(elem_id("recdel", &rec.id))
                    .px_1()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .hover(|s| s.text_color(rgb(0xE74C3C)))
                    .child("✕")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.delete_record(del_id.clone(), window, cx)
                    })),
            );
        }
        row
    }

    // ───────────────────────────── 渲染：右栏 ─────────────────────────────

    /// 录音按钮：按状态变色/变字；Recording 时叠加脉冲呼吸动画；Processing 不可点击。
    fn render_record_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let (bg, label, clickable) = match self.state.recording_state {
            RecordingState::Idle => (rgb(0x27AE60), tr("record.start"), true),
            RecordingState::Recording => (rgb(0xE74C3C), tr("record.stop"), true),
            RecordingState::Processing => (rgb(0xF39C12), tr("record.processing"), false),
        };

        let mut button = div()
            .id("record-button")
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(48.))
            .rounded(px(8.))
            .bg(bg)
            .text_color(white())
            .text_lg()
            .child(label);

        if clickable {
            button = button
                .cursor_pointer()
                .hover(|s| s.opacity(0.92))
                .on_click(cx.listener(Self::on_toggle_recording));
        } else {
            button = button.opacity(0.85);
        }

        if self.state.recording_state == RecordingState::Recording {
            button
                .with_animation(
                    "record-pulse",
                    Animation::new(Duration::from_millis(1200))
                        .repeat()
                        .with_easing(ease_in_out),
                    |this, delta| {
                        // 三角波 0→1→0，营造脉冲呼吸感。
                        let t = 1.0 - (2.0 * delta - 1.0).abs();
                        this.opacity(0.6 + 0.4 * t)
                    },
                )
                .into_any_element()
        } else {
            button.into_any_element()
        }
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (status_text, status_color) = self.status();
        let is_streaming = self.state.transcription_mode == TranscriptionMode::Streaming;

        v_flex()
            .w_full()
            .gap_3()
            .px_4()
            .py_4()
            .items_center()
            .child(self.render_record_button(cx))
            // 转录模式切换：实时 / 离线
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("mode-streaming")
                            .when(is_streaming, |b| b.primary())
                            .when(!is_streaming, |b| b.outline())
                            .label(tr("mode.streaming"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.on_select_mode(TranscriptionMode::Streaming, cx)
                            })),
                    )
                    .child(
                        Button::new("mode-offline")
                            .when(!is_streaming, |b| b.primary())
                            .when(is_streaming, |b| b.outline())
                            .label(tr("mode.offline"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.on_select_mode(TranscriptionMode::Offline, cx)
                            })),
                    ),
            )
            // 状态指示：● 状态  MM:SS
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().text_color(status_color).child("●"))
                    .child(status_text)
                    .child(self.duration_label()),
            )
            // 实时识别未稳定文本（pending）：浅色显示以区分稳定结果（§4.2.1）。
            .when(!self.state.pending_text.is_empty(), |this| {
                this.child(
                    div()
                        .w_full()
                        .px_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("✍ {}", self.state.pending_text)),
                )
            })
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex_1().w_full().px_4().py_2().child(
            div()
                .size_full()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(px(6.))
                .child(Input::new(&self.editor).h_full().bordered(false)),
        )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let char_count = self.editor.read(cx).value().chars().count();

        h_flex()
            .justify_between()
            .items_center()
            .w_full()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(format!("{}: {char_count}", tr("footer.words")))
            .child(
                Button::new("copy")
                    .primary()
                    .label(if self.copied {
                        tr("footer.copied")
                    } else {
                        tr("footer.copy")
                    })
                    .on_click(cx.listener(Self::on_copy)),
            )
    }
}

impl Render for VoxInk {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // gpui-component 的 Notification/Dialog 浮层不由 Root::render 自动绘制——
        // 顶层内容视图须显式渲染并作为子元素加入，否则 push_notification 只改状态却看不到。
        let notification_layer = Root::render_notification_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);

        div()
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .size_full()
                    .child(self.render_sidebar(cx))
                    .child(
                        // 右栏：录制控制 + 编辑区 + 底栏。
                        v_flex()
                            .flex_1()
                            .h_full()
                            .child(self.render_controls(cx))
                            .child(self.render_editor(cx))
                            .child(self.render_footer(cx)),
                    ),
            )
            // 设置面板覆盖层（M11）：显示时盖在主界面之上。
            .when(self.show_settings, |this| this.child(self.settings.clone()))
            .children(notification_layer)
            .children(dialog_layer)
    }
}

/// 复制文本到系统剪贴板（任务 2.2，使用 `arboard`）。
///
/// 注：Windows/macOS 下设置后内容常驻系统剪贴板；Linux(X11) 的所有权语义不同，
/// 后续若支持 Linux 需保持 Clipboard 实例存活，留待相应里程碑处理。
fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("打开系统剪贴板失败")?;
    clipboard.set_text(text.to_owned()).context("写入剪贴板失败")?;
    Ok(())
}

/// 读取 WAV → 用配置选定的离线后端转写（在 Tokio 运行时执行）。
/// 后端由用户在设置中显式选择（`offline_backend`）；不再按音频大小自动切换。
async fn run_offline_transcription(
    config: AsrConfig,
    wav_path: PathBuf,
) -> Result<String, AsrError> {
    let audio = tokio::fs::read(&wav_path).await?;
    let backend_id = config.backend_id.clone();
    tracing::info!(%backend_id, bytes = audio.len(), "离线转写后端");

    let registry = BackendRegistry::with_builtins();
    let backend = registry
        .get(&backend_id)
        .ok_or_else(|| AsrError::InvalidConfig(format!("未找到离线后端: {backend_id}")))?;
    backend.transcribe_offline(&config, audio).await
}

/// 某后端在 api_key 留空时回退的环境变量名（"后端 → 环境变量" 的**单一映射**，
/// 由 [`runtime_asr_config`] 与设置面板提示共用）。新增后端时在此登记其专属变量。
pub(crate) fn api_key_env_var(backend_id: &str) -> &'static str {
    match backend_id {
        "qwen3_asr_selfhosted" => "QWEN3_ASR_API_KEY",
        // 阿里云百炼系（streaming/offline/filetrans）共用 DashScope Key。
        _ => "DASHSCOPE_API_KEY",
    }
}

/// 从持久化配置构造运行期 `AsrConfig`（供主视图与设置面板共用）。
/// 按 `want_streaming` 选用对应模式的后端（streaming_backend / offline_backend），并取该后端的独立配置；
/// 后端 api_key/OSS 为空时回退到环境变量（自建服务 `QWEN3_ASR_API_KEY`，百炼系 `DASHSCOPE_API_KEY`；OSS 用 `OSS_*`）。api_key 为内存明文（§5.3 不记录值）。
pub(crate) fn runtime_asr_config(cx: &App, want_streaming: bool) -> AsrConfig {
    let Some(global) = cx.try_global::<GlobalConfig>() else {
        return AsrConfig::default();
    };
    let asr = &global.0.asr;
    let backend_id = if want_streaming {
        asr.streaming_backend.clone()
    } else {
        asr.offline_backend.clone()
    };
    // 迁移兜底：配置里的后端 id 若已不在注册表（如旧版 generic_ws），回退到默认后端，
    // 避免运行期 "未找到后端" 错误。
    let backend_id = if BackendRegistry::with_builtins().get(&backend_id).is_some() {
        backend_id
    } else {
        let fallback = if want_streaming {
            "aliyun_bailian_streaming"
        } else {
            "aliyun_bailian_offline"
        };
        tracing::warn!(%backend_id, fallback, "配置的 ASR 后端不存在，回退到默认后端");
        fallback.to_string()
    };
    let bs = asr.backend(&backend_id);
    let or_env = |v: &str, env_key: &str| {
        if v.trim().is_empty() {
            std::env::var(env_key).unwrap_or_default()
        } else {
            v.to_string()
        }
    };
    AsrConfig {
        api_key: or_env(&bs.api_key, api_key_env_var(&backend_id)),
        backend_id,
        api_endpoint: bs.endpoint.clone(),
        language: asr.language.clone(),
        oss_endpoint: or_env(&bs.oss_endpoint, "OSS_ENDPOINT"),
        oss_bucket: or_env(&bs.oss_bucket, "OSS_BUCKET"),
        oss_access_key_id: or_env(&bs.oss_access_key_id, "OSS_ACCESS_KEY_ID"),
        oss_access_key_secret: or_env(&bs.oss_access_key_secret, "OSS_ACCESS_KEY_SECRET"),
    }
}

/// 将转写文本追加到编辑器末尾（追加模式，§4.3.1）。
fn append_text(editor: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut Context<VoxInk>) {
    editor.update(cx, |state, cx| {
        let mut value = state.value().to_string();
        if !value.is_empty() {
            value.push('\n');
        }
        value.push_str(text);
        state.set_value(value, window, cx);
    });
}

/// 由前缀 + 记录 id 派生稳定且唯一的 ElementId（避免同列表内 id 冲突）。
fn elem_id(prefix: &str, id: &str) -> SharedString {
    SharedString::from(format!("{prefix}-{id}"))
}

/// 模式图标。
fn mode_icon(mode: &str) -> &'static str {
    match mode {
        "streaming" => "🎤",
        "offline" => "📄",
        _ => "•",
    }
}

/// RFC3339 UTC → 本地时间标签：今天显示 HH:MM，否则 MM-DD。
fn time_label(updated_at: &str) -> String {
    match DateTime::parse_from_rfc3339(updated_at) {
        Ok(dt) => {
            let local = dt.with_timezone(&Local);
            if local.date_naive() == Local::now().date_naive() {
                local.format("%H:%M").to_string()
            } else {
                local.format("%m-%d").to_string()
            }
        }
        Err(_) => updated_at.chars().take(10).collect(),
    }
}

/// 按记录创建时间计算时间分组（类聊天应用：今天/昨天/近 7 天/近 30 天/更早）。
fn record_bucket(created_at: &str) -> &'static str {
    let Ok(dt) = DateTime::parse_from_rfc3339(created_at) else {
        return "更早";
    };
    let date = dt.with_timezone(&Local).date_naive();
    let days = (Local::now().date_naive() - date).num_days();
    if days <= 0 {
        "group.today"
    } else if days == 1 {
        "group.yesterday"
    } else if days <= 7 {
        "group.last7"
    } else if days <= 30 {
        "group.last30"
    } else {
        "group.older"
    }
}

/// 把 `AsrError` 映射为用户友好的中文提示（任务 4.5）。
pub(crate) fn friendly_asr_error(error: &AsrError) -> String {
    match error {
        AsrError::AuthError => "API Key 无效或未配置，请在设置中检查".to_string(),
        AsrError::QuotaExceeded(_) => "API 配额已用尽".to_string(),
        AsrError::Timeout => "转写超时，请检查网络或缩短录音时长".to_string(),
        AsrError::EmptyResult => "未识别到语音内容".to_string(),
        AsrError::EmptyAudio => "录音数据为空".to_string(),
        AsrError::UnsupportedFormat(msg) => msg.clone(),
        AsrError::NetworkError(_) | AsrError::WebSocketError(_) => {
            "网络错误，请检查网络连接（录音文件已保留）".to_string()
        }
        other => format!("转写失败: {other}"),
    }
}
