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
//! 📝 改键 UI（2026-06-19）：设置面板「快捷键」区支持捕获按键直接改键并即时重注册，
//! 无需编辑 config.toml。[`apply_shortcuts`] 用新配置重注册全部热键并返回冲突项；
//! [`suspend`] 在捕获按键期间临时注销全部热键（否则全局热键会截获按键、传不到窗口）。

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Result, anyhow};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui::{AnyWindowHandle, App, Entity, WindowHandle};
use gpui_component::Root;

use crate::app::{VoxInk, notify};
use crate::config::ShortcutsConfig;

/// 三类全局动作。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    ToggleRecording,
    ToggleWindow,
    CopyAndPaste,
    ToggleMiniBar,
}

/// 持有热键管理器（drop 会注销所有热键），并维护「已注册 id → 动作」映射，
/// 供轮询循环分发，以及设置面板内的实时重注册。
/// gpui `Global` 无 `Send` 约束，故 !Send 的管理器可放入（与 `GlobalTray` 同）。
pub struct GlobalHotkeys {
    manager: GlobalHotKeyManager,
    bindings: HashMap<u32, HotkeyAction>,
    registered: Vec<HotKey>,
    /// 上次 [`apply_shortcuts`] 注册失败（多为被其它应用占用）的动作集合，供设置面板标记冲突。
    failed: Vec<HotkeyAction>,
}

impl gpui::Global for GlobalHotkeys {}

impl GlobalHotkeys {
    /// 注销当前已注册的全部热键并清空映射（用于改键前的暂停与重注册前的复位）。
    fn clear(&mut self) {
        for hk in std::mem::take(&mut self.registered) {
            let _ = self.manager.unregister(hk);
        }
        self.bindings.clear();
    }
}

/// 某个全局动作在上次注册时是否失败（被占用/解析失败）。供设置面板显示「注册失败」标记。
pub fn registration_failed(action: HotkeyAction, cx: &App) -> bool {
    cx.try_global::<GlobalHotkeys>()
        .is_some_and(|hk| hk.failed.contains(&action))
}

/// 创建热键管理器、按配置注册并启动事件轮询分发。
pub fn setup_hotkeys(
    window: WindowHandle<Root>,
    view: Entity<VoxInk>,
    shortcuts: &ShortcutsConfig,
    cx: &mut App,
) -> Result<()> {
    let manager = GlobalHotKeyManager::new().map_err(|e| anyhow!("创建全局热键管理器失败: {e}"))?;
    cx.set_global(GlobalHotkeys {
        manager,
        bindings: HashMap::new(),
        registered: Vec::new(),
        failed: Vec::new(),
    });

    let conflicts = apply_shortcuts(shortcuts, cx);

    // 用 AnyWindowHandle（其 update 传入 AnyView、**不租借 Root**）访问窗口；
    // 而 WindowHandle<Root>::update 会租借 Root，若闭包内再 push_notification（其内部又租借
    // Root）会触发双重租借 panic。必须用 `.into()` 显式转换为 AnyWindowHandle。
    let any_window: AnyWindowHandle = window.into();

    if !conflicts.is_empty() {
        let msg = format!(
            "以下快捷键注册失败（可能与其它应用冲突，可在「设置 → 快捷键」中改键）：{}",
            conflicts.join("、")
        );
        tracing::warn!("{msg}");
        let _ = any_window.update(cx, |_, win, app| notify(win, msg, app));
    }

    // 轮询热键事件并分发（仅处理按下）。映射可能被设置面板更新，故每次事件都现取全局映射。
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
                let _ = any_window.update(cx, |_, win, app| {
                    let action = app
                        .try_global::<GlobalHotkeys>()
                        .and_then(|hk| hk.bindings.get(&id).copied());
                    if let Some(action) = action {
                        dispatch(action, &view, win, app);
                    }
                });
            }
        }
    })
    .detach();

    Ok(())
}

/// 把热键动作派发到对应行为。
fn dispatch(action: HotkeyAction, view: &Entity<VoxInk>, win: &mut gpui::Window, app: &mut App) {
    match action {
        HotkeyAction::ToggleRecording => {
            view.update(app, |view, vcx| view.toggle_recording(win, vcx));
        }
        HotkeyAction::ToggleWindow => {
            crate::tray::toggle_window_visibility(win);
        }
        HotkeyAction::CopyAndPaste => {
            view.update(app, |view, vcx| view.copy_and_paste(win, vcx));
        }
        HotkeyAction::ToggleMiniBar => {
            view.update(app, |view, vcx| view.toggle_mini_bar(win, vcx));
        }
    }
}

/// 用给定配置重注册全部全局热键（先清空旧的），返回注册失败（多为冲突）的项标签。
/// 设置面板改键后调用即可即时生效；亦用作改键捕获后的「恢复」。
pub fn apply_shortcuts(shortcuts: &ShortcutsConfig, cx: &mut App) -> Vec<String> {
    if cx.try_global::<GlobalHotkeys>().is_none() {
        return Vec::new();
    }
    let specs = [
        (
            shortcuts.toggle_recording.clone(),
            HotkeyAction::ToggleRecording,
            "开始/停止录音",
        ),
        (
            shortcuts.toggle_window.clone(),
            HotkeyAction::ToggleWindow,
            "唤起/隐藏窗口",
        ),
        (
            shortcuts.copy_and_paste.clone(),
            HotkeyAction::CopyAndPaste,
            "复制并粘贴",
        ),
        (
            shortcuts.toggle_mini_bar.clone(),
            HotkeyAction::ToggleMiniBar,
            "显示/隐藏迷你条",
        ),
    ];
    let hk = cx.global_mut::<GlobalHotkeys>();
    hk.clear();
    hk.failed.clear();
    let mut conflicts = Vec::new();
    for (spec, action, label) in specs {
        match register_one(&hk.manager, spec.trim()) {
            Ok(hotkey) => {
                tracing::info!("已注册全局快捷键 [{label}]: {spec}");
                hk.bindings.insert(hotkey.id(), action);
                hk.registered.push(hotkey);
            }
            Err(e) => {
                tracing::warn!("注册全局快捷键失败 [{label}: {spec}]: {e:#}");
                hk.failed.push(action);
                conflicts.push(format!("{label}（{spec}）"));
            }
        }
    }
    conflicts
}

