//! 主界面 View —— M1 任务 1.5 / §6.2 主界面布局。
//!
//! 布局自上而下：Header / 控制区（录音按钮 + 模式 Toggle + 状态）/ 文本编辑区 / Footer。
//! M1 阶段按钮点击仅打印日志（任务 1.6），状态机与剪贴板反馈在 M2 落地。

use gpui::{
    ClipboardItem, Context, Entity, Focusable, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px, rgb,
};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};

use crate::state::{AppState, RecordingState, TranscriptionMode};

/// VoxInk 主窗口视图。
pub struct VoxInk {
    /// 应用全局状态（§2.1）。
    state: AppState,
    /// 文本编辑器状态（gpui-component 多行输入）。
    editor: Entity<InputState>,
}

impl VoxInk {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("点击「开始录音」用语音输入提示词，或直接在此编辑……")
        });

        // 启动时聚焦编辑器，便于直接键盘输入。
        let focus_handle = editor.focus_handle(cx);
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });

        Self {
            state: AppState::default(),
            editor,
        }
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

    fn on_toggle_recording(&mut self, _: &gpui::ClickEvent, _: &mut Window, _: &mut Context<Self>) {
        // M1：占位逻辑，仅打印日志；状态机在 M2 实现。
        tracing::info!(state = ?self.state.recording_state, "录音按钮被点击");
    }

    fn on_select_mode(&mut self, mode: TranscriptionMode, cx: &mut Context<Self>) {
        self.state.transcription_mode = mode;
        tracing::info!(?mode, "切换转录模式");
        cx.notify();
    }

    fn on_copy(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        tracing::info!(chars = text.chars().count(), "复制按钮被点击");
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn on_open_settings(&mut self, _: &gpui::ClickEvent, _: &mut Window, _: &mut Context<Self>) {
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

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (status_text, status_color) = self.status();
        let is_streaming = self.state.transcription_mode == TranscriptionMode::Streaming;

        let record_label = match self.state.recording_state {
            RecordingState::Idle => "🎤 开始录音",
            RecordingState::Recording => "⏹ 停止录音",
            RecordingState::Processing => "⏳ 处理中…",
        };

        v_flex()
            .w_full()
            .gap_3()
            .px_4()
            .py_4()
            .items_center()
            // 录音按钮（主操作区）
            .child(
                Button::new("record")
                    .primary()
                    .large()
                    .w_full()
                    .label(record_label)
                    .on_click(cx.listener(Self::on_toggle_recording)),
            )
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
        div()
            .flex_1()
            .w_full()
            .px_4()
            .py_2()
            .child(
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
                    .label("📋 一键复制")
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
