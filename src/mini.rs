//! 迷你状态条窗口（IDEAS：极致交互——置顶单行小组件，MVP）。
//!
//! 一个无边框、不进任务栏、置顶的小窗，主窗口隐藏到托盘时仍可见，
//! 显示录制状态/时长/转写字数 + 一个开始/停止按钮。
//!
//! 架构要点：
//! - 独立 GPUI 窗口（`WindowKind::PopUp` → Windows 后端给到 `WS_EX_TOOLWINDOW`（无任务栏）
//!   + 无边框；但 **不自动置顶**，故创建后用 Win32 `SetWindowPos(HWND_TOPMOST)` 补上）。
//! - 状态同步用 `cx.observe(主视图)`：主视图每次 `notify` 即触发本窗重渲染，从
//!   [`VoxInk::mini_snapshot`] 现取状态——单向、零手工推送，避开双重借用。
//! - 开始/停止经**主窗口** `AnyWindowHandle`（非本窗）路由到 `toggle_recording`，
//!   使其 toast/Root 层落在主窗口（否则通知静默失效，见 CLAUDE.md）。

use gpui::{
    AnyWindowHandle, Bounds, Context, Entity, Window, WindowBounds, WindowControlArea,
    WindowKind, WindowOptions, div, point, prelude::*, px, size, white,
};
use gpui_component::{ActiveTheme, Icon, h_flex};

use crate::app::VoxInk;
use crate::i18n::tr;
use crate::state::RecordingState;
use crate::theme::{STATUS_IDLE, STATUS_PROCESSING, STATUS_RECORDING};

/// 迷你条窗口尺寸（px）。
pub const MINI_W: f32 = 300.0;
pub const MINI_H: f32 = 40.0;

/// 迷你状态条视图：镜像主视图状态（经 observe 拉取），并把开始/停止路由回主窗口。
pub struct MiniBar {
    /// 主视图（取状态 + 路由录制开关）。
    voxink: Entity<VoxInk>,
    /// 主窗口句柄：开始/停止须落在主窗口上下文（toast/Root 层）。
    main_window: AnyWindowHandle,
}

impl MiniBar {
    pub fn new(voxink: Entity<VoxInk>, main_window: AnyWindowHandle, cx: &mut Context<Self>) -> Self {
        // 主视图每次 notify → 本窗重渲染（拉取最新状态）。
        cx.observe(&voxink, |_, _, cx| cx.notify()).detach();
        Self {
            voxink,
            main_window,
        }
    }
}

impl Render for MiniBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (state, duration_secs, chars) = self.voxink.read(cx).mini_snapshot(cx);
        let dot = match state {
            RecordingState::Idle => STATUS_IDLE,
            RecordingState::Recording => STATUS_RECORDING,
            RecordingState::Processing => STATUS_PROCESSING,
        };
        let duration = format!("{:02}:{:02}", duration_secs / 60, duration_secs % 60);
        let chars_label = format!("{chars} {}", tr("mini.chars_suffix"));

        // 开始/停止按钮的字形与可点性（处理中不可点）。
        let recording = state == RecordingState::Recording;
        let clickable = state != RecordingState::Processing;

        h_flex()
            .size_full()
            .items_center()
            .justify_between()
            .px_2()
            .gap_2()
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(8.))
            .text_color(cx.theme().foreground)
            .child(
                // 拖拽区（状态/时长/字数）：标 Drag 即整片 HTCAPTION，可拖动整窗。
                // 按钮必须是其**兄弟**而非子元素，否则点击被系统当拖窗吞掉（CLAUDE.md）。
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .child(div().flex_shrink_0().size(px(8.)).rounded_full().bg(dot))
                    .child(div().text_sm().child(duration))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(chars_label),
                    ),
            )
            .child(self.toggle_button(recording, clickable, cx))
    }
}

impl MiniBar {
    /// 自绘开始/停止按钮（录音中=红底方块；否则=主色麦克风）。处理中变灰不可点。
    fn toggle_button(
        &self,
        recording: bool,
        clickable: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let bg = if recording {
            STATUS_RECORDING
        } else {
            cx.theme().primary
        };
        let glyph = if recording {
            // 停止：圆角白方块。
            div()
                .size(px(10.))
                .rounded(px(2.))
                .bg(white())
                .into_any_element()
        } else {
            Icon::empty()
                .path("icons/mic.svg")
                .size(px(14.))
                .text_color(white())
                .into_any_element()
        };

        let mut btn = h_flex()
            .id("mini-toggle")
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(px(26.))
            .rounded(px(6.))
            .bg(bg)
            .child(glyph);

        if clickable {
            btn = btn.cursor_pointer().hover(|s| s.opacity(0.9)).on_click(
                cx.listener(|mini, _, _window, cx| {
                    // 经主窗口路由 toggle_recording；用 spawn 推迟到本窗事件分发之外，
                    // 避免在迷你窗 update 内嵌套更新主窗口。
                    let main = mini.main_window;
                    let view = mini.voxink.clone();
                    cx.spawn(async move |_, cx| {
                        let _ = main.update(cx, |_, win, app| {
                            view.update(app, |v, vcx| v.toggle_recording(win, vcx));
                        });
                    })
                    .detach();
                }),
            );
        } else {
            btn = btn.opacity(0.6);
        }
        btn
    }
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

/// 创建后调用：置顶 + 移到主屏工作区底部居中。
pub fn place_topmost(window: &Window) {
    #[cfg(windows)]
    if let Some(h) = window_hwnd(window) {
        winimpl::place_topmost(h, MINI_W as i32, MINI_H as i32);
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// 切换迷你窗显隐（隐藏后再显示会重新置顶）。
pub fn toggle_visibility(window: &Window) {
    #[cfg(windows)]
    if let Some(h) = window_hwnd(window) {
        winimpl::toggle(h, MINI_W as i32, MINI_H as i32);
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
        HWND_TOPMOST, IsWindowVisible, SPI_GETWORKAREA, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowPos, ShowWindow, SystemParametersInfoW,
    };

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut c_void)
    }

    /// 主屏工作区（排除任务栏）矩形；失败回退一个保守默认。
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

    /// 置顶并移到工作区底部居中（留 24px 下边距）。
    pub fn place_topmost(h: isize, w: i32, height: i32) {
        let wa = work_area();
        let x = wa.left + ((wa.right - wa.left) - w) / 2;
        let y = wa.bottom - height - 24;
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

    pub fn toggle(h: isize, w: i32, height: i32) {
        unsafe {
            if IsWindowVisible(hwnd(h)).as_bool() {
                let _ = ShowWindow(hwnd(h), SW_HIDE);
            } else {
                // 重新显示时再置顶一次（拓扑可能已变）。
                place_topmost(h, w, height);
            }
        }
    }
}
