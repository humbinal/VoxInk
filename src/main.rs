//! VoxInk 应用入口 —— M1 任务 1.3；M2 任务 2.3（配置生命周期）。
//!
//! 职责：初始化日志、创建 Tokio 运行时、加载/保存配置、启动 GPUI 并打开主窗口（800×600）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod asr;
mod assets;
mod audio;
mod branding;
mod autolaunch;
mod config;
mod diagnostics;
mod history;
mod hotkey;
mod i18n;
mod mini;
mod polish;
mod settings;
mod single_instance;
mod state;
mod theme;
mod tray;
mod update;

use anyhow::Result;
use app::{GlobalConfig, GlobalTokioHandle, VoxInk, notify};
use assets::VoxInkAssets;
use config::VoxInkConfig;
use gpui::{
    AnyWindowHandle, App, Bounds, Entity, WindowBounds, WindowHandle, WindowOptions, prelude::*,
    px, size,
};
use gpui_component::{Root, TitleBar};
use rolling_file::{RollingConditionBasic, RollingFileAppender};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

// 多语言词典（编译期嵌入 crate 根 `locales/`），缺省回退简体中文（M11 任务 11.3）。
rust_i18n::i18n!("locales", fallback = "zh-CN");

/// 构建滚动文件日志写入器：`%LOCALAPPDATA%\VoxInk\logs\voxink.log`，
/// 按天 **或** 单文件超过 10MB 任一触发即滚动（rolling-file 双条件），保留最近 14 个历史文件。
fn build_file_appender() -> Result<RollingFileAppender<RollingConditionBasic>> {
    let dir = VoxInkConfig::log_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("voxink.log");
    let condition = RollingConditionBasic::new()
        .daily()
        .max_size(10 * 1024 * 1024);
    let appender = RollingFileAppender::new(&path, condition, 14)
        .map_err(|e| anyhow::anyhow!("无法创建日志文件 {}: {e}", path.display()))?;
    Ok(appender)
}

/// 初始化日志：始终写滚动文件（release 关键——`windows` 子系统无控制台，否则日志全丢），
/// 并保留控制台层（debug 终端 / release 从已有终端启动时可见）。
///
/// 默认过滤：应用自身 INFO；屏蔽 gpui Windows 后端的伪错误噪声 ——
/// `gpui_windows::events` 会把 GetLastError==0（"操作成功完成。 0x0"）当成 ERROR 打印；
/// `gpui_windows::window` / `gpui::window` 会在窗口关闭时打印句柄失效日志。均非真实错误。
/// 需要查看完整日志时用 RUST_LOG 覆盖（如 `RUST_LOG=debug`）。
///
/// 返回的 [`WorkerGuard`] 必须在 `main` 全程持有，否则非阻塞写入 worker 的缓冲日志会丢失。
fn init_tracing() -> Option<WorkerGuard> {
    let filter = || {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(
                "info,gpui_windows::events=off,gpui_windows::window=off,gpui::window=off",
            )
        })
    };
    // EnvFilter 不是 Clone，控制台与文件各取一份（语义一致）。
    let console_layer = fmt::layer().with_filter(filter());

    match build_file_appender() {
        Ok(appender) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            let file_layer = fmt::layer()
                .with_ansi(false) // 文件里不要 ANSI 颜色转义码
                .with_writer(non_blocking)
                .with_filter(filter());
            tracing_subscriber::registry()
                .with(console_layer)
                .with(file_layer)
                .init();
            Some(guard)
        }
        Err(e) => {
            tracing_subscriber::registry().with(console_layer).init();
            tracing::warn!("文件日志初始化失败，仅控制台输出: {e:#}");
            None
        }
    }
}

