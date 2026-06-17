//! 系统托盘集成（M5，任务 5.1/5.2）。
//!
//! - `tray-icon` 创建托盘图标 + 右键菜单；左键单击切换主窗口显隐。
//! - 托盘/菜单事件经全局 channel 投递，用 GPUI 前台定时任务轮询并分发（GPUI 的消息循环
//!   会泵送 tray-icon/muda 隐藏窗口的消息）。
//! - 关闭按钮（X）经 `on_window_should_close` 改为隐藏到托盘，不退出进程。
//!
//! ⚠️ 平台说明（§12.3）：gpui 在 Windows 上不公开 hide/minimize，故隐藏/显示窗口直接调用
//! Win32 `ShowWindow`（经 HWND）。非 Windows 平台暂为降级实现（不隐藏），后续里程碑再补。

use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui::{App, Entity, Window, WindowHandle};
use gpui_component::Root;

use crate::app::VoxInk;

/// 持有托盘实例使其在应用生命周期内存活（拖放会导致图标消失）。
pub struct GlobalTray(#[allow(dead_code)] pub tray_icon::TrayIcon);

impl gpui::Global for GlobalTray {}

/// 创建托盘图标与菜单，注册关闭拦截，并启动事件轮询。
pub fn setup_tray(window: WindowHandle<Root>, view: Entity<VoxInk>, cx: &mut App) -> Result<()> {
    use tray_icon::TrayIconBuilder;
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

    let menu = Menu::new();
    let open = MenuItem::with_id("open", "打开主界面", true, None);
    let record = MenuItem::with_id("record", "开始/停止录音", true, None);
    let settings = MenuItem::with_id("settings", "设置", true, None);
    let quit = MenuItem::with_id("quit", "退出", true, None);
    let append = |item: &dyn tray_icon::menu::IsMenuItem| -> Result<()> {
        menu.append(item).map_err(|e| anyhow!("添加托盘菜单项失败: {e}"))
    };
    append(&open)?;
    append(&PredefinedMenuItem::separator())?;
    append(&record)?;
    append(&settings)?;
    append(&PredefinedMenuItem::separator())?;
    append(&quit)?;

    let mut builder = TrayIconBuilder::new()
        .with_id("voxink-tray")
        .with_menu(Box::new(menu))
        .with_tooltip("VoxInk")
        // 左键不弹菜单——留给"切换窗口显隐"。
        .with_menu_on_left_click(false);
    if let Some(icon) = tray_icon_image() {
        builder = builder.with_icon(icon);
    }
    let tray = builder.build().map_err(|e| anyhow!("创建系统托盘失败: {e}"))?;
    cx.set_global(GlobalTray(tray));

    // 关闭按钮（X）→ 隐藏到托盘，取消真正的关闭。
    let _ = window.update(cx, |_, win, ctx| {
        win.on_window_should_close(ctx, |win, _app| {
            hide_to_tray(win);
            false
        });
    });

    // 轮询托盘/菜单事件。
    cx.spawn(async move |cx| {
        use tray_icon::menu::MenuEvent;
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

        loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;

            // 左键单击：切换窗口显隐。
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = window.update(cx, |_, win, _| {
                        if let Some(h) = window_hwnd(win) {
                            toggle_window(h);
                        }
                    });
                }
            }

            // 右键菜单项。
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == "quit" {
                    cx.update(|cx| cx.quit());
                    return;
                } else if event.id == "open" {
                    let _ = window.update(cx, |_, win, _| {
                        if let Some(h) = window_hwnd(win) {
                            show_window(h);
                        }
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
                } else if event.id == "settings" {
                    // 设置是主窗口上的覆盖层：先把主窗口显示/置前，再打开覆盖层。
                    // 用 AnyWindowHandle（`*window`）取 Window 而不租借 Root，避免与
                    // 视图更新/通知产生双重租借（同 record 分支）。
                    let any_window = *window;
                    let _ = any_window.update(cx, |_, win, app| {
                        if let Some(h) = window_hwnd(win) {
                            show_window(h);
                        }
                        view.update(app, |view, vcx| {
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

/// 切换主窗口显隐（供全局快捷键"唤起/隐藏窗口"调用，M9）。
pub fn toggle_window_visibility(window: &Window) {
    if let Some(h) = window_hwnd(window) {
        toggle_window(h);
    }
}

/// 程序化生成 32×32 的托盘图标（强调色实心圆），避免依赖图片资源。
fn tray_icon_image() -> Option<tray_icon::Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let center = SIZE as f32 / 2.0;
    let radius = 14.0_f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            if dx * dx + dy * dy <= radius * radius {
                let idx = ((y * SIZE + x) * 4) as usize;
                rgba[idx] = 0x4A;
                rgba[idx + 1] = 0x90;
                rgba[idx + 2] = 0xD9;
                rgba[idx + 3] = 0xFF;
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).ok()
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

fn toggle_window(h: isize) {
    #[cfg(windows)]
    {
        if winimpl::is_visible(h) {
            winimpl::hide(h);
        } else {
            winimpl::show(h);
        }
    }
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
        IsWindowVisible, SetForegroundWindow, ShowWindow, SW_HIDE, SW_SHOW,
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
