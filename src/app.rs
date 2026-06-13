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

use crate::asr::traits::StreamingResult;
use crate::asr::{AsrConfig, AsrError, BackendRegistry};
use crate::audio::{AudioError, Recorder, StreamingCapture};
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
    /// 当前离线录音会话（None 表示未在录音）。
    recorder: Option<Recorder>,
    /// 当前实时流式会话（None 表示未在流式录音）。
    streaming: Option<StreamingSession>,
    /// 实时识别失败后是否已切换到"停止后离线转写"。
    streaming_fallback: bool,
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
            streaming: None,
            streaming_fallback: false,
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

    // ───────────────────────────── 实时流式（M6）─────────────────────────────

    /// 开始实时流式识别：流式采集 + WS 后端 + 增量结果回 UI。
    fn start_streaming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.build_asr_config(cx);
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
        self.state.recording_state = RecordingState::Recording;
        self.state.recording_duration_secs = 0;
        self.state.pending_text.clear();
        cx.notify();
        window.push_notification("实时识别中…", cx);

        // 按配置 backend_id 选流式后端（默认百炼实时）；只依赖 trait + 注册表。
        let streaming_backend_id = resolve_backend_id(&config, true, None);
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
                self.state.recording_state = RecordingState::Processing;
                self.state.recording_duration_secs = 0;
                cx.notify();
                if self.streaming_fallback {
                    window.push_notification("实时识别失败，正在离线转写…", cx);
                    self.start_transcription(window, cx, outcome.path);
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
            // 整句稳定：固化到编辑器，清空 pending。
            append_text(&self.editor, &result.delta_text, window, cx);
            self.state.pending_text.clear();
        } else {
            // 未稳定：替换 pending（DashScope 发整句而非增量）。
            self.state.pending_text = result.delta_text;
        }
        cx.notify();
    }

    /// 后端结束处理：成功转 Idle；鉴权失败提示；其它失败标记回退离线。
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
            RecordingState::Idle => self.start_capture(window, cx),
            RecordingState::Recording => self.stop_capture(window, cx, false),
            // Processing 不可点击/不可切换，理论上不会到这里。
            RecordingState::Processing => {}
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

    fn on_open_settings(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        // M7 临时：把「设置」按钮用作"测试连接"（正式设置面板在 M11，§6.4）。
        let config = self.build_asr_config(cx);
        let want_streaming = self.state.transcription_mode == TranscriptionMode::Streaming;
        let backend_id = resolve_backend_id(&config, want_streaming, None);
        let Some(handle) = cx.try_global::<GlobalTokioHandle>().map(|g| g.0.clone()) else {
            return;
        };
        window.push_notification(format!("正在测试连接（{backend_id}）…"), cx);

        cx.spawn_in(window, async move |this, cx| {
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
            let _ = this.update_in(cx, |_this, window, cx| match outcome {
                Ok(Ok(())) => window.push_notification("连接测试成功 ✓", cx),
                Ok(Err(e)) => {
                    window.push_notification(format!("连接测试失败：{}", friendly_asr_error(&e)), cx)
                }
                Err(_) => window.push_notification("连接测试中断", cx),
            });
        })
        .detach();
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

/// 读取 WAV → 按配置 backend_id + 大小选离线后端 → 转写（在 Tokio 运行时执行）。
async fn run_offline_transcription(
    config: AsrConfig,
    wav_path: PathBuf,
) -> Result<String, AsrError> {
    let audio = tokio::fs::read(&wav_path).await?;
    let backend_id = resolve_backend_id(&config, false, Some(audio.len()));
    tracing::info!(%backend_id, bytes = audio.len(), "选择离线转写后端");

    let registry = BackendRegistry::with_builtins();
    let backend = registry
        .get(&backend_id)
        .ok_or_else(|| AsrError::InvalidConfig(format!("未找到离线后端: {backend_id}")))?;
    backend.transcribe_offline(&config, audio).await
}

/// 按配置 `backend_id` 与能力（流式/离线）+ 音频大小，解析出实际使用的后端 id。
/// 仅依赖注册表枚举的能力，新增后端无需改此处（开闭原则，§7.3）。
fn resolve_backend_id(config: &AsrConfig, want_streaming: bool, audio_len: Option<usize>) -> String {
    let registry = BackendRegistry::with_builtins();
    let supports = |id: &str| {
        registry
            .get(id)
            .map(|b| {
                if want_streaming {
                    b.supports_streaming()
                } else {
                    b.supports_offline()
                }
            })
            .unwrap_or(false)
    };

    let configured = config.backend_id.trim();
    if !configured.is_empty() && supports(configured) {
        // 百炼离线同步后端的大文件特例：透明改用 filetrans。
        if !want_streaming
            && configured == "aliyun_bailian_offline"
            && audio_len.is_some_and(|len| len > SYNC_OFFLINE_MAX_BYTES)
        {
            return "aliyun_bailian_filetrans".to_string();
        }
        return configured.to_string();
    }

    // 配置无效/不支持该模式 → 合理默认。
    if want_streaming {
        "aliyun_bailian_streaming".to_string()
    } else if audio_len.is_some_and(|len| len > SYNC_OFFLINE_MAX_BYTES) {
        "aliyun_bailian_filetrans".to_string()
    } else {
        "aliyun_bailian_offline".to_string()
    }
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
