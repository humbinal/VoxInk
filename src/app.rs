//! 主界面 View —— M1 任务 1.5 / §6.2 主界面布局；M2 任务 2.1/2.2/2.5 交互。
//!
//! - 录音按钮状态机：Idle ↔ Recording（M2 仅 UI 切换，真实录音在 M3）。
//! - 一键复制：arboard 写入系统剪贴板，"✓ 已复制" 1.5s 反馈 + Toast。
//! - 模式切换：同步写入全局配置，退出时持久化。

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, Entity, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div, ease_in_out, prelude::*, px, rgb,
    white,
};
use gpui_component::{
    ActiveTheme, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};

use crate::asr::{AsrConfig, AsrError, BackendRegistry};
use crate::audio::{AudioError, Recorder};
use crate::config::VoxInkConfig;
use crate::state::{AppState, RecordingState, TranscriptionMode};

/// 以全局形式承载持久化配置，便于跨 View 读写、退出时统一保存。
pub struct GlobalConfig(pub VoxInkConfig);

impl gpui::Global for GlobalConfig {}

/// Tokio 运行时句柄，供把网络任务派发到 Tokio 执行（reqwest 需要 reactor）。
pub struct GlobalTokioHandle(pub tokio::runtime::Handle);

impl gpui::Global for GlobalTokioHandle {}

/// VoxInk 主窗口视图。
pub struct VoxInk {
    /// 应用全局状态（§2.1）。
    state: AppState,
    /// 文本编辑器状态（gpui-component 多行输入）。
    editor: Entity<InputState>,
    /// 复制成功后的短暂反馈标记（1.5s 后复位）。
    copied: bool,
    /// 当前录音会话（None 表示未在录音）。
    recorder: Option<Recorder>,
}

