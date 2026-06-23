//! 设置面板（M11 任务 11.1/11.4，§6.4；2026-06-14 重构为按模式独立配置后端）。
//!
//! 作为全屏覆盖层渲染在主视图之上（不依赖 Sheet/Dialog 浮层，避免父子视图互租借）。
//! 自身只读写 `GlobalConfig` 与全局 locale/theme，关闭时发 [`SettingsEvent::Closed`] 由主视图收起。
//!
//! ASR 区：**实时**与**离线**各有独立下拉选择后端实现，并各自配置 api_key / endpoint；
//! 离线选「大文件」后端时额外显示 OSS 参数。下拉为自绘内联展开列表（避免浮层裁剪/复杂依赖）。

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, IntoElement, KeyDownEvent,
    ParentElement, Render, ScrollHandle, Styled, Window, div, prelude::*, px, rgba,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    scroll::{Scrollbar, ScrollbarShow},
    switch::Switch,
    v_flex,
};

use crate::app::{GlobalConfig, GlobalTokioHandle, friendly_asr_error, notify, runtime_asr_config};
use crate::asr::{AsrError, BackendRegistry};
use crate::config::{ShortcutsConfig, VoxInkConfig};
use crate::i18n::tr;
use crate::state::TranscriptionMode;
use crate::update;
use crate::theme::{BRAND, DANGER, MODE_OFFLINE, MODE_STREAMING};

const FILETRANS_ID: &str = "aliyun_bailian_filetrans";
/// 最长录音时长的可设上限：10 小时。防止用户误设过大值导致长时间录制忘记停止。
const MAX_RECORDING_SECONDS_LIMIT: u32 = 10 * 60 * 60;
/// 设置面板宽度（px）。固定居中覆盖层，与主窗口尺寸无关；上限取主窗口最小宽度（640）以免溢出。
/// 设置面板宽度上限（窗口足够宽时不再继续变宽）。
const PANEL_MAX_WIDTH: f32 = 820.0;
/// 设置面板宽度下限（窗口缩到最小 640 时仍留出左右边距）。
const PANEL_MIN_WIDTH: f32 = 560.0;
/// 设置面板高度上限（窗口足够高时不再继续变高）。
const PANEL_MAX_HEIGHT: f32 = 560.0;
/// 设置面板高度下限（保证标题栏 + 一段可滚动正文 + 保存按钮始终放得下）。
const PANEL_MIN_HEIGHT: f32 = 320.0;
/// 「标签在左、控件在右」字段行中右侧控件列的固定宽度（px）。
const FIELD_CONTROL_WIDTH: f32 = 300.0;

/// 设置面板事件：请求关闭。
pub enum SettingsEvent {
    Closed,
}

/// 当前展开的下拉。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dropdown {
    None,
    Streaming,
    Offline,
}

/// 「关于」区更新检查/下载的 UI 状态（M13，§11.3）。
enum UpdateStatus {
    /// 未检查。
    Idle,
    /// 正在向 GitHub 查询最新版本。
    Checking,
    /// 已是最新版本（或用户已跳过该版本）。
    UpToDate,
    /// 发现可用新版本。
    Available(crate::update::LatestRelease),
    /// 正在下载并安装。
    Downloading,
    /// 检查或下载失败。
    Failed(String),
}

/// 可改键的快捷键槽位（全局 + 应用内）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutSlot {
    // 全局热键
    Recording,
    Window,
    Paste,
    MiniBar,
    // 应用内快捷键
    CopyAll,
    NewRecord,
    ToggleMode,
}

impl ShortcutSlot {
    /// 全局热键槽位（OS 级注册）。
    const GLOBAL: [ShortcutSlot; 4] = [
        ShortcutSlot::Recording,
        ShortcutSlot::Window,
        ShortcutSlot::Paste,
        ShortcutSlot::MiniBar,
    ];
    /// 应用内快捷键槽位（仅主窗口聚焦时生效）。
    const IN_APP: [ShortcutSlot; 3] = [
        ShortcutSlot::CopyAll,
        ShortcutSlot::NewRecord,
        ShortcutSlot::ToggleMode,
    ];

    /// 行标题 locale key。
    fn label_key(self) -> &'static str {
        match self {
            ShortcutSlot::Recording => "shortcut.toggle_recording",
            ShortcutSlot::Window => "shortcut.toggle_window",
            ShortcutSlot::Paste => "shortcut.copy_paste",
            ShortcutSlot::MiniBar => "shortcut.toggle_mini_bar",
            ShortcutSlot::CopyAll => "shortcut.copy_all",
            ShortcutSlot::NewRecord => "shortcut.new_record",
            ShortcutSlot::ToggleMode => "shortcut.toggle_mode",
        }
    }

    /// 当前绑定字符串。
    fn get(self, s: &ShortcutsConfig) -> &str {
        match self {
            ShortcutSlot::Recording => &s.toggle_recording,
            ShortcutSlot::Window => &s.toggle_window,
            ShortcutSlot::Paste => &s.copy_and_paste,
            ShortcutSlot::MiniBar => &s.toggle_mini_bar,
            ShortcutSlot::CopyAll => &s.app_copy_all,
            ShortcutSlot::NewRecord => &s.app_new_record,
            ShortcutSlot::ToggleMode => &s.app_toggle_mode,
        }
    }

    fn set(self, s: &mut ShortcutsConfig, v: String) {
        match self {
            ShortcutSlot::Recording => s.toggle_recording = v,
            ShortcutSlot::Window => s.toggle_window = v,
            ShortcutSlot::Paste => s.copy_and_paste = v,
            ShortcutSlot::MiniBar => s.toggle_mini_bar = v,
            ShortcutSlot::CopyAll => s.app_copy_all = v,
            ShortcutSlot::NewRecord => s.app_new_record = v,
            ShortcutSlot::ToggleMode => s.app_toggle_mode = v,
        }
    }

    /// 该槽位若为全局热键，返回对应动作（用于查询注册失败状态）。
    fn global_action(self) -> Option<crate::hotkey::HotkeyAction> {
        use crate::hotkey::HotkeyAction;
        match self {
            ShortcutSlot::Recording => Some(HotkeyAction::ToggleRecording),
            ShortcutSlot::Window => Some(HotkeyAction::ToggleWindow),
            ShortcutSlot::Paste => Some(HotkeyAction::CopyAndPaste),
            ShortcutSlot::MiniBar => Some(HotkeyAction::ToggleMiniBar),
            _ => None,
        }
    }
}

/// 设置分类标签（左侧栏）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Asr,
    Recording,
    Polish,
    General,
    Shortcuts,
    Data,
    About,
}