fn main() -> Result<()> {
    // guard 须持有至 main 结束，否则非阻塞日志 worker 的缓冲会在退出时丢失。
    let _log_guard = init_tracing();

    // 单实例限制（§4.5.4）：已有实例时唤起其窗口并退出本进程——须在创建 runtime / 加载配置 /
    // 打开数据库之前，这些都会争抢 config.toml / history.db / 自动更新等共享状态。
    // guard 持有至 main 结束（进程退出由 OS 释放命名互斥量）。
    let _instance = match single_instance::acquire() {
        Some(guard) => guard,
        None => return Ok(()),
    };

    // 创建 Tokio 多线程运行时，供音频 I/O、网络等耗时任务调度。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("无法创建 Tokio 运行时");
    let _runtime_guard = runtime.enter();
    // 句柄供 GPUI 处理器把网络任务派发到 Tokio 运行时执行（reqwest 需要 reactor）。
    let tokio_handle = runtime.handle().clone();

    tracing::info!("VoxInk 启动中……");

    // 清理上次自动更新残留的旧版本可执行文件（`*.old.exe`）；幂等、失败不阻塞启动（M13）。
    update::cleanup_old_exe();

    // 启动时加载配置（不存在则用默认值；密文 API Key 自动解密为内存明文）。
    let config = VoxInkConfig::load();
    let first_run = VoxInkConfig::config_path()
        .map(|p| !p.exists())
        .unwrap_or(false);

    // 首次运行将默认配置落盘，便于用户查看/编辑。
    if first_run {
        match config.save() {
            Ok(()) => tracing::info!("已创建默认配置"),
            Err(e) => tracing::error!("写入默认配置失败: {e:#}"),
        }
    }

    // 同步开机自启状态（M11 设置面板上线前，由配置项 general.launch_at_startup 驱动）。
    if let Err(e) = autolaunch::set_enabled(config.general.launch_at_startup) {
        tracing::warn!("同步开机自启状态失败: {e:#}");
    }

    let app = gpui_platform::application().with_assets(VoxInkAssets);
    app.run(move |cx| {
        // 初始化 gpui-component（主题、输入、菜单等子系统）。
        gpui_component::init(cx);

        // 按配置应用界面语言（设置全局 locale；同时影响 gpui-component 内置文案）。
        i18n::apply_locale(&config.general.language);

        // 配置以全局形式承载，供各 View 读写。
        cx.set_global(GlobalConfig(config.clone()));
        cx.set_global(GlobalTokioHandle(tokio_handle));

        // 历史数据库（M10）：打开 + 按保留天数清理过期记录（任务 10.4）。
        // 须在打开窗口前设置，便于主视图初始化时确定当前会话。
        match history::db::HistoryDb::open() {
            Ok(db) => {
                match db.purge_older_than(config.text.history_retention_days) {
                    Ok(n) if n > 0 => tracing::info!("已清理 {n} 条过期历史记录"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("清理过期历史失败: {e:#}"),
                }
                // 音频维护：过期片段清理 + 孤儿/旧临时文件清理（2026-06-16）。
                app::cleanup_audio_on_startup(&db, &config.storage);
                cx.set_global(history::GlobalHistory(db));
            }
            Err(e) => tracing::error!("打开历史数据库失败（历史功能将不可用）: {e:#}"),
        }

        // 退出时持久化配置（含已加密的 API Key）。
        cx.on_app_quit(|cx: &mut App| {
            let config = cx.try_global::<GlobalConfig>().map(|g| g.0.clone());
            async move {
                if let Some(config) = config {
                    match config.save() {
                        Ok(()) => tracing::info!("配置已保存"),
                        Err(e) => tracing::error!("保存配置失败: {e:#}"),
                    }
                }
            }
        })
        .detach();

        // 窗口尺寸取自配置（无配置文件用默认值，§6.1）。
        let bounds = Bounds::centered(
            None,
            size(
                px(config.window.width as f32),
                px(config.window.height as f32),
            ),
            cx,
        );
        let mut view_holder: Option<Entity<VoxInk>> = None;
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // 自绘标题栏（无系统标题栏）：appears_transparent: true，配套 gpui_component::TitleBar。
                    // 仍设置 title（不绘制，仅供任务栏/Alt-Tab 标识）。
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("VoxInk".into()),
                        ..TitleBar::title_bar_options()
                    }),
                    window_min_size: Some(size(px(640.), px(420.))),
                    ..Default::default()
                },
                |window, cx| {
                    // gpui-component 要求顶层视图包裹在 Root 中（承载弹窗/抽屉/通知层）。
                    let view = cx.new(|cx| VoxInk::new(window, cx));
                    view_holder = Some(view.clone());
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("无法创建主窗口");

        cx.activate(true);
        tracing::info!("主窗口已打开");

        // 系统托盘集成（M5）：图标 + 菜单 + 关闭隐藏到托盘。
        // 全局快捷键（M9）：录音切换 / 窗口切换 / 复制并粘贴。
        if let Some(view) = view_holder {
            if let Err(e) = tray::setup_tray(window, view.clone(), cx) {
                tracing::error!("初始化系统托盘失败: {e:#}");
            }
            if let Err(e) = hotkey::setup_hotkeys(window, view, &config.shortcuts, cx) {
                tracing::error!("初始化全局快捷键失败: {e:#}");
            }
        }

        // 启动时静默检查更新（每日至多一次；有新版发 toast 提示，不打断操作）。M13。
        spawn_startup_update_check(window, cx);

        // 启动最小化到托盘（首次运行仍显示主窗口，便于初次使用）。
        if config.general.start_minimized && !first_run {
            let _ = window.update(cx, |_, win, _| tray::hide_to_tray(win));
            tracing::info!("按配置启动最小化到托盘");
        }
    });

    Ok(())
}

