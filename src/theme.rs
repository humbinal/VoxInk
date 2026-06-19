//! 主题系统（M11 任务 11.2 + 2026-06-16 界面美化）。
//!
//! 在 gpui-component 基础浅/深色主题之上，叠加一套 VoxInk 品牌色板，
//! 营造「现代、简洁、小清新」的整体观感（薄荷青绿主色 + 柔和中性背景 + 更大圆角）。
//!
//! 关键点：`Theme::change` / `sync_system_appearance` 会用基础主题重置 `colors`，
//! 所以品牌覆盖必须在其之后再应用一次；主题切换时同样要重新叠加（见 [`apply`]）。

use gpui::{App, Hsla, Window};
use gpui_component::{Theme, ThemeMode};

/// 由角度/百分比构造 Hsla（gpui 内部用 0..1 表示），便于按设计稿书写。
const fn hsl(h: f32, s: f32, l: f32) -> Hsla {
    Hsla {
        h: h / 360.0,
        s: s / 100.0,
        l: l / 100.0,
        a: 1.0,
    }
}

// ───────────────────────────── 品牌色板 ─────────────────────────────
// 主色：薄荷青绿（teal/mint），清新而不刺眼。

/// 品牌主色（按钮、强调）。
pub const BRAND: Hsla = hsl(172.0, 58.0, 43.0);
/// 主色悬停态（更深）。
pub const BRAND_HOVER: Hsla = hsl(172.0, 58.0, 38.0);
/// 主色按下态。
pub const BRAND_ACTIVE: Hsla = hsl(172.0, 60.0, 33.0);

/// 状态色：就绪（清新绿）。
pub const STATUS_IDLE: Hsla = hsl(160.0, 50.0, 44.0);
/// 状态色：录音中（柔和珊瑚红，较默认更克制）。
pub const STATUS_RECORDING: Hsla = hsl(5.0, 74.0, 62.0);
/// 状态色：处理中（暖琥珀）。
pub const STATUS_PROCESSING: Hsla = hsl(36.0, 88.0, 56.0);

/// 删除/危险悬停色。
pub const DANGER: Hsla = hsl(5.0, 74.0, 60.0);

/// 转录模式开关轨道色——两种明亮且对比的颜色区分模式（无启用/禁用语义）。
/// 离线：清爽蓝；实时：暖琥珀。两色在明暗主题下均清晰。
pub const MODE_OFFLINE: Hsla = hsl(208.0, 72.0, 55.0);
pub const MODE_STREAMING: Hsla = hsl(33.0, 90.0, 57.0);

/// 主色的浅色填充（用于「新建」按钮底、当前记录项高亮）——浅色主题。
pub const BRAND_TINT_LIGHT: Hsla = hsl(172.0, 46.0, 93.0);
/// 主色的浅色填充——深色主题（低明度青绿）。
pub const BRAND_TINT_DARK: Hsla = hsl(172.0, 30.0, 22.0);

/// 当前主题下的「主色浅填充」（随明暗自动取值）。
pub fn brand_tint(cx: &App) -> Hsla {
    if Theme::global(cx).is_dark() {
        BRAND_TINT_DARK
    } else {
        BRAND_TINT_LIGHT
    }
}

/// 应用主题：先套用基础浅/深色，再叠加品牌色板。`window` 用于切换后立即刷新。
pub fn apply(theme: &str, window: &mut Window, cx: &mut App) {
    match theme.trim().to_ascii_lowercase().as_str() {
        "light" => Theme::change(ThemeMode::Light, Some(window), cx),
        "dark" => Theme::change(ThemeMode::Dark, Some(window), cx),
        // "system" 或未知值：跟随系统外观。
        _ => Theme::sync_system_appearance(Some(window), cx),
    }
    apply_brand(cx);
    window.refresh();
}

