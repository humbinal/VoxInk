//! 系统托盘集成（M5，任务 5.1/5.2）。
//!
//! - `tray-icon` 创建托盘图标 + 右键菜单；左键单击切换主窗口显隐。
//! - 托盘/菜单事件经全局 channel 投递，用 GPUI 前台定时任务轮询并分发（GPUI 的消息循环
//!   会泵送 tray-icon/muda 隐藏窗口的消息）。
//! - 关闭按钮（X）经 `on_window_should_close` 改为隐藏到托盘，不退出进程。
//!
//! ⚠️ 平台说明（§12.3）：gpui 在 Windows 上不公开 hide/minimize，故隐藏/显示窗口直接调用
//! Win32 `ShowWindow`（经 HWND）。非 Windows 平台暂为降级实现（不隐藏），后续里程碑再补。

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Result, anyhow};
use gpui::{App, Entity, Window, WindowHandle};
use gpui_component::Root;

use crate::app::{GlobalConfig, VoxInk};
use crate::config::ShortcutsConfig;

/// 持有托盘实例使其在应用生命周期内存活（拖放会导致图标消失），并保留与全局快捷键
/// 关联的菜单项句柄，供 [`sync_menu_shortcuts`] 在改键后刷新右侧的快捷键标注。
/// `MenuItem` 内部 Rc 共享，存克隆即可继续操作已挂入菜单的同一项。
pub struct GlobalTray {
    #[allow(dead_code)]
    tray: tray_icon::TrayIcon,
    open: tray_icon::menu::MenuItem,
    record: tray_icon::menu::MenuItem,
    mini: tray_icon::menu::MenuItem,
}

impl gpui::Global for GlobalTray {}

/// 把配置中的快捷键字符串解析为可在菜单右侧展示的 accelerator。纯展示：托盘弹出菜单不安装
/// accel 表（应用未调 `TranslateAccelerator`），故不会真正触发，仅借 `\t` 右对齐显示。
/// 空串或无法解析（如不受支持的键）时返回 None（即不显示标注）。
fn parse_accel(spec: &str) -> Option<tray_icon::menu::accelerator::Accelerator> {
    let s = spec.trim();
    if s.is_empty() {
        return None;
    }
    tray_icon::menu::accelerator::Accelerator::from_str(s).ok()
}

/// 按当前配置刷新托盘菜单项右侧的快捷键标注（设置面板改键后调用即时生效）。
/// 托盘未初始化（如测试）时为 no-op。
pub fn sync_menu_shortcuts(shortcuts: &ShortcutsConfig, cx: &App) {
    if let Some(g) = cx.try_global::<GlobalTray>() {
        let _ = g
            .open
            .set_accelerator(parse_accel(&shortcuts.toggle_window));
        let _ = g
            .record
            .set_accelerator(parse_accel(&shortcuts.toggle_recording));
        let _ = g
            .mini
            .set_accelerator(parse_accel(&shortcuts.toggle_mini_bar));
    }
}

