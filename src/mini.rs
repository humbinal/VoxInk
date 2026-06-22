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

/// 迷你条窗口尺寸（px）。宽度运行期随内容自适应（见 [`desired_width`]），此为初始值。
pub const MINI_W: f32 = 280.0;
pub const MINI_H: f32 = 36.0;
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
                // 拖拽区：录制状态图标 + 模式 + 波形 + 时长 + 字数。flex_1 吸收多余宽度——
                // 多出的间距留在字数之后，↗/✕ 始终靠右对齐。标 Drag 即整片 HTCAPTION；
                // 按钮须为其**兄弟**（CLAUDE.md）。
                h_flex()
                    .flex_1()
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
            .child(Icon::new(IconName::Close).size(px(16.)))
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

/// 估算迷你条贴合内容所需的逻辑宽度（用于运行期自适应 resize）。
///
/// 固定元素（按钮/麦克/波形/间距/内边距）+ 文本估宽（模式/时长/字数随内容变化）。
/// 文本按字符估宽（CJK 比 ASCII 宽）；略偏大以免裁切。
pub fn desired_width(snap: &MiniSnapshot) -> f32 {
    let mode = match snap.mode {
        TranscriptionMode::Streaming => tr("mode.streaming"),
        TranscriptionMode::Offline => tr("mode.offline"),
    };
    let duration = format!("{:02}:{:02}", snap.duration_secs / 60, snap.duration_secs % 60);
    let chars_label = format!("{} {}", snap.chars, tr("mini.chars_suffix"));

    // 文本估宽：ASCII / CJK 不同字宽（含字号差异——时长用 text_sm，余用 text_xs）。
    let text_w = |s: &str, ascii: f32, cjk: f32| -> f32 {
        s.chars()
            .map(|c| if (c as u32) > 0x2E80 { cjk } else { ascii })
            .sum()
    };
    let mode_w = text_w(&mode, 7.0, 13.0);
    let dur_w = text_w(&duration, 8.5, 14.0);
    let chars_w = text_w(&chars_label, 7.0, 13.0);
    let wave_w = MINI_WAVE_BARS as f32 * 2.0 + (MINI_WAVE_BARS as f32 - 1.0);

    // 固定：左右 px_2(16) + 开关/麦克/打开主窗/隐藏(各 20/15/20/20) + 7 个 gap_2(8)。
    let fixed = 16.0 + 20.0 + 15.0 + 20.0 + 20.0 + 7.0 * 8.0 + wave_w;
    // 略偏大（+10）：宁可字数后留一点空隙，也不要把内容挤裁。
    (fixed + mode_w + dur_w + chars_w + 10.0).clamp(180.0, 560.0)
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
        // 单行状态条：高度固定、宽度由 tick_mini 程序化自适应。禁用用户拖边缩放
        // （否则拖高留空白、拖宽与自适应对打、拖矮裁切内容）；不影响程序内 resize。
        is_resizable: false,
        ..Default::default()
    }
}

/// 显示并置顶：有保存位置则用之，否则默认右上角（距顶 ~100px、距右 ~40px）。
pub fn show_at(window: &Window, saved: Option<(i32, i32)>) {
    #[cfg(windows)]
    if let Some(h) = window_hwnd(window) {
        winimpl::show_at(h, saved);
    }
    #[cfg(not(windows))]
    let _ = (window, saved);
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

/// 读取迷你窗当前屏幕位置（物理像素左上角），用于位置持久化。
pub fn window_pos(window: &Window) -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        window_hwnd(window).and_then(winimpl::window_pos)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
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
    use std::cell::Cell;
    use std::ffi::c_void;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, GetWindowRect, HTCAPTION, HTTOP, HTTOPLEFT,
        HTTOPRIGHT, HWND_TOPMOST, SPI_GETWORKAREA, SW_HIDE, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowLongPtrW, SetWindowPos,
        ShowWindow, SystemParametersInfoW, WM_NCHITTEST, WNDPROC,
    };

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut c_void)
    }

    thread_local! {
        /// 迷你窗子类化状态：`(hwnd, 原 WNDPROC 指针)`。单一迷你窗，仅安装一次。
        /// 全程在 gpui 主线程（win32 消息泵）上访问，thread_local 即够。
        static SUBCLASS: Cell<(isize, isize)> = const { Cell::new((0, 0)) };
    }

    /// 子类化 WNDPROC：把 gpui 为无边框窗顶边合成的缩放命中（HTTOP/左上/右上）改写为
    /// HTCAPTION——顶边由「缩放」变为「移动」。gpui 的 `handle_hit_test_msg` 只认 is_movable、
    /// 忽略 is_resizable，且只给顶边合成缩放，故无法用 WindowOptions 关掉，只能这样拦截。
    unsafe extern "system" fn subclass_proc(
        hh: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let (stored, orig_ptr) = SUBCLASS.with(|c| c.get());
        let orig: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(orig_ptr) };
        // 未安装 / 非本窗：交回系统默认，避免野指针调用。
        if orig.is_none() || stored != hh.0 as isize {
            return unsafe { DefWindowProcW(hh, msg, wparam, lparam) };
        }
        if msg == WM_NCHITTEST {
            let res = unsafe { CallWindowProcW(orig, hh, msg, wparam, lparam) };
            let code = res.0 as u32;
            if code == HTTOP || code == HTTOPLEFT || code == HTTOPRIGHT {
                return LRESULT(HTCAPTION as isize);
            }
            return res;
        }
        unsafe { CallWindowProcW(orig, hh, msg, wparam, lparam) }
    }

    /// 给迷你窗安装上述子类化（幂等：同一 HWND 只装一次）。
    fn install_no_user_resize(h: isize) {
        let (stored, orig) = SUBCLASS.with(|c| c.get());
        if stored == h && orig != 0 {
            return;
        }
        let proc_ptr = subclass_proc as *const () as isize;
        let prev = unsafe { SetWindowLongPtrW(hwnd(h), GWLP_WNDPROC, proc_ptr) };
        SUBCLASS.with(|c| c.set((h, prev)));
    }

    /// 当前窗口物理尺寸宽度（用于默认右上角定位）。
    fn window_width(h: isize) -> i32 {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(hwnd(h), &mut r) }.is_ok() {
            r.right - r.left
        } else {
            280
        }
    }

    pub fn window_pos(h: isize) -> Option<(i32, i32)> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(hwnd(h), &mut r) }.ok()?;
        Some((r.left, r.top))
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

    /// 显示并置顶到给定/默认位置（仅移动不改尺寸——宽度由 gpui 逻辑 resize 管理）。
    pub fn show_at(h: isize, saved: Option<(i32, i32)>) {
        // 关掉 gpui 无边框窗顶边的用户缩放（is_resizable 对其无效，见 subclass_proc）。
        install_no_user_resize(h);
        let (x, y) = saved.unwrap_or_else(|| {
            let wa = work_area();
            (wa.right - window_width(h) - 40, wa.top + 100)
        });
        unsafe {
            let _ = SetWindowPos(
                hwnd(h),
                Some(HWND_TOPMOST),
                x,
                y,
                0,
                0,
                SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOSIZE,
            );
        }
    }

    pub fn hide(h: isize) {
        unsafe {
            let _ = ShowWindow(hwnd(h), SW_HIDE);
        }
    }
}