/// 在当前明暗模式之上叠加 VoxInk 品牌色板与圆角/间距细节。
fn apply_brand(cx: &mut App) {
    let dark = Theme::global(cx).is_dark();
    let t = Theme::global_mut(cx);

    // 更柔和的圆角，强化「小清新」观感。
    t.radius = gpui::px(8.0);
    t.radius_lg = gpui::px(12.0);

    // 主色族（Button::primary 等取自 primary*）。
    t.primary = BRAND;
    t.primary_hover = BRAND_HOVER;
    t.primary_active = BRAND_ACTIVE;
    t.primary_foreground = hsl(0.0, 0.0, 100.0);
    t.button_primary = BRAND;
    t.button_primary_hover = BRAND_HOVER;
    t.button_primary_active = BRAND_ACTIVE;
    t.button_primary_foreground = hsl(0.0, 0.0, 100.0);

    // 聚焦环 / 光标用主色。
    t.ring = BRAND;
    t.caret = BRAND;
    t.selection = with_alpha(BRAND, 0.20);
    t.link = BRAND;
    t.link_hover = BRAND_HOVER;

    if dark {
        // 深色：冷调深青灰背景，避免纯黑死板。
        t.background = hsl(200.0, 16.0, 12.0);
        t.foreground = hsl(190.0, 14.0, 90.0);
        t.sidebar = hsl(200.0, 16.0, 10.0);
        t.sidebar_foreground = hsl(190.0, 14.0, 88.0);
        t.border = hsl(200.0, 12.0, 22.0);
        t.sidebar_border = hsl(200.0, 12.0, 20.0);
        t.input = hsl(200.0, 12.0, 24.0);
        t.muted = hsl(200.0, 12.0, 20.0);
        t.muted_foreground = hsl(195.0, 10.0, 60.0);
        t.accent = hsl(200.0, 14.0, 18.0);
        t.accent_foreground = hsl(190.0, 14.0, 90.0);
        t.popover = hsl(200.0, 16.0, 14.0);
        t.popover_foreground = hsl(190.0, 14.0, 90.0);
        t.colors.list = t.colors.background;
        t.list_active = BRAND_TINT_DARK;
        t.list_hover = hsl(200.0, 14.0, 18.0);
        t.secondary = hsl(200.0, 14.0, 20.0);
        t.secondary_hover = hsl(200.0, 14.0, 24.0);
        t.secondary_foreground = hsl(190.0, 14.0, 88.0);
        t.title_bar = hsl(200.0, 16.0, 10.0);
        t.title_bar_border = hsl(200.0, 12.0, 20.0);
    } else {
        // 浅色：近白但带极淡冷青调，干净通透。
        t.background = hsl(180.0, 28.0, 99.0);
        t.foreground = hsl(200.0, 18.0, 22.0);
        t.sidebar = hsl(184.0, 30.0, 97.5);
        t.sidebar_foreground = hsl(200.0, 18.0, 26.0);
        t.border = hsl(192.0, 22.0, 90.0);
        t.sidebar_border = hsl(192.0, 22.0, 90.0);
        t.input = hsl(192.0, 22.0, 88.0);
        t.muted = hsl(186.0, 26.0, 95.5);
        t.muted_foreground = hsl(200.0, 10.0, 48.0);
        t.accent = hsl(184.0, 30.0, 95.0);
        t.accent_foreground = hsl(200.0, 18.0, 26.0);
        t.popover = hsl(0.0, 0.0, 100.0);
        t.popover_foreground = hsl(200.0, 18.0, 22.0);
        t.colors.list = t.colors.background;
        t.list_active = BRAND_TINT_LIGHT;
        t.list_hover = hsl(184.0, 30.0, 96.0);
        t.secondary = hsl(186.0, 26.0, 95.0);
        t.secondary_hover = hsl(186.0, 26.0, 92.0);
        t.secondary_foreground = hsl(200.0, 18.0, 30.0);
        t.title_bar = hsl(184.0, 30.0, 98.0);
        t.title_bar_border = hsl(192.0, 22.0, 91.0);
    }
}

/// 给定颜色叠加透明度。
fn with_alpha(mut c: Hsla, a: f32) -> Hsla {
    c.a = a;
    c
}