/// 创建托盘图标与菜单，注册关闭拦截，并启动事件轮询。
pub fn setup_tray(window: WindowHandle<Root>, view: Entity<VoxInk>, cx: &mut App) -> Result<()> {
    use tray_icon::TrayIconBuilder;
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

    // 初始 accelerator 直接取当前配置；后续改键由 hotkey::apply_shortcuts → sync_menu_shortcuts 刷新。
    let shortcuts = cx
        .try_global::<GlobalConfig>()
        .map(|g| g.0.shortcuts.clone());
    let accel =
        |pick: fn(&ShortcutsConfig) -> &str| shortcuts.as_ref().and_then(|s| parse_accel(pick(s)));

    let menu = Menu::new();
    let open = MenuItem::with_id("open", "打开主界面", true, accel(|s| &s.toggle_window));
    let record = MenuItem::with_id(
        "record",
        "开始/停止录音",
        true,
        accel(|s| &s.toggle_recording),
    );
    let mini = MenuItem::with_id(
        "mini",
        "显示/隐藏迷你条",
        true,
        accel(|s| &s.toggle_mini_bar),
    );
    let settings = MenuItem::with_id("settings", "设置", true, None);
    let quit = MenuItem::with_id("quit", "退出", true, None);
    let append = |item: &dyn tray_icon::menu::IsMenuItem| -> Result<()> {
        menu.append(item)
            .map_err(|e| anyhow!("添加托盘菜单项失败: {e}"))
    };
    // 分四组（组间以分隔线区隔）：① 打开主界面 + 显示迷你条；② 开始/停止录音；③ 设置；④ 退出。
    append(&open)?;
    append(&mini)?;
    append(&PredefinedMenuItem::separator())?;
    append(&record)?;
    append(&PredefinedMenuItem::separator())?;
    append(&settings)?;
    append(&PredefinedMenuItem::separator())?;
    append(&quit)?;

    let mut builder = TrayIconBuilder::new()
        .with_id("voxink-tray")
        .with_menu(Box::new(menu))
        .with_tooltip("VoxInk")
        // 左键不弹菜单——留给"切换窗口显隐"。
        .with_menu_on_left_click(false);
    if let Some(icon) = crate::branding::tray_icon(crate::branding::IconStatus::Idle) {
        builder = builder.with_icon(icon);
    }
    let tray = builder
        .build()
        .map_err(|e| anyhow!("创建系统托盘失败: {e}"))?;
    cx.set_global(GlobalTray {
        tray,
        open,
        record,
        mini,
    });

    // 关闭按钮（X）→ 隐藏到托盘，取消真正的关闭。
    let _ = window.update(cx, |_, win, ctx| {
        win.on_window_should_close(ctx, |win, _app| {
            hide_to_tray(win);
            false
        });
    });

    // 轮询托盘/菜单事件 + 录制状态图标刷新。
    cx.spawn(async move |cx| {
        use tray_icon::menu::MenuEvent;
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

        // 集中驱动图标刷新：状态变化时更新托盘图标 + 任务栏角标（150ms 延迟无感）。
        let mut last_status = crate::branding::IconStatus::Idle;

        loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;

            cx.update(|app| {
                let status = view.read(app).current_icon_status();
                if status != last_status {
                    last_status = status;
                    set_tray_status(status, app);
                    if let Ok(Some(h)) = window.update(app, |_, win, _| window_hwnd(win)) {
                        set_taskbar_overlay(h, status);
                    }
                }
                // 迷你条可见时：自适应宽度 + 记录位置（持久化）。
                view.update(app, |v, vcx| v.tick_mini(vcx));
                // 主窗口尺寸随用户拖拽变化时记录到配置，退出时统一落盘（位置仍不记录，启动居中）。
                let _ = window.update(app, |_, win, cx| capture_main_size(win, cx));
            });

            // 左键单击：切换「最近一次主动唤起的窗口形态」（主窗口或迷你条，二者互斥）。
            // 经 view 统一协调；具体唤起谁取决于用户上次打开的是主窗口还是迷你条。
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let any_window = *window;
                    let _ = any_window.update(cx, |_, win, app| {
                        view.update(app, |view, vcx| view.toggle_active_window(win, vcx));
                    });
                }
            }

            // 右键菜单项。
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == "quit" {
                    cx.update(|cx| cx.quit());
                    return;
                } else if event.id == "open" {
                    let any_window = *window;
                    let _ = any_window.update(cx, |_, win, app| {
                        view.update(app, |view, vcx| view.show_main_window(win, vcx));
                    });
                } else if event.id == "record" {
                    // 经 AnyWindowHandle::update 取 Window 而**不租借 Root 视图**：
                    // toggle_recording 内部会 push_notification（更新 Root），若此处已租借 Root
                    // 会触发"cannot update Root while it is already being updated"双重租借 panic。
                    // `*window` 借 WindowHandle<Root> 的 Deref 得到 AnyWindowHandle。
                    let any_window = *window;
                    let _ = any_window.update(cx, |_, win, app| {
                        view.update(app, |view, vcx| {
                            view.toggle_recording(win, vcx);
                        });
                    });
                } else if event.id == "mini" {
                    // 同 record：经 AnyWindowHandle::update 取 Window 而不租借 Root，
                    // 避免与视图更新/通知双重租借。
                    let any_window = *window;
                    let _ = any_window.update(cx, |_, win, app| {
                        view.update(app, |view, vcx| {
                            view.toggle_mini_bar(win, vcx);
                        });
                    });
                } else if event.id == "settings" {
                    // 设置是主窗口上的覆盖层：先显示主窗口（并隐藏迷你条），再打开覆盖层。
                    // 用 AnyWindowHandle（`*window`）取 Window 而不租借 Root，避免双重租借。
                    let any_window = *window;
                    let _ = any_window.update(cx, |_, win, app| {
                        view.update(app, |view, vcx| {
                            view.show_main_window(win, vcx);
                            view.open_settings(win, vcx);
                        });
                    });
                }
            }
        }
    })
    .detach();

    Ok(())
}