impl SettingsTab {
    /// 左侧栏顺序与对应标题 locale key。
    const ALL: [(SettingsTab, &'static str); 7] = [
        (SettingsTab::Asr, "settings.section.asr"),
        (SettingsTab::Recording, "settings.section.recording"),
        (SettingsTab::Polish, "settings.section.polish"),
        (SettingsTab::General, "settings.section.general"),
        (SettingsTab::Shortcuts, "settings.section.shortcuts"),
        (SettingsTab::Data, "settings.section.data"),
        (SettingsTab::About, "settings.section.about"),
    ];
}

pub struct SettingsView {
    max_secs: Entity<InputState>,
    // 实时后端配置
    stream_api_key: Entity<InputState>,
    stream_endpoint: Entity<InputState>,
    // 离线后端配置
    off_api_key: Entity<InputState>,
    off_endpoint: Entity<InputState>,
    off_oss_endpoint: Entity<InputState>,
    off_oss_bucket: Entity<InputState>,
    off_oss_ak_id: Entity<InputState>,
    off_oss_ak_secret: Entity<InputState>,
    open_dropdown: Dropdown,
    // 数据/存储
    audio_dir: Entity<InputState>,
    audio_retention: Entity<InputState>,
    /// 文本记录保留天数（text.history_retention_days）。
    text_retention: Entity<InputState>,
    /// 音频根目录当前占用字节数（打开设置/清理后刷新）。
    audio_usage_bytes: u64,
    // AI 润色
    polish_base_url: Entity<InputState>,
    polish_model: Entity<InputState>,
    polish_api_key: Entity<InputState>,
    /// 当前编辑/选用的润色模板提示词（多行）。
    polish_prompt: Entity<InputState>,
    /// 当前编辑模板的名称（仅自定义模板可改）。
    polish_name: Entity<InputState>,
    /// 当前编辑/选用的润色模板 id。
    polish_edit_template: String,
    /// 当前选中的分类标签。
    active_tab: SettingsTab,
    /// 内容区滚动句柄（驱动可见滚动条）。
    scroll: ScrollHandle,
    /// 改键捕获用焦点句柄（聚焦后由 on_key_down 接收按键）。
    capture_focus: FocusHandle,
    /// 正在捕获按键的槽位（Some 表示改键进行中，此时全局热键已被 suspend）。
    capturing: Option<ShortcutSlot>,
    /// 「关于」区更新检查状态（M13）。
    update_status: UpdateStatus,
    /// 下载进度（0–100），由 tokio 下载任务写入、前台轮询展示。
    update_progress: Arc<AtomicU8>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx));
        Self {
            max_secs: input(window, cx),
            stream_api_key: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("settings.api_key_ph"))),
            stream_endpoint: input(window, cx),
            off_api_key: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("settings.api_key_ph"))),
            off_endpoint: input(window, cx),
            off_oss_endpoint: input(window, cx),
            off_oss_bucket: input(window, cx),
            off_oss_ak_id: input(window, cx),
            off_oss_ak_secret: input(window, cx),
            open_dropdown: Dropdown::None,
            audio_dir: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("settings.audio_dir_ph"))),
            audio_retention: input(window, cx),
            text_retention: input(window, cx),
            audio_usage_bytes: 0,
            polish_base_url: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr("polish.base_url_ph"))
                    .multi_line(true)
                    .auto_grow(2, 3)
            }),
            polish_model: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("polish.model_ph"))),
            polish_api_key: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("settings.api_key_ph"))),
            polish_prompt: cx.new(|cx| InputState::new(window, cx).multi_line(true).auto_grow(3, 8)),
            polish_name: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("polish.name_ph"))),
            polish_edit_template: String::new(),
            active_tab: SettingsTab::Asr,
            scroll: ScrollHandle::new(),
            capture_focus: cx.focus_handle(),
            capturing: None,
            update_status: UpdateStatus::Idle,
            update_progress: Arc::new(AtomicU8::new(0)),
        }
    }

    /// 打开面板时用当前配置填充所有输入框。
    pub fn load_from_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = cx.try_global::<GlobalConfig>().map(|g| g.0.clone()) else {
            return;
        };
        self.max_secs.update(cx, |s, cx| {
            s.set_value(c.asr.max_recording_seconds.to_string(), window, cx)
        });
        self.audio_dir.update(cx, |s, cx| {
            s.set_value(c.storage.audio_dir.clone(), window, cx)
        });
        self.audio_retention.update(cx, |s, cx| {
            s.set_value(c.storage.audio_retention_days.to_string(), window, cx)
        });
        self.text_retention.update(cx, |s, cx| {
            s.set_value(c.text.history_retention_days.to_string(), window, cx)
        });
        self.audio_usage_bytes = c
            .storage
            .audio_root()
            .map(|root| dir_size(&root))
            .unwrap_or(0);
        self.load_stream_inputs(&c, window, cx);
        self.load_offline_inputs(&c, window, cx);
        self.load_polish_inputs(&c, window, cx);
    }

    fn load_polish_inputs(
        &mut self,
        c: &VoxInkConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.polish_base_url
            .update(cx, |s, cx| s.set_value(c.polish.base_url.clone(), window, cx));
        self.polish_model
            .update(cx, |s, cx| s.set_value(c.polish.model.clone(), window, cx));
        self.polish_api_key
            .update(cx, |s, cx| s.set_value(c.polish.api_key.clone(), window, cx));
        // 选用当前模板并载入其提示词。
        let tpl = c
            .polish
            .active()
            .cloned()
            .unwrap_or_default();
        self.polish_edit_template = tpl.id.clone();
        self.polish_prompt
            .update(cx, |s, cx| s.set_value(tpl.prompt.clone(), window, cx));
        self.polish_name
            .update(cx, |s, cx| s.set_value(tpl.name.clone(), window, cx));
    }

    fn load_stream_inputs(
        &mut self,
        c: &VoxInkConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let b = c.asr.backend(&c.asr.streaming_backend);
        self.stream_api_key
            .update(cx, |s, cx| s.set_value(b.api_key.clone(), window, cx));
        self.stream_endpoint
            .update(cx, |s, cx| s.set_value(b.endpoint.clone(), window, cx));
    }

    fn load_offline_inputs(
        &mut self,
        c: &VoxInkConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let b = c.asr.backend(&c.asr.offline_backend);
        self.off_api_key
            .update(cx, |s, cx| s.set_value(b.api_key.clone(), window, cx));
        self.off_endpoint
            .update(cx, |s, cx| s.set_value(b.endpoint.clone(), window, cx));
        self.off_oss_endpoint
            .update(cx, |s, cx| s.set_value(b.oss_endpoint.clone(), window, cx));
        self.off_oss_bucket
            .update(cx, |s, cx| s.set_value(b.oss_bucket.clone(), window, cx));
        self.off_oss_ak_id.update(cx, |s, cx| {
            s.set_value(b.oss_access_key_id.clone(), window, cx)
        });
        self.off_oss_ak_secret.update(cx, |s, cx| {
            s.set_value(b.oss_access_key_secret.clone(), window, cx)
        });
    }

    /// 读改写一份 GlobalConfig（退出时统一落盘；部分项即时生效）。
    fn update_config(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut VoxInkConfig)) {
        if cx.try_global::<GlobalConfig>().is_some() {
            let mut c = cx.global::<GlobalConfig>().0.clone();
            f(&mut c);
            cx.set_global(GlobalConfig(c));
        }
    }

    /// 把输入框的值并入 GlobalConfig（写到各自选中的后端；不落盘）。
    fn flush_inputs_to_config(&mut self, cx: &mut Context<Self>) {
        let s_key = self.stream_api_key.read(cx).value().to_string();
        let s_ep = self.stream_endpoint.read(cx).value().to_string();
        let o_key = self.off_api_key.read(cx).value().to_string();
        let o_ep = self.off_endpoint.read(cx).value().to_string();
        let oss_e = self.off_oss_endpoint.read(cx).value().to_string();
        let oss_b = self.off_oss_bucket.read(cx).value().to_string();
        let oss_id = self.off_oss_ak_id.read(cx).value().to_string();
        let oss_secret = self.off_oss_ak_secret.read(cx).value().to_string();
        let max = self.max_secs.read(cx).value().to_string();
        let audio_dir = self.audio_dir.read(cx).value().to_string();
        let audio_retention = self.audio_retention.read(cx).value().to_string();
        let text_retention = self.text_retention.read(cx).value().to_string();
        let p_base = self.polish_base_url.read(cx).value().to_string();
        let p_model = self.polish_model.read(cx).value().to_string();
        let p_key = self.polish_api_key.read(cx).value().to_string();
        let p_prompt = self.polish_prompt.read(cx).value().to_string();
        let p_name = self.polish_name.read(cx).value().to_string();
        let p_tpl = self.polish_edit_template.clone();

        self.update_config(cx, |c| {
            c.polish.base_url = p_base.trim().to_string();
            c.polish.model = p_model.trim().to_string();
            c.polish.api_key = p_key.trim().to_string();
            if !p_tpl.is_empty() {
                c.polish.active_template = p_tpl.clone();
                if let Some(t) = c.polish.templates.iter_mut().find(|t| t.id == p_tpl) {
                    t.prompt = p_prompt.clone();
                    // 仅自定义模板允许改名。
                    if !crate::config::is_builtin_template(&p_tpl) && !p_name.trim().is_empty() {
                        t.name = p_name.trim().to_string();
                    }
                }
            }
        });
        self.update_config(cx, |c| {
            c.storage.audio_dir = audio_dir.trim().to_string();
            if let Ok(n) = audio_retention.trim().parse::<u32>() {
                c.storage.audio_retention_days = n;
            }
            if let Ok(n) = text_retention.trim().parse::<u32>() {
                c.text.history_retention_days = n;
            }
            let sid = c.asr.streaming_backend.clone();
            let s = c.asr.backends.entry(sid).or_default();
            s.api_key = s_key.trim().to_string();
            s.endpoint = s_ep.trim().to_string();

            // 离线与实时选同一后端时，二者共用同一份配置（离线输入未渲染）；
            // 跳过离线写入，否则离线区的旧值会覆盖实时区刚填的值。
            if c.asr.offline_backend != c.asr.streaming_backend {
                let oid = c.asr.offline_backend.clone();
                let is_filetrans = oid == FILETRANS_ID;
                let o = c.asr.backends.entry(oid).or_default();
                o.api_key = o_key.trim().to_string();
                o.endpoint = o_ep.trim().to_string();
                if is_filetrans {
                    o.oss_endpoint = oss_e.trim().to_string();
                    o.oss_bucket = oss_b.trim().to_string();
                    o.oss_access_key_id = oss_id.trim().to_string();
                    o.oss_access_key_secret = oss_secret.trim().to_string();
                }
            }

            if let Ok(n) = max.trim().parse::<u32>()
                && n > 0
            {
                c.asr.max_recording_seconds = n.min(MAX_RECORDING_SECONDS_LIMIT);
            }
        });
    }

    fn on_select_backend(
        &mut self,
        kind: Dropdown,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 切换前先保存当前输入到旧后端，避免丢失未保存编辑。
        self.flush_inputs_to_config(cx);
        match kind {
            Dropdown::Streaming => self.update_config(cx, |c| c.asr.streaming_backend = id.clone()),
            Dropdown::Offline => self.update_config(cx, |c| c.asr.offline_backend = id.clone()),
            Dropdown::None => {}
        }
        if let Some(c) = cx.try_global::<GlobalConfig>().map(|g| g.0.clone()) {
            match kind {
                Dropdown::Streaming => self.load_stream_inputs(&c, window, cx),
                Dropdown::Offline => self.load_offline_inputs(&c, window, cx),
                Dropdown::None => {}
            }
        }
        self.open_dropdown = Dropdown::None;
        cx.notify();
    }

    fn on_test(&mut self, want_streaming: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_inputs_to_config(cx);
        let config = runtime_asr_config(cx, want_streaming);
        let backend_id = config.backend_id.clone();
        let Some(handle) = cx.try_global::<GlobalTokioHandle>().map(|g| g.0.clone()) else {
            return;
        };
        notify(
            window,
            format!("{}（{backend_id}）…", tr("settings.test")),
            cx,
        );

        cx.spawn_in(window, async move |_this, cx| {
            let (tx, rx) = tokio::sync::oneshot::channel();
            handle.spawn(async move {
                let registry = BackendRegistry::with_builtins();
                let result = match registry.get(&backend_id) {
                    Some(backend) => backend.validate_config(&config).await,
                    None => Err(AsrError::InvalidConfig("未找到后端".to_string())),
                };
                let _ = tx.send(result);
            });
            let outcome = rx.await;
            let _ = cx.update(|window, cx| match outcome {
                Ok(Ok(())) => notify(window, "连接测试成功 ✓", cx),
                Ok(Err(e)) => notify(
                    window,
                    format!("连接测试失败：{}", friendly_asr_error(&e)),
                    cx,
                ),
                Err(_) => notify(window, "连接测试中断", cx),
            });
        })
        .detach();
    }

    /// 当前生效的音频根目录（配置非空则用之，否则默认）。
    fn current_audio_root(&self, cx: &Context<Self>) -> Option<std::path::PathBuf> {
        cx.try_global::<GlobalConfig>()
            .and_then(|g| g.0.storage.audio_root().ok())
    }

    /// 「浏览…」：弹原生目录选择器，选中后写入输入框与配置。
    fn on_browse_audio_dir(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(dir) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    let s = dir.to_string_lossy().to_string();
                    this.audio_dir
                        .update(cx, |st, cx| st.set_value(s.clone(), window, cx));
                    this.update_config(cx, |c| c.storage.audio_dir = s);
                    this.audio_usage_bytes = this
                        .current_audio_root(cx)
                        .map(|r| dir_size(&r))
                        .unwrap_or(0);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 「打开」：在系统文件管理器中打开音频根目录（不存在则先创建）。
    fn on_open_audio_dir(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        // 以输入框当前值为准（可能尚未落盘）。
        let dir = {
            let v = self.audio_dir.read(cx).value().to_string();
            if v.trim().is_empty() {
                crate::config::StorageConfig::default_audio_root().ok()
            } else {
                Some(std::path::PathBuf::from(v.trim()))
            }
        };
        let Some(dir) = dir else { return };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("创建音频目录失败: {e:#}");
            notify(window, "无法打开目录", cx);
            return;
        }
        cx.open_with_system(&dir);
    }

    /// 「立即清理过期音频」：按 audio_retention_days 删除过期片段及文件，刷新占用。
    fn on_clean_audio(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_inputs_to_config(cx);
        let days = cx
            .try_global::<GlobalConfig>()
            .map(|g| g.0.storage.audio_retention_days)
            .unwrap_or(0);
        let removed = match cx.try_global::<crate::history::GlobalHistory>() {
            Some(g) => match g.0.purge_audio_older_than(days) {
                Ok(paths) => {
                    for p in &paths {
                        let _ = std::fs::remove_file(p);
                    }
                    paths.len()
                }
                Err(e) => {
                    tracing::warn!("清理过期音频失败: {e:#}");
                    0
                }
            },
            None => 0,
        };
        self.audio_usage_bytes = self
            .current_audio_root(cx)
            .map(|r| dir_size(&r))
            .unwrap_or(0);
        notify(window, format!("已清理 {removed} 个过期录音"), cx);
        cx.notify();
    }

    /// 导出全部历史记录为 JSON（2026-06-16 从主界面标题栏迁入「数据」区）。
    fn on_export_history(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        match crate::app::export_history_json(cx) {
            Ok(path) => {
                tracing::info!("历史已导出: {}", path.display());
                notify(window, format!("已导出到 {}", path.display()), cx);
            }
            Err(e) => {
                tracing::error!("导出历史失败: {e:#}");
                notify(window, "导出失败", cx);
            }
        }
    }

    fn on_export_diag(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let config = cx.try_global::<GlobalConfig>().map(|g| g.0.clone());
        let Some(config) = config else { return };
        match crate::diagnostics::export(&config) {
            Ok(path) => {
                tracing::info!("诊断已导出: {}", path.display());
                notify(window, format!("已导出到 {}", path.display()), cx);
            }
            Err(e) => {
                tracing::error!("导出诊断失败: {e:#}");
                notify(window, "导出诊断失败", cx);
            }
        }
    }

    /// 在系统文件管理器中打开日志文件夹（`%LOCALAPPDATA%\VoxInk\logs`）。
    /// 目录尚不存在时（极早期失败）先创建，避免打开一个不存在的路径。
    fn on_open_logs(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        match crate::config::VoxInkConfig::log_dir() {
            Ok(dir) => {
                let _ = std::fs::create_dir_all(&dir);
                cx.open_with_system(&dir);
            }
            Err(e) => {
                tracing::error!("无法定位日志目录: {e:#}");
                notify(window, "无法定位日志目录", cx);
            }
        }
    }

    /// 「项目主页」：在系统默认浏览器打开 GitHub 仓库。
    fn on_open_github(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&update::repo_url());
    }

    /// 「手动下载」：打开 Releases 列表页（在线检查更新受网络限制时的兜底）。
    fn on_open_releases(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&update::releases_url());
    }

    /// 「检查更新」：向 GitHub 查询最新版本并更新「关于」区状态（M13，§11.3）。
    fn on_check_update(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.update_status,
            UpdateStatus::Checking | UpdateStatus::Downloading
        ) {
            return;
        }
        let Some(handle) = cx.try_global::<GlobalTokioHandle>().map(|g| g.0.clone()) else {
            return;
        };
        self.update_status = UpdateStatus::Checking;
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let (tx, rx) = tokio::sync::oneshot::channel();
            handle.spawn(async move {
                let _ = tx.send(update::check_latest().await);
            });
            let outcome = rx.await;
            let now = chrono::Utc::now().timestamp();
            let _ = this.update_in(cx, |this, _w, cx| {
                // 写回 last_check，与启动检查共用节流窗口。
                this.update_config(cx, |c| c.update.last_check = now);
                this.update_status = match outcome {
                    // 手动检查即使等于「已跳过」版本也照常展示。
                    Ok(Ok(latest)) if latest.is_newer => UpdateStatus::Available(latest),
                    Ok(Ok(_)) => UpdateStatus::UpToDate,
                    Ok(Err(e)) => UpdateStatus::Failed(format!("{e:#}")),
                    Err(_) => UpdateStatus::Failed("检查已取消".to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// 「立即更新」：下载并自替换，成功后启动新版并退出当前进程（M13，§11.3）。
    fn on_do_update(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (exe_url, sha256_url) = match &self.update_status {
            UpdateStatus::Available(rel) => (rel.exe_url.clone(), rel.sha256_url.clone()),
            _ => return,
        };
        let Some(handle) = cx.try_global::<GlobalTokioHandle>().map(|g| g.0.clone()) else {
            return;
        };
        self.update_progress.store(0, Ordering::Relaxed);
        self.update_status = UpdateStatus::Downloading;
        cx.notify();
        let progress = self.update_progress.clone();

        cx.spawn_in(window, async move |this, cx| {
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            handle.spawn(async move {
                let _ = tx.send(update::download_and_apply(exe_url, sha256_url, progress).await);
            });
            // 轮询：定时刷新进度条；下载任务完成后处理结果。
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        // 替换成功：启动新版进程，退出当前（on_app_quit 会保存配置）。
                        match update::spawn_restart() {
                            Ok(()) => {
                                let _ = cx.update(|_w, app| app.quit());
                            }
                            Err(e) => {
                                let msg = format!("{e:#}");
                                let _ = this.update(cx, |this, cx| {
                                    this.update_status = UpdateStatus::Failed(msg);
                                    cx.notify();
                                });
                            }
                        }
                        break;
                    }
                    Ok(Err(e)) => {
                        let msg = format!("{e:#}");
                        let _ = this.update(cx, |this, cx| {
                            this.update_status = UpdateStatus::Failed(msg);
                            cx.notify();
                        });
                        break;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                        // 仍在下载：刷新进度显示。
                        let _ = this.update(cx, |_, cx| cx.notify());
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        let _ = this.update(cx, |this, cx| {
                            this.update_status = UpdateStatus::Failed("更新任务异常中断".to_string());
                            cx.notify();
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }

    /// 「跳过此版本」：记录到配置，不再于启动时提示该版本（M13）。
    fn on_skip_version(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let UpdateStatus::Available(rel) = &self.update_status {
            let v = rel.version.clone();
            self.update_config(cx, |c| c.update.skipped_version = v.clone());
            notify(window, format!("已跳过版本 v{v}"), cx);
            self.update_status = UpdateStatus::UpToDate;
            cx.notify();
        }
    }

    /// 「打开发布页」：在浏览器打开 GitHub Releases 页面（自更新不可用时的手动入口）。
    fn on_open_release_page(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&update::release_page_url());
    }

    /// 保存当前页配置：把输入框并入内存配置并落盘（停留在面板，给出反馈）。
    fn on_save(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_inputs_to_config(cx);
        if let Some(c) = cx.try_global::<GlobalConfig>().map(|g| g.0.clone())
            && let Err(e) = c.save()
        {
            tracing::error!("保存配置失败: {e:#}");
        }
        notify(window, tr("settings.saved"), cx);
    }

    /// 关闭设置面板（右上角 X）。即时生效项（开关/主题等）已落入内存配置，
    /// 退出应用时统一持久化；未点「保存」的输入框文本不写盘。
    fn on_close(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // 若正在改键，关闭前先恢复全局热键（否则它们会一直处于注销状态）。
        self.cancel_capture(cx);
        cx.emit(SettingsEvent::Closed);
    }

    /// 当前标签底部是否显示「保存」按钮（关于=只读；快捷键=改键即时生效，无需保存）。
    fn tab_is_editable(&self) -> bool {
        matches!(
            self.active_tab,
            SettingsTab::Asr
                | SettingsTab::Recording
                | SettingsTab::Polish
                | SettingsTab::General
                | SettingsTab::Data
        )
    }

    // ───────────────────────────── 快捷键改键（捕获按键）─────────────────────────────

    /// 开始捕获某槽位的新按键：先 suspend 全局热键（否则按键被 OS 截获、传不到窗口），
    /// 再聚焦捕获句柄，由 [`Self::on_capture_key`] 接收下一个按键组合。
    fn begin_capture(&mut self, slot: ShortcutSlot, window: &mut Window, cx: &mut Context<Self>) {
        crate::hotkey::suspend(cx);
        self.capturing = Some(slot);
        window.focus(&self.capture_focus, cx);
        cx.notify();
    }

    /// 取消捕获并用当前配置恢复（重注册）全局热键。
    fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        if self.capturing.take().is_some() {
            if let Some(cfg) = cx.try_global::<GlobalConfig>().map(|g| g.0.clone()) {
                crate::hotkey::apply_shortcuts(&cfg.shortcuts, cx);
            }
            cx.notify();
        }
    }

    /// 捕获到一次按键：Esc 取消；合法组合则写入配置、落盘并即时重注册（含冲突提示）；
    /// 纯修饰键/不支持的键忽略（继续等待有效组合）。
    fn on_capture_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slot) = self.capturing else { return };
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        // 裸 Esc 取消。
        if ks.key == "escape" && !(m.control || m.alt || m.shift || m.platform) {
            self.cancel_capture(cx);
            return;
        }
        let Some(spec) = crate::hotkey::accelerator_from_keystroke(ks) else {
            return; // 等待「修饰键 + 主键」的有效组合
        };
        self.update_config(cx, |c| slot.set(&mut c.shortcuts, spec.clone()));
        self.capturing = None;
        self.apply_shortcuts_and_report(window, cx);
        cx.notify();
    }

    /// 「恢复默认快捷键」：重置三键为默认并即时生效。
    fn on_reset_shortcuts(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.capturing = None;
        self.update_config(cx, |c| c.shortcuts = ShortcutsConfig::default());
        self.apply_shortcuts_and_report(window, cx);
        cx.notify();
    }

    /// 改键后：落盘 + 重注册全部全局热键，并按结果给出提示。
    /// 优先提示「内部重复」（同一组合绑了多个动作，需用户处理），其次提示全局注册失败（被占用）。
    fn apply_shortcuts_and_report(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cfg) = cx.try_global::<GlobalConfig>().map(|g| g.0.clone()) else {
            return;
        };
        if let Err(e) = cfg.save() {
            tracing::error!("保存快捷键失败: {e:#}");
        }
        // 应用内快捷键不做 OS 注册；此处仅重注册三个全局热键（幂等，无论改的是哪类槽位）。
        let reg_conflicts = crate::hotkey::apply_shortcuts(&cfg.shortcuts, cx);
        let dups = duplicate_specs(&cfg.shortcuts);
        if !dups.is_empty() {
            notify(
                window,
                format!("{}：{}", tr("settings.shortcut_dup_warn"), dups.join("、")),
                cx,
            );
        } else if !reg_conflicts.is_empty() {
            notify(
                window,
                format!("{}：{}", tr("settings.shortcut_conflict"), reg_conflicts.join("、")),
                cx,
            );
        } else {
            notify(window, tr("settings.shortcut_updated"), cx);
        }
    }

    fn set_theme(&mut self, theme: &str, window: &mut Window, cx: &mut Context<Self>) {
        let t = theme.to_string();
        self.update_config(cx, |c| c.general.theme = t.clone());
        crate::theme::apply(&t, window, cx);
        cx.notify();
    }

    fn set_language(&mut self, lang: &str, window: &mut Window, cx: &mut Context<Self>) {
        let l = lang.to_string();
        self.update_config(cx, |c| c.general.language = l.clone());
        crate::i18n::apply_locale(&l);
        // 占位符在 InputState 构造时已固化，不随 tr() 重渲染更新——切语言后手动重设。
        self.refresh_placeholders(window, cx);
        window.refresh();
        cx.notify();
    }

    /// 重设各输入框占位符为当前 locale（切换语言后调用）。
    fn refresh_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let api_ph = tr("settings.api_key_ph");
        let dir_ph = tr("settings.audio_dir_ph");
        self.stream_api_key
            .update(cx, |s, cx| s.set_placeholder(api_ph.clone(), window, cx));
        self.off_api_key
            .update(cx, |s, cx| s.set_placeholder(api_ph, window, cx));
        self.audio_dir
            .update(cx, |s, cx| s.set_placeholder(dir_ph, window, cx));
    }

    // ───────────────────────────── 渲染辅助 ─────────────────────────────

    /// 左侧分类标签栏（竖排）。
    /// `round_bl`：本栏是否处于面板底部边缘（无底部页脚时为真），需把左下角倒成与面板一致的圆角，
    /// 否则方角的 sidebar 背景会盖掉面板的圆角边框（GPUI 的 overflow_hidden 只做矩形裁剪，不裁圆角）。
    fn render_tab_rail(&self, round_bl: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rail = v_flex()
            .w(px(132.))
            .flex_shrink_0()
            .h_full()
            .gap_0p5()
            .px_2()
            .py_2()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .when(round_bl, |r| r.rounded_bl(px(11.)));
        for (tab, key) in SettingsTab::ALL {
            let active = self.active_tab == tab;
            let mut item = div()
                .id(gpui::SharedString::from(format!("settings-tab-{key}")))
                .w_full()
                .px_3()
                .py_1()
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .child(tr(key))
                .on_click(cx.listener(move |this, _, _w, cx| {
                    // 离开当前标签时若正在改键，先恢复全局热键。
                    this.cancel_capture(cx);
                    this.active_tab = tab;
                    this.open_dropdown = Dropdown::None;
                    cx.notify();
                }));
            if active {
                item = item
                    .bg(cx.theme().list_active)
                    .text_color(cx.theme().foreground)
                    .font_weight(gpui::FontWeight::MEDIUM);
            } else {
                item = item
                    .text_color(cx.theme().muted_foreground)
                    .hover(|s| s.bg(cx.theme().list_hover));
            }
            rail = rail.child(item);
        }
        rail
    }

    fn field_label(&self, key: &str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .pt_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(tr(key))
    }

    /// api_key 输入框下方的动态提示：随所选后端显示其专属回退环境变量名。
    fn api_key_env_hint(&self, backend_id: &str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .pt_0p5()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(format!(
                "{} {}",
                tr("settings.api_key_env_hint"),
                crate::app::api_key_env_var(backend_id)
            ))
    }

    /// 所选后端的单次录音硬时长上限（秒），无后端侧限制时返回 None。
    /// 仅离线同步等受请求体大小限制的后端会返回 Some（届时录制将提前自动停止）。
    fn backend_cap_secs(id: &str) -> Option<u32> {
        BackendRegistry::with_builtins()
            .get(id)
            .and_then(|b| b.max_recording_seconds())
    }

    fn labeled(&self, label_key: &str, control: impl IntoElement) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_1()
            .gap_3()
            .child(div().child(tr(label_key)))
            .child(control)
    }

    /// 录音片段圆点图例：解释片段列表中每段前圆点颜色对应的转写模式。
    /// 与 `app::mode_dot`、主界面模式切换开关保持一致——实时=MODE_STREAMING，离线=MODE_OFFLINE。
    fn segment_legend(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let item = |color, label_key| {
            h_flex()
                .items_center()
                .gap_1p5()
                .child(div().flex_shrink_0().size(px(6.)).rounded_full().bg(color))
                .child(tr(label_key))
        };
        v_flex()
            .w_full()
            .gap_1()
            .pt_2()
            .text_xs()
            .text_color(muted)
            .child(tr("settings.segment_legend_hint"))
            .child(
                h_flex()
                    .gap_4()
                    .child(item(MODE_STREAMING, "mode.streaming"))
                    .child(item(MODE_OFFLINE, "mode.offline")),
            )
    }

    /// 「标签在左、控件在右」字段行（ASR 区用）：右侧控件列定宽，可在其中纵向叠放输入框 + 提示。
    /// 顶部对齐，使带提示的多行控件与标签对齐自然。
    fn field(&self, label_key: &str, right: impl IntoElement) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_start()
            .py_1()
            .gap_3()
            .child(div().pt_1().child(tr(label_key)))
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(FIELD_CONTROL_WIDTH))
                    .child(right),
            )
    }

    /// 把当前编辑模板的名称/提示词存回配置（切换/增删前调用，避免丢失编辑）。
    fn flush_current_polish_template(&self, cx: &mut Context<Self>) {
        let id = self.polish_edit_template.clone();
        if id.is_empty() {
            return;
        }
        let name = self.polish_name.read(cx).value().to_string();
        let prompt = self.polish_prompt.read(cx).value().to_string();
        self.update_config(cx, move |c| {
            if let Some(t) = c.polish.templates.iter_mut().find(|t| t.id == id) {
                t.prompt = prompt;
                if !crate::config::is_builtin_template(&id) && !name.trim().is_empty() {
                    t.name = name.trim().to_string();
                }
            }
        });
    }

    /// 载入某模板到名称/提示词输入框，并设为当前编辑项。
    fn load_polish_template(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let (name, prompt) = cx
            .try_global::<GlobalConfig>()
            .and_then(|g| {
                g.0.polish
                    .templates
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| (t.name.clone(), t.prompt.clone()))
            })
            .unwrap_or_default();
        self.polish_edit_template = id;
        self.polish_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.polish_prompt
            .update(cx, |s, cx| s.set_value(prompt, window, cx));
    }

    /// 切换选用/编辑的模板（兼作"当前润色模板"）。
    fn switch_polish_template(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_current_polish_template(cx);
        let active = id.clone();
        self.load_polish_template(id, window, cx);
        self.update_config(cx, move |c| c.polish.active_template = active.clone());
        cx.notify();
    }

    /// 新增一个自定义模板并选中编辑。
    fn add_polish_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_current_polish_template(cx);
        let id = format!("custom-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let name = tr("polish.new_template_name");
        let tpl = crate::config::PolishTemplate {
            id: id.clone(),
            name: name.clone(),
            prompt: String::new(),
        };
        let active = id.clone();
        self.update_config(cx, move |c| {
            c.polish.templates.push(tpl);
            c.polish.active_template = active.clone();
        });
        self.polish_edit_template = id;
        self.polish_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.polish_prompt
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    /// 删除当前自定义模板（内置模板不可删）。
    fn delete_polish_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.polish_edit_template.clone();
        if id.is_empty() || crate::config::is_builtin_template(&id) {
            return;
        }
        self.update_config(cx, move |c| {
            c.polish.templates.retain(|t| t.id != id);
            if !c.polish.templates.iter().any(|t| t.id == c.polish.active_template) {
                c.polish.active_template =
                    c.polish.templates.first().map(|t| t.id.clone()).unwrap_or_default();
            }
        });
        let new_id = cx
            .try_global::<GlobalConfig>()
            .map(|g| g.0.polish.active_template.clone())
            .unwrap_or_default();
        self.load_polish_template(new_id, window, cx);
        cx.notify();
    }

    /// AI 润色设置区：厂商预设 + base_url/model/key + 模板选择与提示词编辑。
    fn render_polish_settings(
        &self,
        cfg: &VoxInkConfig,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 厂商预设：点击填充 base_url。
        let mut presets = h_flex().w_full().flex_wrap().gap_1p5();
        for (i, (name, url)) in crate::polish::PROVIDER_PRESETS.iter().enumerate() {
            let url = url.to_string();
            presets = presets.child(
                Button::new(("polish-preset", i))
                    .outline()
                    .small()
                    .label(name.to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.polish_base_url
                            .update(cx, |s, cx| s.set_value(url.clone(), window, cx));
                        cx.notify();
                    })),
            );
        }

        // 模板选择：切换前先把当前模板的提示词编辑存回配置，避免丢失。
        let mut tpls = h_flex().w_full().flex_wrap().gap_1p5();
        for (i, t) in cfg.polish.templates.iter().enumerate() {
            let id = t.id.clone();
            let selected = t.id == self.polish_edit_template;
            tpls = tpls.child(
                Button::new(("polish-tpl", i))
                    .small()
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.outline())
                    .label(t.name.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_polish_template(id.clone(), window, cx)
                    })),
            );
        }
        // 「+ 新增」自定义模板。
        tpls = tpls.child(
            Button::new("polish-tpl-add")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .label(tr("polish.add_template"))
                .on_click(cx.listener(|this, _, window, cx| this.add_polish_template(window, cx))),
        );

        // 当前编辑的是自定义模板：可改名 + 可删除。
        let editing_custom = !self.polish_edit_template.is_empty()
            && !crate::config::is_builtin_template(&self.polish_edit_template);

        // 全部用整行输入（标签在上、输入框占满宽度），避免固定窄宽把长 URL 截断。
        v_flex()
            .w_full()
            .gap_1()
            .child(self.field_label("polish.preset", cx))
            .child(presets)
            .child(self.field_label("polish.base_url", cx))
            .child(Input::new(&self.polish_base_url))
            .child(self.field_label("polish.model", cx))
            .child(Input::new(&self.polish_model).small())
            .child(self.field_label("polish.api_key", cx))
            .child(Input::new(&self.polish_api_key).small())
            .child(
                div()
                    .pt_0p5()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{}{}",
                        tr("polish.api_key_env_hint"),
                        crate::polish::api_key_env_var(&self.polish_base_url.read(cx).value())
                    )),
            )
            .child(self.field_label("polish.template", cx))
            .child(tpls)
            // 自定义模板：名称输入 + 删除按钮（内置模板不显示此行）。
            .when(editing_custom, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().child(Input::new(&self.polish_name).small()))
                        .child(
                            Button::new("polish-tpl-del")
                                .outline()
                                .small()
                                .icon(IconName::Delete)
                                .label(tr("polish.delete_template"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.delete_polish_template(window, cx)
                                })),
                        ),
                )
            })
            .child(self.field_label("polish.prompt", cx))
            .child(Input::new(&self.polish_prompt))
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("polish.settings_hint")),
            )
    }

    /// 「测试连接」按钮行：右对齐、按钮取自然宽度（不占整行）。
    fn test_button(
        &self,
        id: &'static str,
        streaming: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex().w_full().justify_end().py_1().child(
            Button::new(id)
                .outline()
                .small()
                .label(tr("settings.test"))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.on_test(streaming, window, cx)),
                ),
        )
    }

    /// 自绘下拉：当前项按钮 + 展开时内联列表。
    fn render_dropdown(
        &self,
        kind: Dropdown,
        current_id: &str,
        options: Vec<crate::asr::registry::BackendInfo>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let is_open = self.open_dropdown == kind;
        let current_name = options
            .iter()
            .find(|b| b.id == current_id)
            .map(|b| backend_label(&b.id, &b.display_name))
            .unwrap_or_else(|| current_id.to_string());
        let id_prefix = match kind {
            Dropdown::Streaming => "dd-stream",
            _ => "dd-offline",
        };

        let mut col = v_flex().w_full().gap_1().child(
            div()
                .id(gpui::SharedString::from(format!("{id_prefix}-toggle")))
                .w_full()
                .px_2p5()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .flex()
                .justify_between()
                .items_center()
                .cursor_pointer()
                .hover(|s| s.bg(cx.theme().muted))
                .child(current_name)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if is_open { "▲" } else { "▼" }),
                )
                .on_click(cx.listener(move |this, _, _w, cx| {
                    this.open_dropdown = if this.open_dropdown == kind {
                        Dropdown::None
                    } else {
                        kind
                    };
                    cx.notify();
                })),
        );

        if is_open {
            let mut list = v_flex()
                .w_full()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(px(6.))
                .bg(cx.theme().background);
            for b in options {
                let id = b.id.clone();
                let active = b.id == current_id;
                let name = backend_label(&b.id, &b.display_name);
                let mut item = div()
                    .id(gpui::SharedString::from(format!("{id_prefix}-{}", b.id)))
                    .w_full()
                    .px_2p5()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(cx.theme().muted))
                    .child(name)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.on_select_backend(kind, id.clone(), window, cx)
                    }));
                if active {
                    item = item.bg(cx.theme().list_active);
                }
                list = list.child(item);
            }
            col = col.child(list);
        }
        col
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cfg = cx
            .try_global::<GlobalConfig>()
            .map(|g| g.0.clone())
            .unwrap_or_default();
        let theme = cfg.general.theme.clone();
        let lang = crate::i18n::normalize_locale(&cfg.general.language);
        let mode = cfg.asr.default_mode;

        // 按能力筛选并按 id 排序（避免 HashMap 顺序抖动）。
        let mut all = BackendRegistry::with_builtins().list();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        let streaming_opts: Vec<_> = all
            .iter()
            .filter(|b| b.supports_streaming)
            .cloned()
            .collect();
        let offline_opts: Vec<_> = all.iter().filter(|b| b.supports_offline).cloned().collect();
        let off_is_filetrans = cfg.asr.offline_backend == FILETRANS_ID;
        // 离线与实时选同一后端（如 Qwen3-ASR 自建服务）时，共用一份配置：离线区不再重复渲染输入框。
        let off_shared = cfg.asr.offline_backend == cfg.asr.streaming_backend;

        let body = v_flex()
            .id("settings-body")
            .size_full()
            .min_h_0()
            .justify_start()
            .gap_1()
            .px_4()
            // 右侧多留出滚动条宽度，避免整行内容（如润色厂商 chips）被滚动条遮挡裁切。
            .pr_6()
            .py_2()
            .text_size(px(13.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            // ── ASR ──
            .when(self.active_tab == SettingsTab::Asr, |this| {
                this.child(self.field(
                    "settings.streaming_backend",
                    self.render_dropdown(
                        Dropdown::Streaming,
                        &cfg.asr.streaming_backend,
                        streaming_opts,
                        cx,
                    ),
                ))
                .child(
                    self.field(
                        "settings.api_key",
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(Input::new(&self.stream_api_key).small())
                            .child(self.api_key_env_hint(&cfg.asr.streaming_backend, cx)),
                    ),
                )
                .child(self.field(
                    "settings.endpoint",
                    Input::new(&self.stream_endpoint).small(),
                ))
                .child(self.test_button("test-stream", true, cx))
                // ── ASR：离线 ──
                .child(self.field(
                    "settings.offline_backend",
                    self.render_dropdown(
                        Dropdown::Offline,
                        &cfg.asr.offline_backend,
                        offline_opts,
                        cx,
                    ),
                ))
                // 所选离线后端有硬时长上限（如离线同步受 ~10MB 请求体限制）时，提示能力受限 + 会自动停止。
                .when_some(Self::backend_cap_secs(&cfg.asr.offline_backend), |this, cap| {
                    this.child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(DANGER)
                            .child(
                                rust_i18n::t!("settings.offline_cap_hint", min => cap / 60)
                                    .to_string(),
                            ),
                    )
                })
                // 与实时共用同一后端时，仅给出提示，不重复渲染输入框（避免双份编辑互相覆盖）。
                .when(off_shared, |this| {
                    this.child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("settings.shared_config_hint")),
                    )
                })
                .when(!off_shared, |this| {
                    this.child(
                        self.field(
                            "settings.api_key",
                            v_flex()
                                .w_full()
                                .gap_1()
                                .child(Input::new(&self.off_api_key).small())
                                .child(self.api_key_env_hint(&cfg.asr.offline_backend, cx)),
                        ),
                    )
                    .child(self.field("settings.endpoint", Input::new(&self.off_endpoint).small()))
                    // 大文件后端额外的 OSS 参数
                    .when(off_is_filetrans, |this| {
                        this.child(
                            div()
                                .pt_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("settings.oss_hint")),
                        )
                        .child(self.field(
                            "settings.oss_endpoint",
                            Input::new(&self.off_oss_endpoint).small(),
                        ))
                        .child(self.field(
                            "settings.oss_bucket",
                            Input::new(&self.off_oss_bucket).small(),
                        ))
                        .child(self.field(
                            "settings.oss_ak_id",
                            Input::new(&self.off_oss_ak_id).small(),
                        ))
                        .child(self.field(
                            "settings.oss_ak_secret",
                            Input::new(&self.off_oss_ak_secret).small(),
                        ))
                    })
                })
                .child(self.test_button("test-offline", false, cx))
            })
            // ── 录音 ──
            .when(self.active_tab == SettingsTab::Recording, |this| {
                this.child(self.labeled("settings.default_mode", self.mode_choice(mode, cx)))
                    .child(
                        self.labeled(
                            "settings.auto_copy",
                            Switch::new("auto-copy")
                                .checked(cfg.text.auto_copy)
                                .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                                    let v = *checked;
                                    this.update_config(cx, |c| c.text.auto_copy = v);
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        self.labeled(
                            "settings.audio_feedback",
                            Switch::new("audio-feedback")
                                .checked(cfg.general.audio_feedback)
                                .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                                    let v = *checked;
                                    this.update_config(cx, |c| c.general.audio_feedback = v);
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(self.labeled(
                        "settings.max_seconds",
                        div().w(px(120.)).child(Input::new(&self.max_secs).small()),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("settings.max_seconds_hint")),
                    )
                    .child(self.segment_legend(cx))
            })
            // ── AI 润色 ──
            .when(self.active_tab == SettingsTab::Polish, |this| {
                this.child(self.render_polish_settings(&cfg, cx))
            })
            // ── 通用 ──
            .when(self.active_tab == SettingsTab::General, |this| {
                this.child(
                    self.labeled(
                        "settings.autostart",
                        Switch::new("autostart")
                            .checked(cfg.general.launch_at_startup)
                            .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                                let v = *checked;
                                this.update_config(cx, |c| c.general.launch_at_startup = v);
                                if let Err(e) = crate::autolaunch::set_enabled(v) {
                                    tracing::warn!("设置开机自启失败: {e:#}");
                                }
                                cx.notify();
                            })),
                    ),
                )
                .child(
                    self.labeled(
                        "settings.minimized",
                        Switch::new("minimized")
                            .checked(cfg.general.start_minimized)
                            .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                                let v = *checked;
                                this.update_config(cx, |c| c.general.start_minimized = v);
                                cx.notify();
                            })),
                    ),
                )
                .child(
                    self.labeled(
                        "settings.on_top",
                        Switch::new("on-top")
                            .checked(cfg.general.window_on_top)
                            .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                                let v = *checked;
                                this.update_config(cx, |c| c.general.window_on_top = v);
                                cx.notify();
                            })),
                    ),
                )
                .child(self.labeled("settings.theme", self.theme_choice(&theme, cx)))
                .child(self.labeled("settings.language", self.lang_choice(lang, cx)))
            })
            // ── 快捷键 ──
            .when(self.active_tab == SettingsTab::Shortcuts, |this| {
                this.child(self.render_shortcuts(&cfg, cx))
            })
            // ── 数据 ──
            .when(self.active_tab == SettingsTab::Data, |this| {
                this.child(
                    self.labeled(
                        "settings.save_audio",
                        Switch::new("save-audio")
                            .checked(cfg.storage.save_audio)
                            .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                                let v = *checked;
                                this.update_config(cx, |c| c.storage.save_audio = v);
                                cx.notify();
                            })),
                    ),
                )
                .child(self.field_label("settings.audio_dir", cx))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.audio_dir).small()))
                        .child(
                            Button::new("audio-browse")
                                .outline()
                                .small()
                                .label(tr("settings.browse"))
                                .on_click(cx.listener(Self::on_browse_audio_dir)),
                        )
                        .child(
                            Button::new("audio-open")
                                .outline()
                                .small()
                                .label(tr("settings.open_folder"))
                                .on_click(cx.listener(Self::on_open_audio_dir)),
                        ),
                )
                .child(
                    div()
                        .pt_0p5()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("settings.audio_dir_hint")),
                )
                .child(
                    self.labeled(
                        "settings.text_retention",
                        div()
                            .w(px(120.))
                            .child(Input::new(&self.text_retention).small()),
                    ),
                )
                .child(
                    self.labeled(
                        "settings.audio_retention",
                        div()
                            .w(px(120.))
                            .child(Input::new(&self.audio_retention).small()),
                    ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .py_1()
                        .child(div().child(format!(
                            "{} {}",
                            tr("settings.audio_usage"),
                            human_size(self.audio_usage_bytes)
                        )))
                        .child(
                            Button::new("audio-clean")
                                .outline()
                                .small()
                                .label(tr("settings.clean_audio"))
                                .on_click(cx.listener(Self::on_clean_audio)),
                        ),
                )
                .child(
                    div().pt_3().child(
                        Button::new("export-history")
                            .outline()
                            .small()
                            .label(tr("settings.export_history"))
                            .on_click(cx.listener(Self::on_export_history)),
                    ),
                )
                .child(
                    div()
                        .pt_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("settings.export_history_hint")),
                )
            })
            // ── 关于 ──
            .when(self.active_tab == SettingsTab::About, |this| {
                this.child(self.about_row(
                    "about.version",
                    &format!("v{}", crate::diagnostics::VERSION),
                    cx,
                ))
                    .child(self.about_row(
                        "about.build",
                        &crate::diagnostics::build_time_display(),
                        cx,
                    ))
                    .child(self.about_row("about.commit", crate::diagnostics::GIT_HASH, cx))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .py_0p5()
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("about.repo")),
                            )
                            .child(
                                // 可点击链接：用与其它行一致的正文字号（不用 Button::link，那会偏大），
                                // 显示完整 https 地址。
                                div()
                                    .id("github-link")
                                    .cursor_pointer()
                                    .text_color(cx.theme().link)
                                    .child(crate::update::repo_url())
                                    .on_click(cx.listener(Self::on_open_github)),
                            ),
                    )
                    .child(
                        h_flex()
                            .pt_2()
                            .gap_2()
                            .child(
                                Button::new("export-diag")
                                    .outline()
                                    .small()
                                    .label(tr("about.export_diag"))
                                    .on_click(cx.listener(Self::on_export_diag)),
                            )
                            .child(
                                Button::new("open-logs")
                                    .outline()
                                    .small()
                                    .label(tr("about.open_logs"))
                                    .on_click(cx.listener(Self::on_open_logs)),
                            ),
                    )
                    .child(self.render_update_section(&cfg, cx))
            });

        // 面板尺寸跟随主窗：窗口越大面板越大，留出边距并设上下限。
        // 宽度保证缩到最小（640）时不溢出、放大时也不至于过宽；
        // 高度保证缩矮时不被遮挡——面板随之变矮，由正文区内部滚动条承接溢出，
        // 顶部标题栏与底部保存按钮（flex_shrink_0）始终可见。
        let viewport = window.viewport_size();
        let panel_w = (f32::from(viewport.width) - 80.0).clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH);
        let panel_h = (f32::from(viewport.height) - 60.0).clamp(PANEL_MIN_HEIGHT, PANEL_MAX_HEIGHT);

        // 覆盖层：半透明遮罩 + 居中面板。`.occlude()` 拦截鼠标，防止穿透到主视图误触录音。
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000099))
            .child(
                v_flex()
                    .w(px(panel_w))
                    .h(px(panel_h))
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(12.))
                    // 裁掉超出面板的滚动内容。注意：GPUI 的 content mask 只做矩形裁剪、不裁圆角，
                    // 故底部页脚/左侧栏的圆角需各自单独倒（见 footer.rounded_b / rail round_bl）。
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .justify_between()
                            .items_center()
                            .w_full()
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(tr("settings.title")),
                            )
                            .child(
                                Button::new("settings-close")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Close)
                                    .on_click(cx.listener(Self::on_close)),
                            ),
                    )
                    .child(
                        // 左侧标签栏 + 右侧内容区（带常驻可见滚动条）。
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .child(self.render_tab_rail(!self.tab_is_editable(), cx))
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    // min_w_0：否则长内容（URL/提示文字/整行输入）会把内容区
                                    // 撑出对话框右边界、整列被裁切（CLAUDE.md 既有坑）。
                                    .min_w_0()
                                    .h_full()
                                    .min_h_0()
                                    .child(body)
                                    .child(
                                        div().absolute().inset_0().child(
                                            Scrollbar::vertical(&self.scroll)
                                                .id("settings-scrollbar")
                                                .scrollbar_show(ScrollbarShow::Always),
                                        ),
                                    ),
                            ),
                    )
                    // 可保存页面底部：保存按钮（右下角）。
                    .when(self.tab_is_editable(), |this| {
                        this.child(
                            h_flex()
                                .w_full()
                                .flex_shrink_0()
                                .justify_end()
                                .items_center()
                                .px_4()
                                .py_2()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                // 页脚是面板底部边缘元素，倒底部圆角与面板一致，
                                // 否则方角背景会盖掉面板圆角（content mask 不裁圆角）。
                                .rounded_b(px(11.))
                                .child(
                                    Button::new("settings-save")
                                        .primary()
                                        .small()
                                        .label(tr("settings.save"))
                                        .on_click(cx.listener(Self::on_save)),
                                ),
                        )
                    }),
            )
    }
}

