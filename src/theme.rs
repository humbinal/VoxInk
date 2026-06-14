//! 主题系统（M11 任务 11.2）。
//!
//! 把配置项 `general.theme`（"light" | "dark" | "system"）映射为 gpui-component 的主题模式并应用。
//! "system" 跟随操作系统外观（`Theme::sync_system_appearance`）。

use gpui::{App, Window};
use gpui_component::{Theme, ThemeMode};

/// 应用主题。`window` 用于切换后立即刷新渲染。
pub fn apply(theme: &str, window: &mut Window, cx: &mut App) {
    match theme.trim().to_ascii_lowercase().as_str() {
        "light" => Theme::change(ThemeMode::Light, Some(window), cx),
        "dark" => Theme::change(ThemeMode::Dark, Some(window), cx),
        // "system" 或未知值：跟随系统外观。
        _ => Theme::sync_system_appearance(Some(window), cx),
    }
}
