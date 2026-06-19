//! 主界面 View —— 双栏布局（2026-06-14 重设计）：左侧常驻识别记录栏 + 右侧当前记录编辑区。
//!
//! - 左栏：「＋ 新建」+ 搜索 + 按时间分组（今天/昨天/近 7 天/近 30 天）的记录列表；当前项高亮、可删除。
//! - 右栏：录音按钮（对当前记录续录追加）+ 模式切换 + 状态 + 文本编辑区 + 字数/复制。
//! - 每条记录是一个可编辑、可续录的文档（§2.8 单表 records）；启动默认打开最近一条。
//! - 录制中禁用「新建」与切换记录；正文手动编辑防抖自动保存。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Local};
use gpui::{
    Animation, AnimationExt, AnyElement, App, ClickEvent, Context, Entity, Focusable, Hsla,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Styled, Subscription, Window,
    WindowControlArea, div, ease_in_out, prelude::*, px, white,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Root, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    v_flex,
};

use crate::theme::{BRAND, DANGER, STATUS_IDLE, STATUS_PROCESSING, STATUS_RECORDING, brand_tint};

use crate::asr::traits::StreamingResult;
use crate::asr::{AsrConfig, AsrError, BackendRegistry};
use crate::audio::{
    AudioError, LevelMeter, MicProbe, Recorder, StreamingCapture, list_input_devices, load_level,
};
use crate::config::VoxInkConfig;
use crate::history::GlobalHistory;
use crate::history::db::{Record, Segment};
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
/// Toast 通知宽度（px）——比 gpui-component 默认 448 窄，避免遮挡正文；内容过长自动换行。
const TOAST_WIDTH: f32 = 280.0;
/// Toast 自动消失时延（ms）。gpui-component 内置 5s 偏长，这里关掉它改用自管定时器缩短。
const TOAST_DURATION_MS: u64 = 2500;

/// 给每条 toast 一个唯一 id（配合自管定时器按 key 精确移除）的类型标记。
struct ToastKind;
/// 全局自增计数器：每条 toast 取唯一 key，避免同 id 互相替换（让多条 toast 可堆叠）。
static TOAST_SEQ: AtomicU32 = AtomicU32::new(0);

/// 推送一条窄版 toast 通知：
/// - 覆盖 gpui-component 默认宽度（[`TOAST_WIDTH`]），文本过长自动换行；
/// - 字体比默认（text_sm）更小（text_xs）——故用 `content` 自绘文本而非 `message`；
/// - 关掉库内置 5s 自动隐藏，改用更短的 [`TOAST_DURATION_MS`] 自管定时器移除。
pub fn notify(window: &mut Window, message: impl Into<SharedString>, cx: &mut App) {
    let text = message.into();
    let key = TOAST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as usize;
    window.push_notification(
        Notification::new()
            .id1::<ToastKind>(key)
            .autohide(false)
            .w(px(TOAST_WIDTH))
            .content(move |_, _, _| div().text_xs().child(text.clone()).into_any_element()),
        cx,
    );

    // 自管定时器：到点按 key 移除该条（库内置 autohide 时长不可配，故自行实现）。
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(TOAST_DURATION_MS))
            .await;
        let _ = cx.update_window(handle, |_, window, app| {
            window.remove_notification1::<ToastKind>(key, app);
        });
    })
    .detach();
}

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
    /// 流式会话累计的最终文本（用于落库为该段 segment 的 text）。
    streaming_text: String,
    /// 进行中/刚结束的录音元信息（用于完成时落库片段或删临时文件）。
    active_recording: Option<ActiveRecording>,
    /// 录音 worker 写入的实时电平（峰值幅度 0..1）。
    level_meter: LevelMeter,
    /// 近期电平历史（用于绘制滚动波形）。
    levels: Vec<f32>,
    /// 缓存的可用麦克风设备名（打开下拉时刷新；不每帧查询）。
    mic_devices: Vec<String>,
    /// 麦克风下拉是否展开。
    mic_dropdown_open: bool,
    /// 是否正在做麦克风可用性测试。
    mic_testing: bool,
    /// 测试期间的探测句柄（保持采集流存活；结束/drop 即停止）。
    mic_probe: Option<MicProbe>,
    /// 自动保存防抖代际计数（仅最新一次定时器生效）。
    autosave_gen: u64,
    /// 当前记录的录音片段列表（随记录切换/录制/删除刷新）。
    segments: Vec<Segment>,
    /// 是否展开右侧"录音片段"面板。
    show_segments: bool,
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

/// 进行中录音的元信息：完成时据此落库 segment（持久化）或删除临时文件（不持久化）。
struct ActiveRecording {
    /// 归属记录 id。
    record_id: String,
    /// WAV 文件路径。
    path: PathBuf,
    /// 是否持久化（保存到音频库）。false 表示临时文件，完成后删除。
    persisted: bool,
    /// 录制模式 "streaming" | "offline"。
    mode: String,
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

