//! 单实例限制（§4.5.4，Windows）。
//!
//! 用命名互斥量（`CreateMutexW`）检测同一登录会话内是否已有实例：
//! - 首个实例创建互斥量并持有句柄至进程退出（OS 自动释放，崩溃也不残留陈旧锁）。
//! - 后续实例发现 `ERROR_ALREADY_EXISTS`，则把已运行实例的窗口唤到前台再让本进程退出，
//!   避免多实例争抢 config.toml / history.db / 自动更新 / 全局热键 / 托盘等共享状态。
//!
//! 非 Windows 平台暂不限制（项目 Windows 优先），始终返回可继续启动的 guard。

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SW_SHOW, SetForegroundWindow, ShowWindow,
    };
    use windows::core::{PCWSTR, w};

    /// 互斥量名。`Local\` = 限当前登录会话单实例（不同 Windows 用户各自一个实例，合理）。
    const MUTEX_NAME: PCWSTR = w!("Local\\VoxInk_SingleInstance");
    /// 主窗口标题（须与 `WindowOptions.titlebar.title` 一致，供 `FindWindowW` 定位已有实例）。
    const WINDOW_TITLE: PCWSTR = w!("VoxInk");

    /// 持有互斥量句柄使其存活至进程退出（drop/进程结束时 OS 释放命名互斥量）。
    pub struct InstanceGuard {
        #[allow(dead_code)]
        handle: HANDLE,
    }

    /// 尝试获取单实例。
    /// - `Some(guard)`：本进程是唯一实例，须持有 guard 至退出。
    /// - `None`：已有实例（已尝试唤起其窗口），调用方应立即退出。
    pub fn acquire() -> Option<InstanceGuard> {
        unsafe {
            let handle = match CreateMutexW(None, true, MUTEX_NAME) {
                Ok(h) => h,
                Err(e) => {
                    // 互斥量创建失败：放行，绝不因单实例机制自身故障而无法启动。
                    tracing::warn!("创建单实例互斥量失败，跳过单实例限制: {e}");
                    return Some(InstanceGuard {
                        handle: HANDLE::default(),
                    });
                }
            };
            // CreateMutexW 成功路径不改 last error，故此处可靠反映"互斥量是否已存在"。
            if GetLastError() == ERROR_ALREADY_EXISTS {
                tracing::info!("检测到已有 VoxInk 实例，唤起其窗口并退出本进程");
                activate_existing();
                let _ = CloseHandle(handle); // 本进程多余句柄；不影响已有实例持有的同名互斥量
                return None;
            }
            Some(InstanceGuard { handle })
        }
    }

    /// 定位已运行实例的主窗口并显示 + 置前（隐藏到托盘的窗口仍可被 `FindWindowW` 找到）。
    fn activate_existing() {
        unsafe {
            if let Ok(hwnd) = FindWindowW(PCWSTR::null(), WINDOW_TITLE) {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct InstanceGuard;

    pub fn acquire() -> Option<InstanceGuard> {
        Some(InstanceGuard)
    }
}

pub use imp::acquire;
