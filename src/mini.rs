//! 迷你状态条窗口（IDEAS：极致交互——置顶单行小组件）。
//!
//! 一个无边框、不进任务栏、置顶的小窗，与主窗口**互斥**显示（显示其一即隐藏另一），
//! 默认停靠屏幕右上角。单行从左到右：录制状态图标 / 转录模式 / 窄波形 / 时长 / 字数 /
//! 「打开主窗口」按钮 / 开始·停止（▶·⏸）按钮。
//!
//! 架构要点：
//! - 独立 GPUI 窗口（`WindowKind::PopUp` → Windows 后端给 `WS_EX_TOOLWINDOW`（无任务栏）
//!   + 无边框；不自动置顶，故创建后 Win32 `SetWindowPos(HWND_TOPMOST)` 补上）。
//! - 状态同步用 `cx.observe(主视图)`：主视图每次 `notify` 即触发本窗重渲染，从
//!   [`VoxInk::mini_snapshot`] 现取状态——单向、零手工推送，避开双重借用。
//! - 按钮经**主窗口** `AnyWindowHandle`（非本窗）路由到主视图方法，使 toast/Root 层
//!   落在主窗口（否则通知静默失效，见 CLAUDE.md）。

use gpui::{
    Animation, AnimationExt, AnyWindowHandle, Bounds, Context, Entity, Hsla, IntoElement, Window,
    WindowBounds, WindowControlArea, WindowKind, WindowOptions, div, ease_in_out, point, prelude::*,
    px, size,
};
use gpui_component::{ActiveTheme, Icon, IconName, h_flex};

use crate::app::VoxInk;
use crate::i18n::tr;
use crate::state::{RecordingState, TranscriptionMode};
use crate::theme::{BRAND, STATUS_PROCESSING, STATUS_RECORDING};

/// 迷你条窗口尺寸（px）。
pub const MINI_W: f32 = 360.0;
pub const MINI_H: f32 = 40.0;
/// 窄波形竖条数量（也即 [`VoxInk::mini_snapshot`] 回传的电平尾巴长度）。
pub const MINI_WAVE_BARS: usize = 16;
/// 波形竖条最大/最小高度（px），贴合单行高度。
const WAVE_MAX: f32 = 16.0;
const WAVE_MIN: f32 = 2.0;

/// 主视图回传给迷你条的状态快照。
pub struct MiniSnapshot {
    pub state: RecordingState,
    pub mode: TranscriptionMode,
    pub duration_secs: u32,
    pub chars: usize,
    /// 最近 [`MINI_WAVE_BARS`] 个电平（0..1），用于绘制窄波形。
    pub levels: Vec<f32>,
}

/// 迷你状态条视图：镜像主视图状态（经 observe 拉取），按钮路由回主窗口。
pub struct MiniBar {
    voxink: Entity<VoxInk>,
    main_window: AnyWindowHandle,
}

impl MiniBar {
    pub fn new(voxink: Entity<VoxInk>, main_window: AnyWindowHandle, cx: &mut Context<Self>) -> Self {
        cx.observe(&voxink, |_, _, cx| cx.notify()).detach();
        Self {
            voxink,
            main_window,
        }
    }
}

impl Render for MiniBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = self.voxink.read(cx).mini_snapshot(cx);
        let recording = snap.state == RecordingState::Recording;
        let clickable = snap.state != RecordingState::Processing;

        let mode_text = match snap.mode {
            TranscriptionMode::Streaming => tr("mode.streaming"),
            TranscriptionMode::Offline => tr("mode.offline"),
        };
        let duration = format!("{:02}:{:02}", snap.duration_secs / 60, snap.duration_secs % 60);
        let chars_label = format!("{} {}", snap.chars, tr("mini.chars_suffix"));
        let wave_color = if recording {
            STATUS_RECORDING
        } else {
            cx.theme().muted_foreground
        };

        h_flex()
            .size_full()
            .items_center()
            .gap_2()
            .px_2()
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(8.))
            .text_color(cx.theme().foreground)
            // 开始/停止按钮置于最前。
            .child(self.toggle_button(recording, clickable, cx))
            .child(
                // 拖拽区：录制状态图标 + 模式 + 波形 + 时长 + 字数。
                // 标 Drag 即整片 HTCAPTION；按钮须为其**兄弟**（CLAUDE.md）。
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .child(record_indicator(snap.state, cx))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(BRAND)
                            .child(mode_text),
                    )
                    .child(waveform(&snap.levels, wave_color))
                    .child(div().flex_shrink_0().text_sm().child(duration))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(chars_label),
                    ),
            )
            // 末尾：打开主窗口 + 隐藏迷你条。
            .child(self.show_main_button(cx))
            .child(self.hide_button(cx))
    }
}