impl SettingsView {
    fn mode_choice(
        &self,
        mode: TranscriptionMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let seg = |id: &'static str,
                   key: &'static str,
                   val: TranscriptionMode,
                   cx: &mut Context<Self>| {
            let active = mode == val;
            Button::new(id)
                .when(active, |b| b.primary())
                .when(!active, |b| b.outline())
                .small()
                .label(tr(key))
                .on_click(cx.listener(move |this, _, _w, cx| {
                    this.update_config(cx, |c| c.asr.default_mode = val);
                    cx.notify();
                }))
        };
        h_flex()
            .gap_2()
            .child(seg("mode-s", "mode.streaming", TranscriptionMode::Streaming, cx))
            .child(seg("mode-o", "mode.offline", TranscriptionMode::Offline, cx))
            .child(seg(
                "mode-r",
                "mode.recordonly",
                TranscriptionMode::RecordOnly,
                cx,
            ))
    }

    fn theme_choice(&self, theme: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let cur = theme.to_string();
        let btn = |id: &'static str,
                   key: &'static str,
                   val: &'static str,
                   cur: &str,
                   cx: &mut Context<Self>| {
            let active = cur == val;
            Button::new(id)
                .when(active, |b| b.primary())
                .when(!active, |b| b.outline())
                .small()
                .label(tr(key))
                .on_click(cx.listener(move |this, _, window, cx| this.set_theme(val, window, cx)))
        };
        h_flex()
            .gap_2()
            .child(btn("th-light", "theme.light", "light", &cur, cx))
            .child(btn("th-dark", "theme.dark", "dark", &cur, cx))
            .child(btn("th-system", "theme.system", "system", &cur, cx))
    }

