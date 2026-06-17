//! 设置面板（M11 任务 11.1/11.4，§6.4；2026-06-14 重构为按模式独立配置后端）。
//!
//! 作为全屏覆盖层渲染在主视图之上（不依赖 Sheet/Dialog 浮层，避免父子视图互租借）。
//! 自身只读写 `GlobalConfig` 与全局 locale/theme，关闭时发 [`SettingsEvent::Closed`] 由主视图收起。
//!
//! ASR 区：**实时**与**离线**各有独立下拉选择后端实现，并各自配置 api_key / endpoint；
//! 离线选「大文件」后端时额外显示 OSS 参数。下拉为自绘内联展开列表（避免浮层裁剪/复杂依赖）。

use gpui::{
    div, prelude::*, px, rgba, ClickEvent, Context, Entity, EventEmitter, IntoElement,
    ParentElement, Render, ScrollHandle, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    scroll::{Scrollbar, ScrollbarShow},
    switch::Switch,
    v_flex, ActiveTheme, IconName, Sizable, WindowExt,
};

use crate::app::{friendly_asr_error, runtime_asr_config, GlobalConfig, GlobalTokioHandle};
use crate::asr::{AsrError, BackendRegistry};
use crate::config::VoxInkConfig;
use crate::i18n::tr;
use crate::state::TranscriptionMode;

const FILETRANS_ID: &str = "aliyun_bailian_filetrans";

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

/// 设置分类标签（左侧栏）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Asr,
    Recording,
    General,
    Shortcuts,
    Data,
    About,
}