impl MiniBar {
    /// 「打开主窗口」按钮：显示主窗并隐藏迷你条（互斥）。
    fn show_main_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("mini-show-main")
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_color(cx.theme().muted_foreground)
            .hover(|s| s.bg(cx.theme().list_hover).text_color(cx.theme().foreground))
            .child(Icon::new(IconName::Maximize).size(px(13.)))
            .on_click(cx.listener(|mini, _, _window, cx| {
                let main = mini.main_window;
                let view = mini.voxink.clone();
                cx.spawn(async move |_, cx| {
                    let _ = main.update(cx, |_, win, app| {
                        view.update(app, |v, vcx| v.show_main_window(win, vcx));
                    });
                })
                .detach();
            }))
    }

    /// 「隐藏迷你条」按钮：风格同「打开主窗口」按钮，收起迷你条（回托盘）。
    fn hide_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("mini-hide")
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_color(cx.theme().muted_foreground)
            .hover(|s| s.bg(cx.theme().list_hover).text_color(cx.theme().foreground))
            .child(Icon::new(IconName::Minus).size(px(13.)))
            .on_click(cx.listener(|mini, _, _window, cx| {
                let main = mini.main_window;
                let view = mini.voxink.clone();
                cx.spawn(async move |_, cx| {
                    let _ = main.update(cx, |_, _win, app| {
                        view.update(app, |v, vcx| v.hide_mini_bar(vcx));
                    });
                })
                .detach();
            }))
    }

    /// 开始/停止录音按钮：与文字同高的小按钮，▶=开始、⏸=停止（处理中变灰不可点）。
    fn toggle_button(
        &self,
        recording: bool,
        clickable: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (icon, color) = if recording {
            (IconName::Pause, STATUS_RECORDING)
        } else {
            (IconName::Play, BRAND)
        };
        let mut btn = div()
            .id("mini-toggle")
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .size(px(20.))
            .rounded(px(5.))
            .text_color(color)
            .child(Icon::new(icon).size(px(14.)).text_color(color));

        if clickable {
            btn = btn
                .cursor_pointer()
                .hover(|s| s.bg(cx.theme().list_hover))
                .on_click(cx.listener(|mini, _, _window, cx| {
                    let main = mini.main_window;
                    let view = mini.voxink.clone();
                    cx.spawn(async move |_, cx| {
                        let _ = main.update(cx, |_, win, app| {
                            view.update(app, |v, vcx| v.toggle_recording(win, vcx));
                        });
                    })
                    .detach();
                }));
        } else {
            btn = btn.opacity(0.5);
        }
        btn
    }
}

/// 迷你条最前面的录制状态图标：麦克风随状态变色；录制中红色 + 呼吸脉冲。
fn record_indicator(state: RecordingState, cx: &Context<MiniBar>) -> impl IntoElement {
    let (color, recording) = match state {
        RecordingState::Idle => (cx.theme().muted_foreground, false),
        RecordingState::Recording => (STATUS_RECORDING, true),
        RecordingState::Processing => (STATUS_PROCESSING, false),
    };
    let icon = Icon::empty()
        .path("icons/mic.svg")
        .size(px(15.))
        .text_color(color);
    let base = div().flex_shrink_0().flex().items_center().child(icon);
    if recording {
        base.with_animation(
            "mini-rec-pulse",
            Animation::new(std::time::Duration::from_millis(1200))
                .repeat()
                .with_easing(ease_in_out),
            |this, delta| {
                let t = 1.0 - (2.0 * delta - 1.0).abs();
                this.opacity(0.5 + 0.5 * t)
            },
        )
        .into_any_element()
    } else {
        base.into_any_element()
    }
}

/// 窄波形：固定 [`MINI_WAVE_BARS`] 根竖条；无电平（空闲）时显示一排细基线。
fn waveform(levels: &[f32], color: Hsla) -> impl IntoElement {
    let mut row = h_flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(1.))
        .h(px(WAVE_MAX));
    for i in 0..MINI_WAVE_BARS {
        let l = levels.get(i).copied().unwrap_or(0.0);
        row = row.child(div().w(px(2.)).h(px(bar_height(l))).rounded_full().bg(color));
    }
    row
}

/// 电平（0..1 峰值幅度）→ 竖条高度，dBFS 对数刻度（与主波形一致的感知）。
fn bar_height(level: f32) -> f32 {
    const MIN_DB: f32 = -42.0;
    const GAMMA: f32 = 0.85;
    let norm = if level <= 1e-5 {
        0.0
    } else {
        let lin = ((20.0 * level.log10() - MIN_DB) / -MIN_DB).clamp(0.0, 1.0);
        lin.powf(GAMMA)
    };
    WAVE_MIN + norm * (WAVE_MAX - WAVE_MIN)
}

/// 迷你窗口的创建参数：无边框（PopUp 给 `WINDOW_STYLE(0)`）+ 不进任务栏 + 不抢焦点。
pub fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(120.), px(120.)),
            size: size(px(MINI_W), px(MINI_H)),
        })),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: true,
        ..Default::default()
    }
}

/// 显示并置顶：移到主屏工作区右上角（距顶 ~100px、距右 ~40px）。
pub fn show_topmost(window: &Window) {
    #[cfg(windows)]
    if let Some(h) = window_hwnd(window) {
        winimpl::show_topmost(h, MINI_W as i32, MINI_H as i32);
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// 隐藏迷你窗。
pub fn hide(window: &Window) {
    #[cfg(windows)]
    if let Some(h) = window_hwnd(window) {
        winimpl::hide(h);
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(windows)]
fn window_hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(windows)]
mod winimpl {
    use std::ffi::c_void;

    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SPI_GETWORKAREA, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowPos, ShowWindow, SystemParametersInfoW,
    };

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut c_void)
    }

    fn work_area() -> RECT {
        let mut rect = RECT::default();
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut rect as *mut RECT as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        if ok.is_err() {
            rect = RECT {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            };
        }
        rect
    }

    /// 置顶并移到工作区右上角（距顶 100px、距右 40px）。
    pub fn show_topmost(h: isize, w: i32, height: i32) {
        let wa = work_area();
        let x = wa.right - w - 40;
        let y = wa.top + 100;
        unsafe {
            let _ = SetWindowPos(
                hwnd(h),
                Some(HWND_TOPMOST),
                x,
                y,
                w,
                height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
        }
    }

    pub fn hide(h: isize) {
        unsafe {
            let _ = ShowWindow(hwnd(h), SW_HIDE);
        }
    }
}