    fn lang_choice(&self, lang: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let zh = lang == "zh-CN";
        h_flex()
            .gap_2()
            .child(
                Button::new("lang-zh")
                    .when(zh, |b| b.primary())
                    .when(!zh, |b| b.outline())
                    .small()
                    .label("中文")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.set_language("zh-CN", window, cx)),
                    ),
            )
            .child(
                Button::new("lang-en")
                    .when(!zh, |b| b.primary())
                    .when(zh, |b| b.outline())
                    .small()
                    .label("English")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.set_language("en", window, cx)),
                    ),
            )
    }

    /// 快捷键改键区：三行可点击改键的「键帽」+ 提示 + 恢复默认。
    /// 容器 track_focus + on_key_down，捕获期间聚焦它以接收按键。
    fn render_shortcuts(&self, cfg: &VoxInkConfig, cx: &mut Context<Self>) -> impl IntoElement {
        let mut pane = v_flex()
            .id("shortcuts-pane")
            .track_focus(&self.capture_focus)
            .on_key_down(cx.listener(Self::on_capture_key))
            .w_full()
            .gap_1()
            // ── 全局快捷键 ──
            .child(self.shortcuts_section_header("settings.shortcuts.global", cx));
        for slot in ShortcutSlot::GLOBAL {
            pane = pane.child(self.shortcut_row(slot, cfg, cx));
        }
        pane = pane
            .child(self.shortcuts_hint("settings.shortcuts.global_hint", cx))
            // ── 应用内快捷键 ──
            .child(self.shortcuts_section_header("settings.shortcuts.in_app", cx));
        for slot in ShortcutSlot::IN_APP {
            pane = pane.child(self.shortcut_row(slot, cfg, cx));
        }
        pane.child(self.shortcuts_hint("settings.shortcuts.in_app_hint", cx))
            .child(self.shortcuts_hint("settings.shortcuts_hint", cx))
            .child(
                div().pt_2().child(
                    Button::new("shortcut-reset")
                        .outline()
                        .small()
                        .label(tr("settings.shortcuts_reset"))
                        .on_click(cx.listener(Self::on_reset_shortcuts)),
                ),
            )
    }

    /// 快捷键分节标题（「全局快捷键」/「应用内快捷键」）。
    fn shortcuts_section_header(&self, key: &str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .pt_3()
            .pb_0p5()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .child(tr(key))
    }

    /// 快捷键区的浅色说明文字。
    fn shortcuts_hint(&self, key: &str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .pt_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(tr(key))
    }

    /// 单行：标题 + 冲突标记 + 可点击的快捷键「键帽」。
    /// 冲突时键帽边框转危险色；标记区分「内部重复」与「全局注册失败」。
    fn shortcut_row(
        &self,
        slot: ShortcutSlot,
        cfg: &VoxInkConfig,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = &cfg.shortcuts;
        let binding = slot.get(s);
        let capturing = self.capturing == Some(slot);
        let duplicated = slot_duplicated(slot, s);
        // 注册失败仅对全局槽位有意义，且重复时优先报「冲突」不再叠加「注册失败」。
        let reg_failed = !duplicated
            && slot
                .global_action()
                .is_some_and(|a| crate::hotkey::registration_failed(a, cx));
        let conflict_key = if duplicated {
            Some("settings.shortcut_duplicate")
        } else if reg_failed {
            Some("settings.shortcut_failed")
        } else {
            None
        };

        let display = if capturing {
            tr("settings.shortcut_capturing")
        } else if binding.trim().is_empty() {
            "—".to_string()
        } else {
            binding.to_string()
        };
        let border = if capturing {
            BRAND
        } else if conflict_key.is_some() {
            DANGER
        } else {
            cx.theme().border
        };

        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_1()
            .child(div().child(tr(slot.label_key())))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .when_some(conflict_key, |this, k| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(DANGER)
                                .child(format!("⚠ {}", tr(k))),
                        )
                    })
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!(
                                "sc-cap-{}",
                                slot.label_key()
                            )))
                            .flex()
                            .justify_center()
                            .min_w(px(150.))
                            .px_3()
                            .py_1()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(border)
                            .bg(cx.theme().muted)
                            .text_xs()
                            .cursor_pointer()
                            .hover(|s| s.border_color(BRAND))
                            .child(display)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.begin_capture(slot, window, cx)
                            })),
                    ),
            )
    }

    fn about_row(&self, label_key: &str, value: &str, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_0p5()
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(label_key)),
            )
            .child(div().child(value.to_string()))
    }

    /// 「关于」区更新检查/下载子区（M13，§11.3）。
    fn render_update_section(
        &self,
        cfg: &VoxInkConfig,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = matches!(
            self.update_status,
            UpdateStatus::Checking | UpdateStatus::Downloading
        );

        let mut col = v_flex()
            .w_full()
            .gap_2()
            .pt_3()
            .mt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            // 顶部一行：检查按钮 + 简要状态文本。
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("check-update")
                            .outline()
                            .small()
                            .label(tr("about.check_update"))
                            .disabled(busy)
                            .on_click(cx.listener(Self::on_check_update)),
                    )
                    .child(self.update_status_text(cx)),
            );

        match &self.update_status {
            UpdateStatus::Available(rel) => {
                if !rel.changelog.trim().is_empty() {
                    col = col.child(
                        div()
                            .w_full()
                            .max_h(px(160.))
                            .overflow_hidden()
                            .p_2()
                            .rounded(px(6.))
                            .bg(cx.theme().muted)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(rel.changelog.clone()),
                    );
                }
                col = col.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("update-now")
                                .primary()
                                .small()
                                .label(tr("about.update_now"))
                                .on_click(cx.listener(Self::on_do_update)),
                        )
                        .child(
                            Button::new("skip-version")
                                .outline()
                                .small()
                                .label(tr("about.skip_version"))
                                .on_click(cx.listener(Self::on_skip_version)),
                        )
                        .child(
                            Button::new("release-page")
                                .ghost()
                                .small()
                                .label(tr("about.release_page"))
                                .on_click(cx.listener(Self::on_open_release_page)),
                        ),
                );
            }
            UpdateStatus::Downloading => {
                let pct = self.update_progress.load(Ordering::Relaxed);
                col = col.child(
                    v_flex()
                        .w_full()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} {pct}%", tr("about.downloading"))),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(6.))
                                .rounded(px(3.))
                                .bg(cx.theme().muted)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded(px(3.))
                                        .bg(BRAND)
                                        .w(gpui::relative(pct as f32 / 100.0)),
                                ),
                        ),
                );
            }
            UpdateStatus::Failed(msg) => {
                col = col.child(div().text_xs().text_color(DANGER).child(msg.clone())).child(
                    Button::new("release-page-fail")
                        .ghost()
                        .small()
                        .label(tr("about.release_page"))
                        .on_click(cx.listener(Self::on_open_release_page)),
                );
            }
            _ => {}
        }

        // 手动下载兜底：部分用户因网络问题无法在线检查/更新，始终给出 Releases 地址。
        col = col.child(
            v_flex()
                .w_full()
                .gap_1()
                .pt_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("about.manual_download_hint")),
                )
                .child(
                    div()
                        .id("releases-link")
                        .cursor_pointer()
                        .text_xs()
                        .text_color(cx.theme().link)
                        .child(crate::update::releases_url())
                        .on_click(cx.listener(Self::on_open_releases)),
                ),
        );

        col.child(
            self.labeled(
                "about.auto_check",
                Switch::new("auto-check")
                    .checked(cfg.general.auto_check_update)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        let v = *checked;
                        this.update_config(cx, |c| c.general.auto_check_update = v);
                        cx.notify();
                    })),
            ),
        )
    }

    /// 更新状态的简要文本（随状态变色）。
    fn update_status_text(&self, cx: &Context<Self>) -> impl IntoElement {
        let (text, danger) = match &self.update_status {
            UpdateStatus::Idle | UpdateStatus::Downloading => (String::new(), false),
            UpdateStatus::Checking => (tr("about.checking"), false),
            UpdateStatus::UpToDate => (tr("about.up_to_date"), false),
            UpdateStatus::Available(rel) => {
                (format!("{} v{}", tr("about.new_version"), rel.version), false)
            }
            UpdateStatus::Failed(_) => (tr("about.check_failed"), true),
        };
        div()
            .text_sm()
            .text_color(if danger {
                DANGER
            } else {
                cx.theme().muted_foreground
            })
            .child(text)
    }
}