impl SettingsTab {
    /// 左侧栏顺序与对应标题 locale key。
    const ALL: [(SettingsTab, &'static str); 6] = [
        (SettingsTab::Asr, "settings.section.asr"),
        (SettingsTab::Recording, "settings.section.recording"),
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
    /// 当前选中的分类标签。
    active_tab: SettingsTab,
    /// 内容区滚动句柄（驱动可见滚动条）。
    scroll: ScrollHandle,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx));
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
            audio_dir: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr("settings.audio_dir_ph"))
            }),
            audio_retention: input(window, cx),
            text_retention: input(window, cx),
            audio_usage_bytes: 0,
            active_tab: SettingsTab::Asr,
            scroll: ScrollHandle::new(),
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
        self.audio_dir
            .update(cx, |s, cx| s.set_value(c.storage.audio_dir.clone(), window, cx));
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
    }

    fn load_stream_inputs(&mut self, c: &VoxInkConfig, window: &mut Window, cx: &mut Context<Self>) {
        let b = c.asr.backend(&c.asr.streaming_backend);
        self.stream_api_key
            .update(cx, |s, cx| s.set_value(b.api_key.clone(), window, cx));
        self.stream_endpoint
            .update(cx, |s, cx| s.set_value(b.endpoint.clone(), window, cx));
    }

    fn load_offline_inputs(&mut self, c: &VoxInkConfig, window: &mut Window, cx: &mut Context<Self>) {
        let b = c.asr.backend(&c.asr.offline_backend);
        self.off_api_key
            .update(cx, |s, cx| s.set_value(b.api_key.clone(), window, cx));
        self.off_endpoint
            .update(cx, |s, cx| s.set_value(b.endpoint.clone(), window, cx));
        self.off_oss_endpoint
            .update(cx, |s, cx| s.set_value(b.oss_endpoint.clone(), window, cx));
        self.off_oss_bucket
            .update(cx, |s, cx| s.set_value(b.oss_bucket.clone(), window, cx));
        self.off_oss_ak_id
            .update(cx, |s, cx| s.set_value(b.oss_access_key_id.clone(), window, cx));
        self.off_oss_ak_secret
            .update(cx, |s, cx| s.set_value(b.oss_access_key_secret.clone(), window, cx));
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
                c.asr.max_recording_seconds = n;
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
        window.push_notification(format!("{}（{backend_id}）…", tr("settings.test")), cx);

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
                Ok(Ok(())) => window.push_notification("连接测试成功 ✓", cx),
                Ok(Err(e)) => {
                    window.push_notification(format!("连接测试失败：{}", friendly_asr_error(&e)), cx)
                }
                Err(_) => window.push_notification("连接测试中断", cx),
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
                    this.audio_dir.update(cx, |st, cx| st.set_value(s.clone(), window, cx));
                    this.update_config(cx, |c| c.storage.audio_dir = s);
                    this.audio_usage_bytes =
                        this.current_audio_root(cx).map(|r| dir_size(&r)).unwrap_or(0);
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
            window.push_notification("无法打开目录", cx);
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
        self.audio_usage_bytes = self.current_audio_root(cx).map(|r| dir_size(&r)).unwrap_or(0);
        window.push_notification(format!("已清理 {removed} 个过期录音"), cx);
        cx.notify();
    }

    /// 导出全部历史记录为 JSON（2026-06-16 从主界面标题栏迁入「数据」区）。
    fn on_export_history(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        match crate::app::export_history_json(cx) {
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

    fn on_export_diag(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let config = cx.try_global::<GlobalConfig>().map(|g| g.0.clone());
        let Some(config) = config else { return };
        match crate::diagnostics::export(&config) {
            Ok(path) => {
                tracing::info!("诊断已导出: {}", path.display());
                window.push_notification(format!("已导出到 {}", path.display()), cx);
            }
            Err(e) => {
                tracing::error!("导出诊断失败: {e:#}");
                window.push_notification("导出诊断失败", cx);
            }
        }
    }

    /// 保存当前页配置：把输入框并入内存配置并落盘（停留在面板，给出反馈）。
    fn on_save(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_inputs_to_config(cx);
        if let Some(c) = cx.try_global::<GlobalConfig>().map(|g| g.0.clone())
            && let Err(e) = c.save()
        {
            tracing::error!("保存配置失败: {e:#}");
        }
        window.push_notification(tr("settings.saved"), cx);
    }

    /// 关闭设置面板（右上角 X）。即时生效项（开关/主题等）已落入内存配置，
    /// 退出应用时统一持久化；未点「保存」的输入框文本不写盘。
    fn on_close(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::Closed);
    }

    /// 当前标签是否含可保存配置（关于/快捷键为只读）。
    fn tab_is_editable(&self) -> bool {
        matches!(
            self.active_tab,
            SettingsTab::Asr | SettingsTab::Recording | SettingsTab::General | SettingsTab::Data
        )
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
        window.refresh();
        cx.notify();
    }

    // ───────────────────────────── 渲染辅助 ─────────────────────────────

    /// 左侧分类标签栏（竖排）。
    fn render_tab_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rail = v_flex()
            .w(px(132.))
            .flex_shrink_0()
            .h_full()
            .gap_0p5()
            .px_2()
            .py_2()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar);
        for (tab, key) in SettingsTab::ALL {
            let active = self.active_tab == tab;
            let mut item = div()
                .id(gpui::SharedString::from(format!("settings-tab-{key}")))
                .w_full()
                .px_3()
                .py_1p5()
                .rounded(px(6.))
                .text_sm()
                .cursor_pointer()
                .child(tr(key))
                .on_click(cx.listener(move |this, _, _w, cx| {
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

    fn labeled(&self, label_key: &str, control: impl IntoElement) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_1()
            .gap_3()
            .child(div().text_sm().child(tr(label_key)))
            .child(control)
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
            .map(|b| b.display_name.clone())
            .unwrap_or_else(|| current_id.to_string());
        let id_prefix = match kind {
            Dropdown::Streaming => "dd-stream",
            _ => "dd-offline",
        };

        let mut col = v_flex().w_full().gap_1().child(
            div()
                .id(gpui::SharedString::from(format!("{id_prefix}-toggle")))
                .w_full()
                .px_3()
                .py_1p5()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .flex()
                .justify_between()
                .items_center()
                .cursor_pointer()
                .hover(|s| s.bg(cx.theme().muted))
                .child(current_name)
                .child(if is_open { "▲" } else { "▼" })
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
                let mut item = div()
                    .id(gpui::SharedString::from(format!("{id_prefix}-{}", b.id)))
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .cursor_pointer()
                    .hover(|s| s.bg(cx.theme().muted))
                    .child(b.display_name.clone())
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let streaming_opts: Vec<_> = all.iter().filter(|b| b.supports_streaming).cloned().collect();
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
            .py_2()
            .text_sm()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            // ── ASR ──
            .when(self.active_tab == SettingsTab::Asr, |this| this
            .child(self.field_label("settings.streaming_backend", cx))
            .child(self.render_dropdown(
                Dropdown::Streaming,
                &cfg.asr.streaming_backend,
                streaming_opts,
                cx,
            ))
            .child(self.field_label("settings.api_key", cx))
            .child(Input::new(&self.stream_api_key).small())
            .child(self.api_key_env_hint(&cfg.asr.streaming_backend, cx))
            .child(self.field_label("settings.endpoint", cx))
            .child(Input::new(&self.stream_endpoint).small())
            .child(
                div().pt_2().child(
                    Button::new("test-stream")
                        .outline()
                        .label(tr("settings.test"))
                        .on_click(cx.listener(|this, _, window, cx| this.on_test(true, window, cx))),
                ),
            )
            // ── ASR：离线 ──
            .child(self.field_label("settings.offline_backend", cx))
            .child(self.render_dropdown(
                Dropdown::Offline,
                &cfg.asr.offline_backend,
                offline_opts,
                cx,
            ))
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
                this.child(self.field_label("settings.api_key", cx))
                    .child(Input::new(&self.off_api_key).small())
                    .child(self.api_key_env_hint(&cfg.asr.offline_backend, cx))
                    .child(self.field_label("settings.endpoint", cx))
                    .child(Input::new(&self.off_endpoint).small())
                    // 大文件后端额外的 OSS 参数
                    .when(off_is_filetrans, |this| {
                        this.child(
                            div()
                                .pt_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("settings.oss_hint")),
                        )
                        .child(self.field_label("settings.oss_endpoint", cx))
                        .child(Input::new(&self.off_oss_endpoint).small())
                        .child(self.field_label("settings.oss_bucket", cx))
                        .child(Input::new(&self.off_oss_bucket).small())
                        .child(self.field_label("settings.oss_ak_id", cx))
                        .child(Input::new(&self.off_oss_ak_id).small())
                        .child(self.field_label("settings.oss_ak_secret", cx))
                        .child(Input::new(&self.off_oss_ak_secret).small())
                    })
            })
            .child(
                div().pt_2().child(
                    Button::new("test-offline")
                        .outline()
                        .label(tr("settings.test"))
                        .on_click(cx.listener(|this, _, window, cx| this.on_test(false, window, cx))),
                ),
            ))
            // ── 录音 ──
            .when(self.active_tab == SettingsTab::Recording, |this| this
            .child(self.labeled("settings.default_mode", self.mode_choice(mode, cx)))
            .child(self.labeled(
                "settings.auto_copy",
                Switch::new("auto-copy")
                    .checked(cfg.text.auto_copy)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        let v = *checked;
                        this.update_config(cx, |c| c.text.auto_copy = v);
                        cx.notify();
                    })),
            ))
            .child(self.labeled(
                "settings.audio_feedback",
                Switch::new("audio-feedback")
                    .checked(cfg.general.audio_feedback)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        let v = *checked;
                        this.update_config(cx, |c| c.general.audio_feedback = v);
                        cx.notify();
                    })),
            ))
            .child(self.labeled(
                "settings.max_seconds",
                div().w(px(120.)).child(Input::new(&self.max_secs).small()),
            )))
            // ── 通用 ──
            .when(self.active_tab == SettingsTab::General, |this| this
            .child(self.labeled(
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
            ))
            .child(self.labeled(
                "settings.minimized",
                Switch::new("minimized")
                    .checked(cfg.general.start_minimized)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        let v = *checked;
                        this.update_config(cx, |c| c.general.start_minimized = v);
                        cx.notify();
                    })),
            ))
            .child(self.labeled(
                "settings.on_top",
                Switch::new("on-top")
                    .checked(cfg.general.window_on_top)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        let v = *checked;
                        this.update_config(cx, |c| c.general.window_on_top = v);
                        cx.notify();
                    })),
            ))
            .child(self.labeled("settings.theme", self.theme_choice(&theme, cx)))
            .child(self.labeled("settings.language", self.lang_choice(lang, cx))))
            // ── 快捷键 ──
            .when(self.active_tab == SettingsTab::Shortcuts, |this| this
            .child(self.shortcut_row("shortcut.toggle_recording", &cfg.shortcuts.toggle_recording, cx))
            .child(self.shortcut_row("shortcut.toggle_window", &cfg.shortcuts.toggle_window, cx))
            .child(self.shortcut_row("shortcut.copy_paste", &cfg.shortcuts.copy_and_paste, cx))
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("settings.shortcuts_hint")),
            ))
            // ── 数据 ──
            .when(self.active_tab == SettingsTab::Data, |this| this
            .child(self.labeled(
                "settings.save_audio",
                Switch::new("save-audio")
                    .checked(cfg.storage.save_audio)
                    .on_click(cx.listener(|this, checked: &bool, _w, cx| {
                        let v = *checked;
                        this.update_config(cx, |c| c.storage.save_audio = v);
                        cx.notify();
                    })),
            ))
            .child(self.field_label("settings.audio_dir", cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.audio_dir).small()))
                    .child(
                        Button::new("audio-browse")
                            .outline()
                            .label(tr("settings.browse"))
                            .on_click(cx.listener(Self::on_browse_audio_dir)),
                    )
                    .child(
                        Button::new("audio-open")
                            .outline()
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
            .child(self.labeled(
                "settings.text_retention",
                div().w(px(120.)).child(Input::new(&self.text_retention).small()),
            ))
            .child(self.labeled(
                "settings.audio_retention",
                div().w(px(120.)).child(Input::new(&self.audio_retention).small()),
            ))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(div().text_sm().child(format!(
                        "{} {}",
                        tr("settings.audio_usage"),
                        human_size(self.audio_usage_bytes)
                    )))
                    .child(
                        Button::new("audio-clean")
                            .outline()
                            .label(tr("settings.clean_audio"))
                            .on_click(cx.listener(Self::on_clean_audio)),
                    ),
            )
            .child(
                div().pt_3().child(
                    Button::new("export-history")
                        .outline()
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
            ))
            // ── 关于 ──
            .when(self.active_tab == SettingsTab::About, |this| this
            .child(self.about_row("about.version", crate::diagnostics::VERSION, cx))
            .child(self.about_row("about.build", &crate::diagnostics::build_time_display(), cx))
            .child(self.about_row("about.commit", crate::diagnostics::GIT_HASH, cx))
            .child(
                div().pt_2().child(
                    Button::new("export-diag")
                        .outline()
                        .label(tr("about.export_diag"))
                        .on_click(cx.listener(Self::on_export_diag)),
                ),
            ));

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
                    .w(px(600.))
                    .h(px(560.))
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(12.))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .w_full()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(tr("settings.title")),
                            )
                            .child(
                                Button::new("settings-close")
                                    .ghost()
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
                            .child(self.render_tab_rail(cx))
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
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
    fn mode_choice(&self, mode: TranscriptionMode, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let is_streaming = mode == TranscriptionMode::Streaming;
        h_flex()
            .gap_2()
            .child(
                Button::new("mode-s")
                    .when(is_streaming, |b| b.primary())
                    .when(!is_streaming, |b| b.outline())
                    .label(tr("mode.streaming"))
                    .on_click(cx.listener(|this, _, _w, cx| {
                        this.update_config(cx, |c| c.asr.default_mode = TranscriptionMode::Streaming);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("mode-o")
                    .when(!is_streaming, |b| b.primary())
                    .when(is_streaming, |b| b.outline())
                    .label(tr("mode.offline"))
                    .on_click(cx.listener(|this, _, _w, cx| {
                        this.update_config(cx, |c| c.asr.default_mode = TranscriptionMode::Offline);
                        cx.notify();
                    })),
            )
    }

    fn theme_choice(&self, theme: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let cur = theme.to_string();
        let btn = |id: &'static str, key: &'static str, val: &'static str, cur: &str, cx: &mut Context<Self>| {
            let active = cur == val;
            Button::new(id)
                .when(active, |b| b.primary())
                .when(!active, |b| b.outline())
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
                    .label("中文")
                    .on_click(cx.listener(|this, _, window, cx| this.set_language("zh-CN", window, cx))),
            )
            .child(
                Button::new("lang-en")
                    .when(!zh, |b| b.primary())
                    .when(zh, |b| b.outline())
                    .label("English")
                    .on_click(cx.listener(|this, _, window, cx| this.set_language("en", window, cx))),
            )
    }

    fn shortcut_row(&self, label_key: &str, binding: &str, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_1()
            .child(div().text_sm().child(tr(label_key)))
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.))
                    .bg(cx.theme().muted)
                    .text_xs()
                    .child(binding.to_string()),
            )
    }

    fn about_row(&self, label_key: &str, value: &str, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_0p5()
            .text_sm()
            .child(div().text_color(cx.theme().muted_foreground).child(tr(label_key)))
            .child(div().child(value.to_string()))
    }
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