/// 改键捕获期间临时注销全部热键，使按键能传达到窗口被捕获
/// （全局热键由 OS 截获、不会作为按键事件传给窗口）。捕获结束后调 [`apply_shortcuts`] 恢复。
pub fn suspend(cx: &mut App) {
    if cx.try_global::<GlobalHotkeys>().is_some() {
        cx.global_mut::<GlobalHotkeys>().clear();
    }
}

/// 解析并注册单个热键；成功返回 [`HotKey`]（含其 id），失败返回错误。
fn register_one(manager: &GlobalHotKeyManager, spec: &str) -> Result<HotKey> {
    let hotkey = HotKey::from_str(spec).map_err(|e| anyhow!("无法解析快捷键 \"{spec}\": {e}"))?;
    manager
        .register(hotkey)
        .map_err(|e| anyhow!("注册失败（可能与其它应用冲突）: {e}"))?;
    Ok(hotkey)
}

/// 把一次 gpui 按键事件转成 global-hotkey 可解析的快捷键字符串（如 "Ctrl+Shift+W"）。
/// 要求「主键 + 至少一个修饰键」且主键可被 global-hotkey 识别，否则返回 None
/// （纯修饰键、不支持的键、无修饰的裸键都返回 None，调用方应继续等待有效组合）。
pub fn accelerator_from_keystroke(ks: &gpui::Keystroke) -> Option<String> {
    let key = normalize_key(&ks.key)?;
    let m = &ks.modifiers;
    // 要求至少一个修饰键，避免把裸键注册成全局热键（会吞掉普通打字）。
    if !(m.control || m.alt || m.shift || m.platform) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    if m.control {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    if m.platform {
        parts.push("Super");
    }
    let spec = format!("{}+{}", parts.join("+"), key);
    // 校验确实可被解析（否则视为不支持的键）。
    HotKey::from_str(&spec).ok().map(|_| spec)
}

/// 把 gpui 的按键名规整为 global-hotkey 接受且显示友好的主键 token；
/// 纯修饰键与不支持的键返回 None。
fn normalize_key(key: &str) -> Option<String> {
    // 纯修饰键不能作为主键。
    if matches!(
        key,
        "control"
            | "ctrl"
            | "alt"
            | "option"
            | "shift"
            | "platform"
            | "super"
            | "win"
            | "cmd"
            | "command"
            | "function"
            | "fn"
    ) {
        return None;
    }
    // 单个字母/数字 → 大写。
    if key.len() == 1 {
        let c = key.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
    }
    // 功能键 f1..f24。
    if let Some(n) = key.strip_prefix('f')
        && let Ok(num) = n.parse::<u8>()
        && (1..=24).contains(&num)
    {
        return Some(format!("F{num}"));
    }
    let pretty = match key {
        "space" => "Space",
        "enter" => "Enter",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "escape" => "Escape",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        _ => return None,
    };
    Some(pretty.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Keystroke, Modifiers};

    fn ks(control: bool, alt: bool, shift: bool, platform: bool, key: &str) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control,
                alt,
                shift,
                platform,
                function: false,
            },
            key: key.to_string(),
            key_char: None,
        }
    }

    #[test]
    fn accelerator_builds_for_letters_named_and_fkeys() {
        assert_eq!(
            accelerator_from_keystroke(&ks(true, false, true, false, "w")).as_deref(),
            Some("Ctrl+Shift+W")
        );
        assert_eq!(
            accelerator_from_keystroke(&ks(true, false, true, false, "space")).as_deref(),
            Some("Ctrl+Shift+Space")
        );
        assert_eq!(
            accelerator_from_keystroke(&ks(true, true, false, false, "f5")).as_deref(),
            Some("Ctrl+Alt+F5")
        );
        assert_eq!(
            accelerator_from_keystroke(&ks(true, false, false, false, "pageup")).as_deref(),
            Some("Ctrl+PageUp")
        );
    }

    #[test]
    fn accelerator_rejects_bare_modifier_only_and_unsupported() {
        // 无修饰键的裸键：拒绝。
        assert!(accelerator_from_keystroke(&ks(false, false, false, false, "w")).is_none());
        // 纯修饰键（未按主键）：拒绝。
        assert!(accelerator_from_keystroke(&ks(true, false, false, false, "control")).is_none());
        // 不支持的主键：拒绝。
        assert!(accelerator_from_keystroke(&ks(true, false, false, false, "menu")).is_none());
    }

    #[test]
    fn accelerator_output_is_parseable_by_global_hotkey() {
        let spec = accelerator_from_keystroke(&ks(true, true, true, false, "v")).unwrap();
        assert_eq!(spec, "Ctrl+Alt+Shift+V");
        assert!(HotKey::from_str(&spec).is_ok());
    }
}