/// 后端显示名按当前 locale 本地化：查 `backend.{id}` 翻译键；缺失时回退后端自报名。
/// （后端 trait 的 `display_name` 是硬编码中文，下拉里改用此函数随语言切换。）
fn backend_label(id: &str, fallback: &str) -> String {
    let key = format!("backend.{id}");
    let s = tr(&key);
    if s == key { fallback.to_string() } else { s }
}

/// 规范化快捷键字符串用于比较：去空白 + 小写（捕获产出的形如 "Ctrl+Shift+C" 大小写一致，
/// 此处仅作稳健兜底）。
fn norm_spec(spec: &str) -> String {
    spec.trim().to_ascii_lowercase()
}

/// 全部快捷键槽位（全局 + 应用内），用于跨节冲突检测。
fn all_slots() -> impl Iterator<Item = ShortcutSlot> {
    ShortcutSlot::GLOBAL.into_iter().chain(ShortcutSlot::IN_APP)
}

/// 某槽位的绑定是否与其它槽位重复（非空且规范化后相等）。
fn slot_duplicated(slot: ShortcutSlot, s: &ShortcutsConfig) -> bool {
    let v = norm_spec(slot.get(s));
    if v.is_empty() {
        return false;
    }
    all_slots().any(|o| o != slot && norm_spec(o.get(s)) == v)
}

