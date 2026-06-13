//! 开机自启动（M5，任务 5.3）。
//!
//! 用 `auto-launch` 管理注册表(Windows)/LaunchAgent(macOS)/XDG Autostart(Linux)。
//! M11 设置面板上线前，由配置项 `general.launch_at_startup` 驱动（启动时同步一次）。

use anyhow::{Context, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

fn build() -> Result<AutoLaunch> {
    let exe = std::env::current_exe().context("获取可执行文件路径失败")?;
    let exe_str = exe.to_string_lossy().to_string();
    AutoLaunchBuilder::new()
        .set_app_name("VoxInk")
        .set_app_path(&exe_str)
        .build()
        .context("构建开机自启配置失败")
}

/// 启用或禁用开机自启（幂等）。
///
/// 仅在状态需要改变时操作——`auto-launch` 的 `disable()` 在条目不存在时会报
/// "系统找不到指定的文件"，故先查当前状态再决定。
pub fn set_enabled(enabled: bool) -> Result<()> {
    let auto = build()?;
    let current = auto.is_enabled().unwrap_or(false);
    if enabled && !current {
        auto.enable().context("启用开机自启失败")?;
    } else if !enabled && current {
        auto.disable().context("禁用开机自启失败")?;
    }
    Ok(())
}

/// 查询当前是否已启用开机自启（M11 设置面板回显用）。
#[allow(dead_code)] // M11 设置面板读取
pub fn is_enabled() -> Result<bool> {
    build()?.is_enabled().context("查询开机自启状态失败")
}