        // 当前记录的录音片段。
        let segments = cx
            .try_global::<GlobalHistory>()
            .and_then(|g| g.0.list_segments(&current_record_id).ok())
            .unwrap_or_default();

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
        let settings_sub = cx.subscribe(
            &settings,
            |this, _s, event: &SettingsEvent, cx| match event {
                SettingsEvent::Closed => {
                    this.show_settings = false;
                    cx.notify();
                }
            },
        );

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
            streaming_text: String::new(),
            active_recording: None,
            level_meter: Arc::new(AtomicU32::new(0)),
            levels: Vec::new(),
            mic_devices: list_input_devices(),
            mic_dropdown_open: false,
            mic_testing: false,
            mic_probe: None,
            segments,
            show_segments: false,
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
            if let Err(e) = global.0.save_record(&id, &text, rec.duration_secs) {
                tracing::error!("自动保存失败: {e:#}");
            }
        }
    }

    /// 录制完成后把编辑器正文写回当前记录，并累加本次时长。
    fn persist_after_recording(&mut self, added_secs: u32, cx: &mut Context<Self>) {
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
            if let Err(e) = db.save_record(&id, &text, base + added_secs) {
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
        self.new_record(window, cx);
    }

    /// 新建一条空记录并切到它（点击「新建记录」或应用内快捷键触发）。
    pub fn new_record(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_idle() {
            notify(window, "录制中，暂不能新建记录", cx);
            return;
        }
        self.flush_editor_to_record(cx);
        let new_rec = match cx.try_global::<GlobalHistory>() {
            Some(global) => match global.0.create_record() {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("新建记录失败: {e:#}");
                    notify(window, "新建记录失败", cx);
                    return;
                }
            },
            None => return,
        };
        self.current_record_id = new_rec.id;
        self.editor.update(cx, |s, cx| s.set_value("", window, cx));
        self.refresh_records(cx);
        self.refresh_segments(cx);
        let focus_handle = self.editor.focus_handle(cx);
        focus_handle.focus(window, cx);
    }

    /// 选择并载入一条记录到编辑器（录制中禁用切换）。
    fn select_record(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.current_record_id {
            return;
        }
        if !self.is_idle() {
            notify(window, "录制中，暂不能切换记录", cx);
            return;
        }
        self.flush_editor_to_record(cx);
        let rec = cx
            .try_global::<GlobalHistory>()
            .and_then(|g| g.0.get_record(&id).ok().flatten());
        if let Some(rec) = rec {
            self.current_record_id = rec.id;
            self.editor
                .update(cx, |s, cx| s.set_value(rec.text, window, cx));
        }
        self.refresh_records(cx);
        self.refresh_segments(cx);
    }

    /// 删除一条记录；若删的是当前记录，则切到最近一条（无则新建空记录）。
    fn delete_record(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_idle() {
            notify(window, "录制中，暂不能删除记录", cx);
            return;
        }
        let deleting_current = id == self.current_record_id;
        if let Some(global) = cx.try_global::<GlobalHistory>() {
            // 先删该记录的音频文件（DB 行由外键级联删除）。
            if let Ok(paths) = global.0.audio_paths_for_record(&id) {
                for p in &paths {
                    if let Err(e) = std::fs::remove_file(p) {
                        tracing::debug!("删除音频文件失败（可能已不存在）: {e}");
                    }
                }
            }
            if let Err(e) = global.0.delete_record(&id) {
                tracing::error!("删除记录失败: {e:#}");
                notify(window, "删除记录失败", cx);
                return;
            }
            // 尝试删除当前根下该记录的（应已空的）目录。
            if let Some(cfg) = cx.try_global::<GlobalConfig>()
                && let Ok(root) = cfg.0.storage.audio_root()
            {
                let _ = std::fs::remove_dir_all(root.join(&id));
            }
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
                    self.editor
                        .update(cx, |s, cx| s.set_value(rec.text, window, cx));
                }
                None => {
                    self.current_record_id = String::new();
                    self.editor.update(cx, |s, cx| s.set_value("", window, cx));
                }
            }
            self.refresh_segments(cx);
        }
        self.refresh_records(cx);
    }

    fn max_recording_seconds(&self, cx: &Context<Self>) -> u32 {
        cx.try_global::<GlobalConfig>()
            .map(|g| g.0.asr.max_recording_seconds)
            .unwrap_or(600)
    }

    // ───────────────────────────── 麦克风选择 / 测试（2026-06-19）─────────────────────────────

    /// 当前配置的首选麦克风名（空 = 系统默认 → None）。
    fn configured_input_device(&self, cx: &Context<Self>) -> Option<String> {
        cx.try_global::<GlobalConfig>()
            .map(|g| g.0.audio.input_device.clone())
            .filter(|s| !s.trim().is_empty())
    }

    /// 麦克风栏显示的当前设备名（空 → "系统默认"）。
    fn current_mic_label(&self, cx: &Context<Self>) -> String {
        match self.configured_input_device(cx) {
            Some(name) => name,
            None => tr("mic.default"),
        }
    }

    /// 展开/收起麦克风下拉（录制中禁止切换）；展开时刷新设备列表。
    fn on_toggle_mic_dropdown(&mut self, _: &ClickEvent, _w: &mut Window, cx: &mut Context<Self>) {
        if !self.is_idle() {
            return;
        }
        self.mic_dropdown_open = !self.mic_dropdown_open;
        if self.mic_dropdown_open {
            self.mic_devices = list_input_devices();
        }
        cx.notify();
    }

    /// 选择麦克风（空串 = 系统默认），写入配置（退出时落盘）。
    fn select_mic(&mut self, name: String, cx: &mut Context<Self>) {
        self.mic_dropdown_open = false;
        if cx.try_global::<GlobalConfig>().is_some() {
            let mut c = cx.global::<GlobalConfig>().0.clone();
            c.audio.input_device = name;
            cx.set_global(GlobalConfig(c));
        }
        cx.notify();
    }

    /// 测试当前麦克风可用性：打开探测流，实时显示电平约 1.7s，结束后据峰值给出结论。
    fn on_test_mic(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_idle() {
            notify(window, tr("mic.busy"), cx);
            return;
        }
        if self.mic_testing {
            return;
        }
        let device = self.configured_input_device(cx);
        let level: LevelMeter = Arc::new(AtomicU32::new(0));
        match MicProbe::start(device, level.clone()) {
            Ok(probe) => {
                self.mic_probe = Some(probe);
                self.mic_testing = true;
                self.mic_dropdown_open = false;
                self.level_meter = level;
                self.levels.clear();
                cx.notify();
                self.spawn_mic_test_poll(window, cx);
            }
            Err(e) => {
                tracing::error!("麦克风测试失败: {e}");
                let msg = match e {
                    AudioError::NoInputDevice => tr("mic.test_no_device"),
                    _ => tr("mic.test_failed"),
                };
                notify(window, msg, cx);
            }
        }
    }

    /// 测试期间 ~60ms 轮询电平、绘制实时波形；约 1.7s 后收尾评估。
    fn spawn_mic_test_poll(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let mut ticks = 0u32;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;
                ticks += 1;
                let alive = this
                    .update(cx, |this, cx| {
                        if !this.mic_testing {
                            return false;
                        }
                        this.push_level(load_level(&this.level_meter));
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive || ticks >= 28 {
                    break;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| this.finish_mic_test(window, cx));
        })
        .detach();
    }

    /// 收尾麦克风测试：停止探测，据峰值电平给出"正常/无信号"提示。
    fn finish_mic_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(probe) = self.mic_probe.take() {
            probe.stop();
        }
        if !self.mic_testing {
            return;
        }
        let peak = self.levels.iter().copied().fold(0.0_f32, f32::max);
        self.mic_testing = false;
        self.levels.clear();
        // RMS 阈值：安静环境本底噪声约 0.001~0.002，说话可达 0.02+，0.005 作"有无信号"分界。
        let msg = if peak < 0.005 {
            tr("mic.test_no_signal")
        } else {
            tr("mic.test_ok")
        };
        notify(window, msg, cx);
        cx.notify();
    }

    // ───────────────────────────── 音频文件管理（2026-06-16）─────────────────────────────

    /// 计算本次录音的 WAV 落点：持久化时为 `{音频根}/{record_id}/{时戳}_{短id}.wav` 并建目录；
    /// 否则（未开启保存或建目录失败）回退临时文件。返回 (路径, 是否持久化)。
    fn prepare_recording_path(&self, cx: &Context<Self>) -> (PathBuf, bool) {
        let cfg = cx
            .try_global::<GlobalConfig>()
            .map(|g| g.0.clone())
            .unwrap_or_default();
        if cfg.storage.save_audio
            && !self.current_record_id.is_empty()
            && let Ok(root) = cfg.storage.audio_root()
        {
            let dir = root.join(&self.current_record_id);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!("创建录音目录失败，回退临时文件: {e:#}");
            } else {
                let name = format!(
                    "{}_{}.wav",
                    Local::now().format("%Y%m%d-%H%M%S"),
                    short_id()
                );
                return (dir.join(name), true);
            }
        }
        (crate::audio::writer::temp_wav_path(), false)
    }

    /// 记录本次录音的元信息（开始时调用）。
    fn begin_active_recording(&mut self, path: PathBuf, persisted: bool, mode: &str) {
        self.active_recording = Some(ActiveRecording {
            record_id: self.current_record_id.clone(),
            path,
            persisted,
            mode: mode.to_string(),
        });
    }

    /// 完成一次录音：持久化则把音频片段落库（segments 表）；否则删除临时文件。
    /// `text` 为该段转写结果，`duration_secs` 为时长。
    fn finalize_segment(&mut self, text: &str, duration_secs: u32, cx: &mut Context<Self>) {
        let Some(active) = self.active_recording.take() else {
            return;
        };
        if !active.persisted {
            // 未持久化：清理临时 WAV，避免泄漏。
            if let Err(e) = std::fs::remove_file(&active.path) {
                tracing::debug!("删除临时录音失败（可能已不存在）: {e}");
            }
            return;
        }
        let size = std::fs::metadata(&active.path)
            .map(|m| m.len())
            .unwrap_or(0);
        let same_record = active.record_id == self.current_record_id;
        if let Some(g) = cx.try_global::<GlobalHistory>()
            && let Err(e) = g.0.add_segment(
                &active.record_id,
                &active.path,
                &active.mode,
                text,
                duration_secs,
                size,
            )
        {
            tracing::error!("录音片段落库失败: {e:#}");
        }
        // 若片段属于当前记录，刷新右侧片段列表。
        if same_record {
            self.refresh_segments(cx);
        }
    }

    /// 重新查询当前记录的录音片段并刷新 UI。
    fn refresh_segments(&mut self, cx: &mut Context<Self>) {
        self.segments = cx
            .try_global::<GlobalHistory>()
            .and_then(|g| g.0.list_segments(&self.current_record_id).ok())
            .unwrap_or_default();
        cx.notify();
    }

    // ───────────────────────────── 录音片段操作（回放/重转写/删除）─────────────────────────────

    /// 切换"录音片段"面板展开/收起。
    fn on_toggle_segments(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_segments = !self.show_segments;
        cx.notify();
    }

    /// 用系统默认播放器打开音频文件（回放）。
    fn play_segment(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let p = PathBuf::from(&path);
        if p.exists() {
            cx.open_with_system(&p);
        } else {
            notify(window, "音频文件不存在（可能已被清理）", cx);
        }
    }

    /// 删除单段：删文件 + 删行 + 刷新。
    fn delete_segment(&mut self, seg_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(g) = cx.try_global::<GlobalHistory>() {
            match g.0.delete_segment(&seg_id) {
                Ok(Some(path)) => {
                    let _ = std::fs::remove_file(&path);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!("删除片段失败: {e:#}");
                    notify(window, "删除片段失败", cx);
                    return;
                }
            }
        }
        self.refresh_segments(cx);
    }

    /// 重新转写某段音频：离线后端转写 → 追加到正文 + 回填该段文本与模式（圆点转灰）+ 刷新。
    fn retranscribe_segment(
        &mut self,
        seg_id: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let wav = PathBuf::from(&path);
        if !wav.exists() {
            notify(window, "音频文件不存在（可能已被清理）", cx);
            return;
        }
        let asr_config = runtime_asr_config(cx, false);
        let Some(handle) = cx.try_global::<GlobalTokioHandle>().map(|g| g.0.clone()) else {
            return;
        };
        notify(window, "重新转写中…", cx);

        cx.spawn_in(window, async move |this, cx| {
            let (tx, rx) = tokio::sync::oneshot::channel();
            handle.spawn(async move {
                let result = run_offline_transcription(asr_config, wav).await;
                let _ = tx.send(result);
            });
            let outcome = rx.await;
            let _ = this.update_in(cx, |this, window, cx| match outcome {
                Ok(Ok(text)) => {
                    append_text(&this.editor, &text, window, cx);
                    this.flush_editor_to_record(cx);
                    if let Some(g) = cx.try_global::<GlobalHistory>() {
                        // 重转写固定走离线，回填后该段标记为 offline（圆点转灰）。
                        let _ = g.0.update_segment_transcription(&seg_id, &text, "offline");
                    }
                    this.refresh_records(cx);
                    this.refresh_segments(cx);
                    notify(window, "重新转写完成", cx);
                }
                Ok(Err(e)) => {
                    tracing::error!("重新转写失败: {e}");
                    notify(window, friendly_asr_error(&e), cx);
                }
                Err(_) => notify(window, "重新转写已取消", cx),
            });
        })
        .detach();
    }

    /// 开始录音：构建 Recorder，进入 Recording 状态，启动计时器。
    fn start_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (wav_path, persisted) = self.prepare_recording_path(cx);
        let level: LevelMeter = Arc::new(AtomicU32::new(0));
        let device = self.configured_input_device(cx);
        match Recorder::start(wav_path.clone(), level.clone(), device) {
            Ok(recorder) => {
                self.begin_active_recording(wav_path, persisted, "offline");
                self.recorder = Some(recorder);
                self.level_meter = level;
                self.levels.clear();
                self.state.recording_state = RecordingState::Recording;
                self.state.recording_duration_secs = 0;
                tracing::info!("开始录音");
                cx.notify();
                let max = self.max_recording_seconds(cx);
                self.spawn_timer(window, cx, max);
                self.spawn_level_poll(window, cx);
            }
            Err(e) => {
                tracing::error!("启动录音失败: {e}");
                let msg = match e {
                    AudioError::NoInputDevice => "未检测到麦克风，请检查录音设备",
                    _ => "无法开始录音，请重试",
                };
                notify(window, msg, cx);
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
                    notify(
                        window,
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
                notify(window, "停止录音时出错", cx);
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
        notify(window, "正在识别…", cx);

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
                        this.persist_after_recording(duration_secs, cx);
                        // 录音片段落库（持久化）或清理临时文件。
                        this.finalize_segment(&text, duration_secs, cx);
                        notify(window, "转写完成", cx);
                    }
                    Ok(Err(e)) => {
                        tracing::error!("离线转写失败: {e}");
                        // 转写失败仍保留音频（可重转写）。
                        this.finalize_segment("", duration_secs, cx);
                        notify(window, friendly_asr_error(&e), cx);
                    }
                    Err(_) => {
                        this.finalize_segment("", duration_secs, cx);
                        notify(window, "转写任务已取消", cx);
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
            notify(
                window,
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

        let (wav_path, persisted) = self.prepare_recording_path(cx);
        let level: LevelMeter = Arc::new(AtomicU32::new(0));
        let device = self.configured_input_device(cx);
        let capture =
            match StreamingCapture::start(audio_tx, wav_path.clone(), level.clone(), device) {
            Ok(capture) => capture,
            Err(e) => {
                tracing::error!("启动流式采集失败: {e}");
                let msg = match e {
                    AudioError::NoInputDevice => "未检测到麦克风，请检查录音设备",
                    _ => "无法开始录音，请重试",
                };
                notify(window, msg, cx);
                return;
            }
        };
        self.begin_active_recording(wav_path, persisted, "streaming");
        self.streaming = Some(StreamingSession { capture });
        self.level_meter = level;
        self.levels.clear();
        self.streaming_fallback = false;
        self.streaming_duration_secs = 0;
        self.streaming_text.clear();
        self.state.recording_state = RecordingState::Recording;
        self.state.recording_duration_secs = 0;
        self.state.pending_text.clear();
        cx.notify();
        notify(window, "实时识别中…", cx);

        // 用配置选定的流式后端（用户在设置中选择）；只依赖 trait + 注册表。
        let streaming_backend_id = config.backend_id.clone();
        handle.spawn(async move {
            let registry = BackendRegistry::with_builtins();
            let result = match registry.get(&streaming_backend_id) {
                Some(backend) => {
                    backend
                        .transcribe_streaming(&config, audio_rx, result_tx)
                        .await
                }
                None => Err(AsrError::InvalidConfig(format!(
                    "未找到后端: {streaming_backend_id}"
                ))),
            };
            let _ = done_tx.send(result);
        });

        let max = self.max_recording_seconds(cx);
        self.spawn_timer(window, cx, max);
        self.spawn_level_poll(window, cx);

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
                    notify(window, "实时识别失败，正在离线转写…", cx);
                    let duration_secs = outcome.duration.as_secs() as u32;
                    self.start_transcription(window, cx, outcome.path, duration_secs);
                } else {
                    // 关闭音频通道触发后端 finish-task → 最终结果 → done（转 Idle）。
                    notify(window, "正在生成最终结果…", cx);
                }
            }
            Err(e) => {
                tracing::error!("停止流式采集失败: {e}");
                notify(window, "停止录音时出错", cx);
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
            if !self.streaming_text.is_empty() {
                self.streaming_text.push('\n');
            }
            self.streaming_text.push_str(&result.delta_text);
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
                self.persist_after_recording(added, cx);
                // 录音片段落库（持久化）或清理临时文件。
                let seg_text = std::mem::take(&mut self.streaming_text);
                self.finalize_segment(&seg_text, added, cx);
                notify(window, "识别完成", cx);
                self.finish_to_idle(cx);
            }
            Err(AsrError::AuthError) => {
                if let Some(session) = self.streaming.take() {
                    let _ = session.capture.stop();
                }
                // 鉴权失败：保留已录音频（可重转写）。
                let added = self.streaming_duration_secs;
                self.finalize_segment("", added, cx);
                notify(window, "API Key 无效，请检查后重试", cx);
                self.finish_to_idle(cx);
            }
            Err(e) => {
                tracing::warn!("实时识别失败: {e}");
                self.streaming_fallback = true;
                if self.streaming.is_some() {
                    // 用户仍在录音：保持录制，停止后用完整 WAV 离线转写（finalize 在转写完成后）。
                    notify(window, "实时识别失败，已切换离线，停止后将转写", cx);
                    cx.notify();
                } else {
                    // 已停止且无回退转写：保留音频片段。
                    let added = self.streaming_duration_secs;
                    self.finalize_segment("", added, cx);
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
                cx.background_executor().timer(Duration::from_secs(1)).await;
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

    /// 波形保留的电平样本数（约 = 面板可容纳的竖条数）。
    const LEVEL_HISTORY: usize = 56;

    /// 追加一个电平样本到历史（超出容量丢弃最旧）。
    fn push_level(&mut self, v: f32) {
        if self.levels.len() >= Self::LEVEL_HISTORY {
            self.levels.remove(0);
        }
        self.levels.push(v);
    }

    /// 录音期间按 ~60ms 轮询实时电平并刷新波形（录音结束自动退出）。
    fn spawn_level_poll(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;
                let stop = this
                    .update(cx, |this, cx| {
                        if this.state.recording_state != RecordingState::Recording {
                            return true;
                        }
                        let lvl = load_level(&this.level_meter);
                        this.push_level(lvl);
                        cx.notify();
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
    fn status(&self) -> (SharedString, Hsla) {
        match self.state.recording_state {
            RecordingState::Idle => (tr("status.idle").into(), STATUS_IDLE),
            RecordingState::Recording => (tr("status.recording").into(), STATUS_RECORDING),
            RecordingState::Processing => (tr("status.processing").into(), STATUS_PROCESSING),
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
            notify(window, "没有可复制的内容", cx);
            return;
        }
        match copy_to_clipboard(&text) {
            Ok(()) => crate::hotkey::simulate_paste(),
            Err(e) => {
                tracing::error!("复制失败: {e:#}");
                notify(window, "复制失败", cx);
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

    /// 在实时/离线之间切换转录模式（应用内快捷键触发）。
    fn toggle_mode(&mut self, cx: &mut Context<Self>) {
        let next = match self.state.transcription_mode {
            TranscriptionMode::Streaming => TranscriptionMode::Offline,
            TranscriptionMode::Offline => TranscriptionMode::Streaming,
        };
        self.on_select_mode(next, cx);
    }

    /// 主窗口聚焦时的按键分发：匹配「应用内快捷键」并执行对应动作。
    ///
    /// 设置面板打开时不处理（其有自己的改键捕获）；非「修饰键+主键」的普通输入直接放行
    /// （[`accelerator_from_keystroke`] 返回 None），故不影响编辑区正常打字。
    fn on_app_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_settings {
            return;
        }
        let Some(spec) = crate::hotkey::accelerator_from_keystroke(&ev.keystroke) else {
            return;
        };
        let Some(s) = cx.try_global::<GlobalConfig>().map(|g| g.0.shortcuts.clone()) else {
            return;
        };
        if spec == s.app_copy_all.trim() {
            self.copy_all(window, cx);
        } else if spec == s.app_new_record.trim() {
            self.new_record(window, cx);
        } else if spec == s.app_toggle_mode.trim() {
            self.toggle_mode(cx);
        }
    }

    fn on_copy(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.copy_all(window, cx);
    }

    /// 复制编辑区全部文本到剪贴板（点击「一键复制」或应用内快捷键触发）。
    pub fn copy_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        tracing::info!(chars = text.chars().count(), "复制全部文本");

        if text.is_empty() {
            notify(window, "没有可复制的内容", cx);
            return;
        }

        match copy_to_clipboard(&text) {
            Ok(()) => {
                self.copied = true;
                cx.notify();
                notify(window, "已复制到剪贴板", cx);

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
                notify(window, "复制失败", cx);
            }
        }
    }

    /// 在系统文件管理器中打开当前记录的录音目录（取最近一段已存在音频的所在目录）。
    fn on_open_recordings(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_record_id.is_empty() {
            notify(window, "该记录暂无录音文件", cx);
            return;
        }
        let paths = cx
            .try_global::<GlobalHistory>()
            .and_then(|g| g.0.audio_paths_for_record(&self.current_record_id).ok())
            .unwrap_or_default();
        // 取最近一段仍存在的音频文件，打开其所在目录。
        let dir = paths
            .iter()
            .rev()
            .find(|p| p.exists())
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        match dir {
            Some(dir) => cx.open_with_system(&dir),
            None => notify(window, "该记录暂无录音文件", cx),
        }
    }

    /// 打开设置面板覆盖层（M11）：用当前配置填充输入框后显示。
    fn on_open_settings(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(window, cx);
    }

    /// 打开设置面板覆盖层（供主界面齿轮与系统托盘菜单调用）。
    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |panel, pcx| panel.load_from_config(window, pcx));
        self.show_settings = true;
        cx.notify();
    }

    // ───────────────────────────── 渲染：左栏 ─────────────────────────────

    /// 自绘标题栏（无系统标题栏）：左=可拖拽的品牌+状态区，右=设置齿轮 + 自绘最小化/最大化/关闭。
    ///
    /// 拖拽区与可点击控件必须是**兄弟**而非父子：gpui 把标了 `WindowControlArea::Drag` 的元素整片
    /// 当成 HTCAPTION，其子元素的点击会被系统当成拖窗而吞掉。故齿轮/窗口按钮独立成兄弟节点。
    fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = !self.is_idle();
        let (status_text, status_color) = self.status();
        let max_icon = if window.is_maximized() {
            IconName::WindowRestore
        } else {
            IconName::WindowMaximize
        };

        h_flex()
            .relative()
            .h(px(34.))
            .w_full()
            .flex_shrink_0()
            .items_center()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            // 拖拽区：品牌（标记 Drag → HTCAPTION：拖动/双击最大化由系统处理）。
            .child(
                h_flex()
                    .flex_1()
                    .h_full()
                    .items_center()
                    .gap_2()
                    .pl_3()
                    .overflow_hidden()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        div()
                            .size(px(22.))
                            .rounded_full()
                            .bg(BRAND)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::empty()
                                    .path("icons/mic.svg")
                                    .size(px(13.))
                                    .text_color(white()),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("VoxInk"),
                    ),
            )
            // 设置齿轮（可点击：不在拖拽区内）。与窗口按钮同色同尺寸，保证标题栏配色统一。
            .child(
                div()
                    .id("settings")
                    .w(px(44.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(cx.theme().muted_foreground)
                    .hover(|s| s.bg(cx.theme().muted).text_color(cx.theme().foreground))
                    .on_click(cx.listener(Self::on_open_settings))
                    .child(Icon::new(IconName::Settings).size(px(15.))),
            )
            // 自绘窗口控制按钮（标记 NC 区域，点击由系统处理 最小化/最大化/关闭）。
            .child(self.render_window_button(
                "win-min",
                WindowControlArea::Min,
                IconName::WindowMinimize,
                false,
                cx,
            ))
            .child(self.render_window_button(
                "win-max",
                WindowControlArea::Max,
                max_icon,
                false,
                cx,
            ))
            .child(self.render_window_button(
                "win-close",
                WindowControlArea::Close,
                IconName::WindowClose,
                true,
                cx,
            ))
            // 状态胶囊：仅录音/处理时浮现，绝对居中于整条标题栏（更优雅；纯展示、不挡拖拽与按钮）。
            .when(active, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .px_2p5()
                                .py_0p5()
                                .rounded_full()
                                .bg(cx.theme().muted)
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(div().size(px(6.)).rounded_full().bg(status_color))
                                .child(status_text)
                                .child(div().font_family("Consolas").child(self.duration_label())),
                        ),
                )
            })
    }

    /// 单个窗口控制按钮：固定宽、整高、居中图标、悬停底色（关闭按钮悬停红）。
    /// 标 `WindowControlArea` 后，系统命中测试返回对应 HT 码，点击由系统处理（无需 on_click）。
    fn render_window_button(
        &self,
        id: &'static str,
        area: WindowControlArea,
        icon: IconName,
        is_close: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        // 悬停反馈：关闭键 → 红底白图标；最小化/最大化 → 浅底 + 深色图标（不可用白色，否则图标看不见）。
        let hover_fg = if is_close {
            white()
        } else {
            cx.theme().foreground
        };
        let hover_bg = if is_close { DANGER } else { cx.theme().muted };
        div()
            .id(id)
            .w(px(44.))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .window_control_area(area)
            .hover(|s| s.bg(hover_bg).text_color(hover_fg))
            .child(Icon::new(icon).size(px(14.)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(div().px_3().pt_3().pb_2().child(self.render_new_button(cx)))
            .child(div().px_3().pb_2().child(Input::new(&self.search)))
            .child(self.render_record_list(cx))
    }

    /// 「＋ 新建」按钮——主色浅填充 + 主色图标文字；录制中禁用（变灰、不可点）。
    fn render_new_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let is_idle = self.is_idle();
        let mut btn = h_flex()
            .id("new-record")
            .items_center()
            .justify_center()
            .gap_1p5()
            .w_full()
            .h(px(36.))
            .rounded(px(8.))
            .bg(brand_tint(cx))
            .text_color(BRAND)
            .text_sm()
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(Icon::new(IconName::Plus).size(px(15.)).text_color(BRAND))
            .child(tr("sidebar.new"));
        if is_idle {
            btn = btn
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .on_click(cx.listener(Self::on_new_record));
        } else {
            btn = btn.opacity(0.45);
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
                            .gap_1p5()
                            .items_center()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
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
            row = row.hover(|s| s.bg(cx.theme().list_hover));
        }

        if is_idle {
            row = row.child(
                div()
                    .id(elem_id("recdel", &rec.id))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(22.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_color(cx.theme().muted_foreground)
                    .hover(|s| s.text_color(DANGER))
                    .child(Icon::new(IconName::Close).size(px(13.)))
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
        let (bg, label, clickable, glyph): (Hsla, _, bool, RecordGlyph) =
            match self.state.recording_state {
                RecordingState::Idle => (BRAND, tr("record.start"), true, RecordGlyph::Dot),
                RecordingState::Recording => (
                    STATUS_RECORDING,
                    tr("record.stop"),
                    true,
                    RecordGlyph::Square,
                ),
                RecordingState::Processing => (
                    STATUS_PROCESSING,
                    tr("record.processing"),
                    false,
                    RecordGlyph::None,
                ),
            };

        let mut button = h_flex()
            .id("record-button")
            .items_center()
            .justify_center()
            .gap_1p5()
            .w(px(140.))
            .h(px(34.))
            .rounded(px(8.))
            .bg(bg)
            .text_color(white())
            .text_sm()
            .font_weight(gpui::FontWeight::MEDIUM)
            .shadow_sm()
            .when_some(glyph.element(), |this, g| this.child(g))
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
                    Animation::new(Duration::from_millis(1400))
                        .repeat()
                        .with_easing(ease_in_out),
                    |this, delta| {
                        // 三角波 0→1→0，营造脉冲呼吸感。
                        let t = 1.0 - (2.0 * delta - 1.0).abs();
                        this.opacity(0.78 + 0.22 * t)
                    },
                )
                .into_any_element()
        } else {
            button.into_any_element()
        }
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recording = self.state.recording_state == RecordingState::Recording;

        v_flex()
            .w_full()
            .gap_2()
            .px_4()
            .pt_4()
            .pb_3()
            // 单行工具条：模式切换 + 录音按钮 + 状态胶囊 + 实时波形。
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_3()
                    .child(self.render_mode_toggle(cx))
                    .child(self.render_record_button(cx))
                    .child(
                        // 右侧弹性区：录音时显示实时波形，空闲时留白。
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .when(recording, |d| d.child(self.render_waveform(cx))),
                    ),
            )
            // 麦克风栏：当前设备 + 下拉切换 + 测试（频繁操作，常驻主界面）。
            .child(self.render_mic_bar(cx))
            // 下拉展开时的设备列表（内联展开，避免浮层依赖）。
            .when(self.mic_dropdown_open, |this| {
                this.child(self.render_mic_dropdown(cx))
            })
            // 实时识别未稳定文本（pending）：浅色显示以区分稳定结果（§4.2.1）。
            .when(!self.state.pending_text.is_empty(), |this| {
                this.child(
                    div()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(brand_tint(cx))
                        .text_sm()
                        .italic()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.state.pending_text.clone()),
                )
            })
    }

    /// 转录模式切换（实时 / 离线）。录音中禁用但仍显示当前模式。
    fn render_mode_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_streaming = self.state.transcription_mode == TranscriptionMode::Streaming;
        let disabled = self.state.recording_state != RecordingState::Idle;
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .child(
                Button::new("mode-streaming")
                    .when(is_streaming, |b| b.primary())
                    .when(!is_streaming, |b| b.outline())
                    .disabled(disabled)
                    .label(tr("mode.streaming"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.on_select_mode(TranscriptionMode::Streaming, cx)
                    })),
            )
            .child(
                Button::new("mode-offline")
                    .when(!is_streaming, |b| b.primary())
                    .when(is_streaming, |b| b.outline())
                    .disabled(disabled)
                    .label(tr("mode.offline"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.on_select_mode(TranscriptionMode::Offline, cx)
                    })),
            )
    }

    /// 麦克风栏：当前设备（可点开下拉切换）+ 测试按钮 + 测试时的实时电平条。
    fn render_mic_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let idle = self.is_idle();
        let testing = self.mic_testing;
        let label = self.current_mic_label(cx);

        let mut trigger = h_flex()
            .id("mic-trigger")
            .items_center()
            .gap_1p5()
            .h(px(28.))
            .px_2p5()
            .rounded(px(6.))
            .border_1()
            .border_color(if self.mic_dropdown_open {
                BRAND
            } else {
                cx.theme().border
            })
            .bg(cx.theme().background)
            .text_xs()
            .child(
                Icon::empty()
                    .path("icons/mic.svg")
                    .size(px(12.))
                    .text_color(cx.theme().muted_foreground),
            )
            .child(div().max_w(px(180.)).truncate().child(label))
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(px(12.))
                    .text_color(cx.theme().muted_foreground),
            );
        if idle {
            trigger = trigger
                .cursor_pointer()
                .hover(|s| s.border_color(BRAND))
                .on_click(cx.listener(Self::on_toggle_mic_dropdown));
        } else {
            trigger = trigger.opacity(0.6);
        }

        // 测试时的实时电平条（与录音波形同风格，置于右侧弹性区）。
        let mut meter = h_flex().items_center().gap_0p5();
        if testing {
            for &l in &self.levels {
                meter = meter.child(
                    div()
                        .w(px(2.))
                        .h(px(level_bar_height(l)))
                        .rounded_full()
                        .bg(STATUS_RECORDING),
                );
            }
        }

        h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(trigger)
            .child(
                Button::new("mic-test")
                    .outline()
                    .small()
                    .label(if testing {
                        tr("mic.testing")
                    } else {
                        tr("mic.test")
                    })
                    .disabled(!idle || testing)
                    .on_click(cx.listener(Self::on_test_mic)),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(WAVE_MAX_PX))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(meter),
            )
    }

    /// 麦克风下拉：「系统默认」+ 各可用设备；当前项高亮。
    fn render_mic_dropdown(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.configured_input_device(cx).unwrap_or_default();
        let mut list = v_flex()
            .id("mic-list")
            .w_full()
            .max_h(px(180.))
            .overflow_y_scroll()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(6.))
            .bg(cx.theme().background)
            .child(self.mic_option("", tr("mic.default"), current.is_empty(), cx));

        if self.mic_devices.is_empty() {
            list = list.child(
                div()
                    .px_2p5()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("mic.no_devices")),
            );
        } else {
            for name in &self.mic_devices {
                let active = *name == current;
                list = list.child(self.mic_option(name, name.clone(), active, cx));
            }
        }
        list
    }

    /// 下拉中的单个设备项。
    fn mic_option(
        &self,
        value: &str,
        label: String,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let value = value.to_string();
        let id_suffix = if value.is_empty() { "default" } else { &value };
        let mut item = div()
            .id(SharedString::from(format!("mic-opt-{id_suffix}")))
            .w_full()
            .px_2p5()
            .py_1()
            .text_xs()
            .cursor_pointer()
            .hover(|s| s.bg(cx.theme().muted))
            .child(div().truncate().child(label))
            .on_click(cx.listener(move |this, _, _w, cx| this.select_mic(value.clone(), cx)));
        if active {
            item = item.bg(cx.theme().list_active);
        }
        item
    }

    /// 状态胶囊：● 状态 + MM:SS。
    /// 实时电平波形：把近期电平历史画成一排竖条（左旧右新，随声音起伏）。
    fn render_waveform(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = h_flex()
            .w_full()
            .h_full()
            .items_center()
            .justify_start()
            .gap_0p5();

        if self.levels.iter().all(|&l| l < 0.001) {
            // 尚无明显输入：提示"聆听中"。
            return row.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("record.listening")),
            );
        }

        for &l in &self.levels {
            row = row.child(
                div()
                    .w(px(2.))
                    .h(px(level_bar_height(l)))
                    .rounded_full()
                    .bg(STATUS_RECORDING),
            );
        }
        row
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex_1().w_full().px_4().pb_1().child(
            div()
                .size_full()
                .p_1()
                .bg(cx.theme().background)
                .border_1()
                .border_color(cx.theme().border)
                .rounded(px(10.))
                .child(Input::new(&self.editor).h_full().bordered(false)),
        )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let char_count = self.editor.read(cx).value().chars().count();
        let copied = self.copied;

        h_flex()
            .justify_between()
            .items_center()
            .w_full()
            .px_4()
            .py_3()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("toggle-segments")
                            .ghost()
                            .small()
                            .icon(IconName::GalleryVerticalEnd)
                            .tooltip(tr("segments.title"))
                            .when(self.show_segments, |b| b.primary())
                            .label(format!("{}", self.segments.len()))
                            .on_click(cx.listener(Self::on_toggle_segments)),
                    )
                    .child(
                        Button::new("open-recordings")
                            .ghost()
                            .small()
                            .icon(IconName::FolderOpen)
                            .tooltip(tr("footer.open_recordings"))
                            .on_click(cx.listener(Self::on_open_recordings)),
                    )
                    .child(format!("{} {char_count}", tr("footer.words"))),
            )
            .child(
                Button::new("copy")
                    .primary()
                    .when(copied, |b| b.success())
                    .icon(if copied {
                        IconName::Check
                    } else {
                        IconName::Copy
                    })
                    .label(if copied {
                        tr("footer.copied")
                    } else {
                        tr("footer.copy")
                    })
                    .on_click(cx.listener(Self::on_copy)),
            )
    }

    /// 右侧"录音片段"面板：当前记录的多段录音，支持回放/重转写/删除。
    fn render_segments(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = v_flex()
            .w_full()
            .max_h(px(200.))
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .px_4()
                    .py_2()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} ({})",
                        tr("segments.title"),
                        self.segments.len()
                    )),
            );

        if self.segments.is_empty() {
            return panel.child(
                div()
                    .px_4()
                    .pb_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("segments.empty")),
            );
        }

        let mut list = v_flex()
            .id("segment-list")
            .w_full()
            .gap_0p5()
            .px_2()
            .pb_2()
            .overflow_y_scroll();
        for seg in &self.segments {
            list = list.child(self.render_segment_row(seg, cx));
        }
        panel.child(list)
    }

    fn render_segment_row(&self, seg: &Segment, cx: &mut Context<Self>) -> impl IntoElement {
        let snippet = seg
            .text
            .lines()
            .next()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .unwrap_or_else(|| tr("segments.no_text"));
        let path = seg.file_path.clone();
        let id_re = seg.id.clone();
        let id_del = seg.id.clone();
        let path_play = path.clone();
        let path_re = path.clone();

        h_flex()
            .id(elem_id("seg", &seg.id))
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(px(6.))
            .hover(|s| s.bg(cx.theme().list_hover))
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(6.))
                    .rounded_full()
                    .bg(mode_dot(&seg.mode, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(div().w_full().text_sm().truncate().mb(px(2.)).child(snippet))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(time_label(&seg.created_at))
                            .child(fmt_duration(seg.duration_secs))
                            .child(fmt_size(seg.byte_size)),
                    ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_0p5()
                    .child(
                        Button::new(elem_id("seg-play", &seg.id))
                            .ghost()
                            .small()
                            .icon(IconName::Play)
                            .tooltip(tr("segments.play"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.play_segment(path_play.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(elem_id("seg-re", &seg.id))
                            .ghost()
                            .small()
                            .icon(IconName::Redo)
                            .tooltip(tr("segments.retranscribe"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.retranscribe_segment(id_re.clone(), path_re.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(elem_id("seg-del", &seg.id))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip(tr("segments.delete"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_segment(id_del.clone(), window, cx)
                            })),
                    ),
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
            // 应用内快捷键：聚焦的编辑/搜索框按键会冒泡到此处分发（见 on_app_key）。
            .on_key_down(cx.listener(Self::on_app_key))
            .child(
                v_flex()
                    .size_full()
                    // 自绘标题栏（常驻顶部，设置覆盖层之上仍可拖拽/最小化/关闭）。
                    .child(self.render_title_bar(window, cx))
                    .child(
                        // 内容区：左侧栏 + 右栏；设置覆盖层只盖此区域。
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(
                                h_flex().size_full().child(self.render_sidebar(cx)).child(
                                    // 右栏：录制控制 + 编辑区 + 底栏。
                                    // min_w_0：flex_1 默认 min-width:auto 会被长内容（编辑区/片段文字）
                                    // 撑破窗口，导致右侧按钮被 overflow_hidden 裁切；收敛后宽度随窗口自适应。
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        .child(self.render_controls(cx))
                                        .child(self.render_editor(cx))
                                        .when(self.show_segments, |this| {
                                            this.child(self.render_segments(cx))
                                        })
                                        .child(self.render_footer(cx)),
                                ),
                            )
                            // 设置面板覆盖层（M11）：盖住内容区，标题栏仍可用。
                            .when(self.show_settings, |this| this.child(self.settings.clone())),
                    ),
            )
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
    clipboard
        .set_text(text.to_owned())
        .context("写入剪贴板失败")?;
    Ok(())
}

/// 8 位短随机串（用于录音文件名去重）。
fn short_id() -> String {
    crate::history::db::short_uuid()
}

/// 启动时音频维护（main.rs 在打开窗口前调用）：
/// 1) 删除过期音频片段（`audio_retention_days`）及其文件；
/// 2) 清理当前音频根下"无对应 segment 记录"的孤儿文件（崩溃/中断残留）；
/// 3) 清理旧版临时目录里的 `voxink_recording_*.wav`。
pub(crate) fn cleanup_audio_on_startup(
    db: &crate::history::db::HistoryDb,
    storage: &crate::config::StorageConfig,
) {
    // 1) 过期片段。
    match db.purge_audio_older_than(storage.audio_retention_days) {
        Ok(paths) => {
            for p in &paths {
                let _ = std::fs::remove_file(p);
            }
            if !paths.is_empty() {
                tracing::info!("已清理 {} 个过期录音文件", paths.len());
            }
        }
        Err(e) => tracing::warn!("清理过期录音失败: {e:#}"),
    }

    // 2) 孤儿文件：当前音频根下、未被任何 segment 行引用的 .wav。
    if let Ok(root) = storage.audio_root()
        && root.is_dir()
    {
        let known: std::collections::HashSet<PathBuf> = db
            .all_segment_paths()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let mut removed = 0usize;
        // 遍历 {root}/{record_id}/*.wav。
        if let Ok(entries) = std::fs::read_dir(&root) {
            for dir in entries.flatten().filter(|e| e.path().is_dir()) {
                if let Ok(files) = std::fs::read_dir(dir.path()) {
                    for f in files.flatten() {
                        let p = f.path();
                        if p.extension().is_some_and(|e| e == "wav") && !known.contains(&p) {
                            let _ = std::fs::remove_file(&p);
                            removed += 1;
                        }
                    }
                }
                // 目录空了顺手删。
                let _ = std::fs::remove_dir(dir.path());
            }
        }
        if removed > 0 {
            tracing::info!("已清理 {removed} 个孤儿录音文件");
        }
    }

    // 3) 旧版临时 WAV。
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("voxink_recording_") && name.ends_with(".wav") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// 导出全部历史记录为 JSON，写入配置目录，返回文件路径（任务 10.4）。
/// 由设置面板「数据」区调用（2026-06-16 从主界面标题栏迁入）。
pub(crate) fn export_history_json(cx: &App) -> Result<PathBuf> {
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
fn append_text(
    editor: &Entity<InputState>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<VoxInk>,
) {
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

/// 录音按钮内的状态字形：录制点（实心圆）/ 停止块（圆角方块）/ 无。
enum RecordGlyph {
    Dot,
    Square,
    None,
}

impl RecordGlyph {
    fn element(&self) -> Option<AnyElement> {
        match self {
            // 麦克风图标，呼应「开始说话」。
            RecordGlyph::Dot => Some(
                Icon::empty()
                    .path("icons/mic.svg")
                    .size(px(15.))
                    .text_color(white())
                    .into_any_element(),
            ),
            // 经典「停止」圆角方块。
            RecordGlyph::Square => Some(
                div()
                    .size(px(10.))
                    .rounded(px(2.))
                    .bg(white())
                    .into_any_element(),
            ),
            RecordGlyph::None => None,
        }
    }
}

/// 波形竖条最大高度（px）。容器与 [`level_bar_height`] 的上限须一致以免裁切。
const WAVE_MAX_PX: f32 = 34.0;
/// 波形竖条静音时的最小高度（px）——保留一条细基线，视觉上"在听"。
const WAVE_MIN_PX: f32 = 2.0;

/// 电平（0..1 幅度包络）→ 波形竖条高度（px）。
///
/// **dBFS 对数刻度**（人耳对响度近似对数感知）映射到 [WAVE_MIN_PX, WAVE_MAX_PX]：
/// 先把 [MIN_DB, 0dB] 线性归一化，再做轻微 `^GAMMA`（GAMMA<1，**扩张**中低段，让常见说话音量
/// 也明显起伏）。配合采集端的峰值包络（[`audio::LevelEnvelope`]），对音量变化的感知更敏感。
/// 经验值：极轻(≈-35dB)≈0.21、轻声(≈-30dB)≈0.34、普通说话(≈-20dB)≈0.58、
/// 偏大(≈-10dB)≈0.79、很大(≈-6dB)≈0.88、削顶(0dB)=1.0。
/// 想更灵敏 → 调小 GAMMA 或抬高 MIN_DB；想更难满 → 调大 GAMMA。
fn level_bar_height(level: f32) -> f32 {
    const MIN_DB: f32 = -42.0;
    const GAMMA: f32 = 0.85;
    let norm = if level <= 1e-5 {
        0.0
    } else {
        let lin = ((20.0 * level.log10() - MIN_DB) / -MIN_DB).clamp(0.0, 1.0);
        lin.powf(GAMMA)
    };
    WAVE_MIN_PX + norm * (WAVE_MAX_PX - WAVE_MIN_PX)
}

/// 秒数格式化为 `MM:SS`。
fn fmt_duration(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// 人类可读字节数。
fn fmt_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

/// 模式指示点颜色：实时=主色，离线/其它=中性。
fn mode_dot(mode: &str, cx: &App) -> Hsla {
    match mode {
        "streaming" => BRAND,
        _ => cx.theme().muted_foreground,
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
