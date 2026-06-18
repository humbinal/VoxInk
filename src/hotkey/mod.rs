//! 全局快捷键（M9，任务 9.1/9.2/9.4）。
//!
//! - 用 `global-hotkey`（Tauri 团队，跨平台）注册三类系统级热键：开始/停止录音、
//!   唤起/隐藏窗口、复制并粘贴。绑定字符串取自配置 §2.7 `[shortcuts]`。
//! - 热键事件经全局 channel 投递，用 GPUI 前台定时任务轮询并分发（与托盘同机制）。
//! - 注册失败（多为与其它应用冲突）会汇总后弹出友好提示（验收：冲突给出提示）。
//! - 一键复制并粘贴：写入剪贴板后用 Win32 `SendInput` 模拟 Ctrl+V 粘贴到前台应用。
//!
//! ⚠️ 平台说明（§12.6）：`global-hotkey` 在 Windows 上创建消息窗口接收 `WM_HOTKEY`，
//! 由 gpui 的主线程消息循环泵送，故事件可达。粘贴模拟各平台不同，非 Windows 暂为降级。
//!
//! 📝 M9/M11 顺序说明：任务 9.3「热键自定义 UI（重新录制 + 冲突检测）」属设置面板，
//! 而设置面板在 M11。当前阶段用户经编辑 config.toml 的 `[shortcuts]` 自定义，重启生效；
//! 注册期冲突检测已实现（见上）。完整重录 UI 留待 M11，与 M5/M7 处理设置缺位的方式一致。

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Result, anyhow};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui::{AnyWindowHandle, App, Entity, WindowHandle};
use gpui_component::Root;

use crate::app::{VoxInk, notify};
use crate::config::ShortcutsConfig;

/// 持有热键管理器使其在应用生命周期内存活（drop 会注销所有热键）。
/// gpui `Global` 无 `Send` 约束，故 !Send 的管理器可放入（与 `GlobalTray` 同）。
pub struct GlobalHotkeys(#[allow(dead_code)] GlobalHotKeyManager);

impl gpui::Global for GlobalHotkeys {}

/// 注册全局快捷键并启动事件轮询分发。
pub fn setup_hotkeys(
    window: WindowHandle<Root>,
    view: Entity<VoxInk>,
    shortcuts: &ShortcutsConfig,
    cx: &mut App,
) -> Result<()> {
    let manager = GlobalHotKeyManager::new().map_err(|e| anyhow!("创建全局热键管理器失败: {e}"))?;

    // 注册三类热键，记录各自 id 以便分发；失败项汇总为冲突提示。
    let mut conflicts: Vec<String> = Vec::new();
    let toggle_recording_id = try_register(
        &manager,
        &shortcuts.toggle_recording,
        "开始/停止录音",
        &mut conflicts,
    );
    let toggle_window_id = try_register(
        &manager,
        &shortcuts.toggle_window,
        "唤起/隐藏窗口",
        &mut conflicts,
    );
    let copy_and_paste_id = try_register(
        &manager,
        &shortcuts.copy_and_paste,
        "复制并粘贴",
        &mut conflicts,
    );

    // 管理器交给全局保活；轮询循环只需各 id（u32，Copy）。
    cx.set_global(GlobalHotkeys(manager));

    // 用 AnyWindowHandle（其 update 传入 AnyView、**不租借 Root**）访问窗口；
    // 而 WindowHandle<Root>::update 会租借 Root，若闭包内再 push_notification（其内部又租借
    // Root）会触发双重租借 panic。`*window` 只是复制 WindowHandle<Root>（Copy），不会变成
    // AnyWindowHandle——必须用 `.into()` 显式转换。下面所有窗口访问都走 any_window。
    let any_window: AnyWindowHandle = window.into();

    if !conflicts.is_empty() {
        let msg = format!(
            "以下快捷键注册失败（可能与其它应用冲突，可在 config.toml 中更换）：{}",
            conflicts.join("、")
        );
        tracing::warn!("{msg}");
        let _ = any_window.update(cx, |_, win, app| notify(win, msg, app));
    }

    // 轮询热键事件并分发到对应动作（仅处理按下，忽略松开）。
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;

            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state != HotKeyState::Pressed {
                    continue;
                }
                let id = event.id;

                if Some(id) == toggle_recording_id {
                    let _ = any_window.update(cx, |_, win, app| {
                        view.update(app, |view, vcx| view.toggle_recording(win, vcx));
                    });
                } else if Some(id) == toggle_window_id {
                    let _ = any_window.update(cx, |_, win, _| {
                        crate::tray::toggle_window_visibility(win);
                    });
                } else if Some(id) == copy_and_paste_id {
                    let _ = any_window.update(cx, |_, win, app| {
                        view.update(app, |view, vcx| view.copy_and_paste(win, vcx));
                    });
                }
            }
        }
    })
    .detach();

    Ok(())
}

/// 解析并注册单个热键；成功返回其 id，失败记入冲突列表并返回 None。
fn try_register(
    manager: &GlobalHotKeyManager,
    spec: &str,
    label: &str,
    conflicts: &mut Vec<String>,
) -> Option<u32> {
    match register_one(manager, spec) {
        Ok(id) => {
            tracing::info!("已注册全局快捷键 [{label}]: {spec}");
            Some(id)
        }
        Err(e) => {
            tracing::warn!("注册全局快捷键失败 [{label}: {spec}]: {e:#}");
            conflicts.push(format!("{label}（{spec}）"));
            None
        }
    }
}

fn register_one(manager: &GlobalHotKeyManager, spec: &str) -> Result<u32> {
    let hotkey =
        HotKey::from_str(spec.trim()).map_err(|e| anyhow!("无法解析快捷键 \"{spec}\": {e}"))?;
    manager
        .register(hotkey)
        .map_err(|e| anyhow!("注册失败（可能与其它应用冲突）: {e}"))?;
    Ok(hotkey.id())
}

// ───────────────────────────── 平台相关：模拟粘贴（任务 9.4）─────────────────────────────

/// 模拟 Ctrl+V 粘贴到当前前台应用（剪贴板内容须已写入）。
///
/// 先释放用户可能仍按住的修饰键（热键如 Ctrl+Alt+B 触发时这些键多半还按着），
/// 再发送干净的 Ctrl+V，避免 Alt/Shift 污染按键组合。
pub fn simulate_paste() {
    #[cfg(windows)]
    winimpl::paste();
}

#[cfg(windows)]
mod winimpl {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT, VK_V,
    };

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    pub fn paste() {
        let inputs = [
            // 释放可能仍被按住的修饰键，得到干净的 Ctrl+V。
            key(VK_MENU, true),
            key(VK_SHIFT, true),
            key(VK_LWIN, true),
            key(VK_CONTROL, true),
            // Ctrl+V
            key(VK_CONTROL, false),
            key(VK_V, false),
            key(VK_V, true),
            key(VK_CONTROL, true),
        ];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }
}