impl VoxInk {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("点击「开始录音」用语音输入提示词，或直接在此编辑……")
        });

        // 初始转录模式取自持久化配置（§2.7 default_mode）。
        let mut state = AppState::default();
        if let Some(global) = cx.try_global::<GlobalConfig>() {
            state.transcription_mode = global.0.asr.default_mode;
        }

        // 启动时聚焦编辑器，便于直接键盘输入。
        let focus_handle = editor.focus_handle(cx);
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });

        Self {
            state,
            editor,
            copied: false,
            recorder: None,
        }
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
                self.start_transcription(window, cx, outcome.path);
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
    fn start_transcription(&mut self, window: &mut Window, cx: &mut Context<Self>, wav_path: PathBuf) {
        self.state.recording_state = RecordingState::Processing;
        cx.notify();
        window.push_notification("正在识别…", cx);

        let asr_config = self.build_asr_config(cx);
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

    /// 从持久化配置构造运行期 `AsrConfig`（api_key 为内存中的明文）。
    fn build_asr_config(&self, cx: &Context<Self>) -> AsrConfig {
        match cx.try_global::<GlobalConfig>() {
            Some(global) => {
                // M11 设置面板上线前，无 UI 录入 API Key；若配置为空则回退到环境变量
                // DASHSCOPE_API_KEY，便于在当前阶段验证（明文不落盘，符合隐私优先）。
                let api_key = if global.0.asr.api_key.trim().is_empty() {
                    std::env::var("DASHSCOPE_API_KEY").unwrap_or_default()
                } else {
                    global.0.asr.api_key.clone()
                };
                AsrConfig {
                    backend_id: global.0.asr.backend_id.clone(),
                    api_key,
                    api_endpoint: global.0.asr.api_endpoint.clone(),
                    local_model_path: None,
                    local_model_size: Some(global.0.asr.local_model_size.clone()),
                    language: global.0.asr.language.clone(),
                    // M11 设置面板上线前，OSS 凭证经环境变量提供（大文件 filetrans 用）。
                    oss_endpoint: std::env::var("OSS_ENDPOINT").unwrap_or_default(),
                    oss_bucket: std::env::var("OSS_BUCKET").unwrap_or_default(),
                    oss_access_key_id: std::env::var("OSS_ACCESS_KEY_ID").unwrap_or_default(),
                    oss_access_key_secret: std::env::var("OSS_ACCESS_KEY_SECRET").unwrap_or_default(),
                }
            }
            None => AsrConfig::default(),
        }
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
                            this.stop_recording(window, cx, true);
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
            RecordingState::Idle => ("就绪".into(), rgb(0x27AE60)),
            RecordingState::Recording => ("录音中".into(), rgb(0xE74C3C)),
            RecordingState::Processing => ("识别中".into(), rgb(0xF39C12)),
        }
    }

    /// 录音按钮点击：Idle 开始录音 / Recording 停止录音（§4.1.2）。
    fn on_toggle_recording(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_recording(window, cx);
    }

    /// 切换录音状态（供录音按钮与系统托盘菜单调用）。
    pub fn toggle_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.state.recording_state {
            RecordingState::Idle => self.start_recording(window, cx),
            RecordingState::Recording => self.stop_recording(window, cx, false),
            // Processing 不可点击/不可切换，理论上不会到这里。
            RecordingState::Processing => {}
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

    fn on_open_settings(&mut self, _: &ClickEvent, _: &mut Window, _: &mut Context<Self>) {
        tracing::info!("设置按钮被点击");
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("🎙 VoxInk"),
            )
            .child(
                Button::new("settings")
                    .ghost()
                    .label("⚙ 设置")
                    .on_click(cx.listener(Self::on_open_settings)),
            )
    }

    /// 录音按钮：按状态变色/变字；Recording 时叠加脉冲呼吸动画；Processing 不可点击。
    fn render_record_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let (bg, label, clickable) = match self.state.recording_state {
            RecordingState::Idle => (rgb(0x27AE60), "🎤 开始录音", true),
            RecordingState::Recording => (rgb(0xE74C3C), "⏹ 停止录音", true),
            RecordingState::Processing => (rgb(0xF39C12), "⏳ 处理中…", false),
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
                            .label("实时")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.on_select_mode(TranscriptionMode::Streaming, cx)
                            })),
                    )
                    .child(
                        Button::new("mode-offline")
                            .when(!is_streaming, |b| b.primary())
                            .when(is_streaming, |b| b.outline())
                            .label("离线")
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
            .child(format!("字数: {char_count}"))
            .child(
                Button::new("copy")
                    .primary()
                    .label(if self.copied {
                        "✓ 已复制"
                    } else {
                        "📋 一键复制"
                    })
                    .on_click(cx.listener(Self::on_copy)),
            )
    }
}

impl Render for VoxInk {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_header(cx))
            .child(self.render_controls(cx))
            .child(self.render_editor(cx))
            .child(self.render_footer(cx))
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

/// 同步接口（qwen3-asr-flash）的原始音频上限：base64 后约 10MB，对应原始约 7MB。
/// 超过则改走大文件异步后端（filetrans + OSS）。
const SYNC_OFFLINE_MAX_BYTES: usize = 7 * 1024 * 1024;

/// 读取 WAV → 按大小路由到同步/大文件后端 → 转写（在 Tokio 运行时执行）。
async fn run_offline_transcription(
    config: AsrConfig,
    wav_path: PathBuf,
) -> Result<String, AsrError> {
    let audio = tokio::fs::read(&wav_path).await?;
    let backend_id = if audio.len() <= SYNC_OFFLINE_MAX_BYTES {
        "aliyun_bailian_offline"
    } else {
        "aliyun_bailian_filetrans"
    };
    tracing::info!(backend_id, bytes = audio.len(), "选择离线转写后端");

    let registry = BackendRegistry::with_builtins();
    let backend = registry
        .get(backend_id)
        .ok_or_else(|| AsrError::InvalidConfig(format!("未找到离线后端: {backend_id}")))?;
    backend.transcribe_offline(&config, audio).await
}

/// 将转写文本追加到编辑器末尾（离线模式追加，§4.3.1）。
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

/// 把 `AsrError` 映射为用户友好的中文提示（任务 4.5）。
fn friendly_asr_error(error: &AsrError) -> String {
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