/// 返回被多个动作共用（重复）的快捷键原始字符串列表（去重，用于提示）。
fn duplicate_specs(s: &ShortcutsConfig) -> Vec<String> {
    let mut dups: Vec<String> = Vec::new();
    for slot in all_slots() {
        let raw = slot.get(s).trim();
        if raw.is_empty() || !slot_duplicated(slot, s) {
            continue;
        }
        if !dups.iter().any(|d| norm_spec(d) == norm_spec(raw)) {
            dups.push(raw.to_string());
        }
    }
    dups
}

/// 递归统计目录内文件总字节数（用于占用显示；忽略错误项）。
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for e in entries.flatten() {
        match e.file_type() {
            Ok(ft) if ft.is_dir() => total += dir_size(&e.path()),
            Ok(ft) if ft.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => {}
        }
    }
    total
}

/// 人类可读的字节数（B/KB/MB/GB）。
fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.2} GB", b / (KB * KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shortcuts_have_no_duplicates() {
        let s = ShortcutsConfig::default();
        assert!(duplicate_specs(&s).is_empty());
        assert!(ShortcutSlot::GLOBAL
            .into_iter()
            .chain(ShortcutSlot::IN_APP)
            .all(|slot| !slot_duplicated(slot, &s)));
    }

    #[test]
    fn detects_duplicate_across_sections() {
        let mut s = ShortcutsConfig::default();
        // 让应用内「复制全部」与全局「复制并粘贴」撞成同一组合（忽略大小写）。
        s.app_copy_all = s.copy_and_paste.to_ascii_lowercase();
        assert!(slot_duplicated(ShortcutSlot::CopyAll, &s));
        assert!(slot_duplicated(ShortcutSlot::Paste, &s));
        assert!(!slot_duplicated(ShortcutSlot::NewRecord, &s));
        // 仅报告一次该重复组合。
        assert_eq!(duplicate_specs(&s).len(), 1);
    }

    #[test]
    fn empty_binding_is_not_a_duplicate() {
        let mut s = ShortcutsConfig::default();
        s.app_copy_all = String::new();
        s.app_new_record = String::new();
        assert!(!slot_duplicated(ShortcutSlot::CopyAll, &s));
        assert!(duplicate_specs(&s).is_empty());
    }
}