/// 隐藏主窗口到托盘（供关闭拦截与启动最小化调用）。
pub fn hide_to_tray(window: &Window) {
    if let Some(h) = window_hwnd(window) {
        hide_window(h);
    }
}

/// 显示主窗口并置前（供 view 协调"显示主窗口"调用）。
pub fn show_to_front(window: &Window) {
    if let Some(h) = window_hwnd(window) {
        show_window(h);
    }
}

/// 主窗口当前是否可见（供 view 切换显隐时判断）。
pub fn is_window_visible(window: &Window) -> bool {
    #[cfg(windows)]
    {
        window_hwnd(window).is_some_and(winimpl::is_visible)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        false
    }
}

/// 记录主窗口当前尺寸到配置（供退出时统一持久化、下次启动还原）。
/// 仅在「普通窗口」态（非最大化/全屏）且可见时记录——否则会把最大化/全屏尺寸误存为还原尺寸，
/// 或在隐藏到托盘时覆盖成无意义值。位置不记录（启动仍居中，见 `main.rs`）。
fn capture_main_size(win: &Window, app: &mut App) {
    if !is_window_visible(win) || win.is_maximized() || win.is_fullscreen() {
        return;
    }
    let sz = win.bounds().size;
    let w = f32::from(sz.width).round() as u32;
    let h = f32::from(sz.height).round() as u32;
    // 拒绝异常的过小尺寸（初始化中途或最小化竞态），避免覆盖成无意义值。
    if w < 200 || h < 200 {
        return;
    }
    if let Some(g) = app.try_global::<GlobalConfig>()
        && (g.0.window.width != w || g.0.window.height != h)
    {
        let wc = &mut app.global_mut::<GlobalConfig>().0.window;
        wc.width = w;
        wc.height = h;
    }
}

/// 设置/清除任务栏按钮右下角的状态角标（ITaskbarList3，Win32-only）。
fn set_taskbar_overlay(hwnd: isize, status: crate::branding::IconStatus) {
    #[cfg(windows)]
    overlay::set(hwnd, status);
    #[cfg(not(windows))]
    {
        let _ = (hwnd, status);
    }
}

/// 按图标状态刷新托盘图标（录制/转录态显示彩色徽标）。
pub fn set_tray_status(status: crate::branding::IconStatus, cx: &App) {
    if let Some(g) = cx.try_global::<GlobalTray>()
        && let Some(icon) = crate::branding::tray_icon(status)
    {
        let _ = g.tray.set_icon(Some(icon));
    }
}

// ───────────────────────────── 平台相关：窗口显隐 ─────────────────────────────

fn window_hwnd(window: &Window) -> Option<isize> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = HasWindowHandle::window_handle(window).ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}

fn show_window(h: isize) {
    #[cfg(windows)]
    winimpl::show(h);
    #[cfg(not(windows))]
    {
        let _ = h;
    }
}