/// 启动时静默检查更新（M13，§11.3）：
/// 仅当 `general.auto_check_update` 且距 `update.last_check` ≥ 24h 时联网；
/// 发现高于当前、且非用户「跳过」的版本时发 toast 提示。检查后写回 `last_check`。
fn spawn_startup_update_check(window: WindowHandle<Root>, cx: &mut App) {
    /// 启动检查节流：两次自动检查至少间隔 24 小时。
    const CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

    let Some(cfg) = cx.try_global::<GlobalConfig>().map(|g| g.0.clone()) else {
        return;
    };
    if !cfg.general.auto_check_update {
        return;
    }
    let now = chrono::Utc::now().timestamp();
    if now.saturating_sub(cfg.update.last_check) < CHECK_INTERVAL_SECS {
        return;
    }
    let Some(handle) = cx.try_global::<GlobalTokioHandle>().map(|g| g.0.clone()) else {
        return;
    };
    let skipped = cfg.update.skipped_version.clone();
    // 用 AnyWindowHandle 访问窗口：其 update 不租借 Root，闭包内 push_notification 才不触发
    // 双重租借 panic（CLAUDE.md §3）。
    let any_window: AnyWindowHandle = window.into();

    cx.spawn(async move |cx| {
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle.spawn(async move {
            let _ = tx.send(update::check_latest().await);
        });
        let Ok(result) = rx.await else { return };

        // 无论结果如何都写回 last_check（避免反复失败时每次启动都联网）。
        // AsyncApp::update 直接返回闭包值（单元），作为裸语句调用（CLAUDE.md §3）。
        cx.update(|app| {
            if let Some(g) = app.try_global::<GlobalConfig>() {
                let mut c = g.0.clone();
                c.update.last_check = now;
                app.set_global(GlobalConfig(c));
            }
        });

        match result {
            Ok(latest) if latest.is_newer && latest.version != skipped => {
                let msg = format!(
                    "发现新版本 v{}，可在「设置 → 关于」中更新",
                    latest.version
                );
                tracing::info!("{msg}");
                let _ = any_window.update(cx, |_, win, app| notify(win, msg, app));
            }
            Ok(_) => tracing::info!("已是最新版本或用户已跳过该版本"),
            Err(e) => tracing::warn!("启动检查更新失败: {e:#}"),
        }
    })
    .detach();
}