fn hide_window(h: isize) {
    #[cfg(windows)]
    winimpl::hide(h);
    #[cfg(not(windows))]
    {
        let _ = h;
    }
}

#[cfg(windows)]
mod winimpl {
    use std::ffi::c_void;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow,
    };

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut c_void)
    }

    pub fn hide(h: isize) {
        unsafe {
            let _ = ShowWindow(hwnd(h), SW_HIDE);
        }
    }

    pub fn show(h: isize) {
        unsafe {
            let _ = ShowWindow(hwnd(h), SW_SHOW);
            let _ = SetForegroundWindow(hwnd(h));
        }
    }

    pub fn is_visible(h: isize) -> bool {
        unsafe { IsWindowVisible(hwnd(h)).as_bool() }
    }
}

/// 任务栏状态角标：ITaskbarList3::SetOverlayIcon + 由 RGBA 造 HICON（GDI）。
#[cfg(windows)]
mod overlay {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::ptr::null_mut;

    use windows::Win32::Foundation::{HWND, TRUE};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};
    use windows::core::PCWSTR;

    use crate::branding::{IconStatus, render_badge_rgba};

    thread_local! {
        /// 缓存的 ITaskbarList3（首次使用时创建；轮询循环固定在主线程）。
        static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) };
    }

    fn taskbar() -> Option<ITaskbarList3> {
        TASKBAR.with(|cell| {
            if cell.borrow().is_none() {
                unsafe {
                    // 若 gpui 已初始化 COM（拖放），这里返回 S_FALSE/RPC_E_CHANGED_MODE 均无妨。
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                    if let Ok(tb) =
                        CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                        && tb.HrInit().is_ok()
                    {
                        *cell.borrow_mut() = Some(tb);
                    }
                }
            }
            cell.borrow().clone()
        })
    }

    pub fn set(h: isize, status: IconStatus) {
        let Some(tb) = taskbar() else { return };
        let hwnd = HWND(h as *mut c_void);
        const SIZE: u32 = 20;
        match render_badge_rgba(SIZE, status) {
            Some(rgba) => {
                if let Some(icon) = hicon_from_rgba(&rgba, SIZE as i32) {
                    unsafe {
                        let _ = tb.SetOverlayIcon(hwnd, icon, PCWSTR::null());
                        // 任务栏会复制图标，随后销毁本地句柄避免泄漏。
                        let _ = DestroyIcon(icon);
                    }
                }
            }
            // Idle：清除角标。
            None => unsafe {
                let _ = tb.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null());
            },
        }
    }

    /// 由直通 RGBA8 构造 32bpp alpha HICON。
    fn hicon_from_rgba(rgba: &[u8], size: i32) -> Option<HICON> {
        unsafe {
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = size;
            bi.bmiHeader.biHeight = -size; // 负高 = top-down
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB.0;

            let mut bits: *mut c_void = null_mut();
            let hbm_color =
                CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            if bits.is_null() {
                let _ = DeleteObject(hbm_color.into());
                return None;
            }
            // RGBA(直通) → BGRA。
            let px = (size * size) as usize;
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, px * 4);
            for i in 0..px {
                dst[i * 4] = rgba[i * 4 + 2]; // B
                dst[i * 4 + 1] = rgba[i * 4 + 1]; // G
                dst[i * 4 + 2] = rgba[i * 4]; // R
                dst[i * 4 + 3] = rgba[i * 4 + 3]; // A
            }

            // 单色掩码（全 0；32bpp 的透明由 alpha 通道处理）。
            let hbm_mask = CreateBitmap(size, size, 1, 1, None);

            let ii = ICONINFO {
                fIcon: TRUE,
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: hbm_mask,
                hbmColor: hbm_color,
            };
            let icon = CreateIconIndirect(&ii).ok();
            let _ = DeleteObject(hbm_color.into());
            let _ = DeleteObject(hbm_mask.into());
            icon
        }
    }
}
