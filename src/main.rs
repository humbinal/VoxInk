//! VoxInk 应用入口 —— M1 任务 1.3；M2 任务 2.3（配置生命周期）。
//!
//! 职责：初始化日志、创建 Tokio 运行时、加载/保存配置、启动 GPUI 并打开主窗口（800×600）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod asr;
mod audio;
mod autolaunch;
mod config;
mod diagnostics;
mod history;
mod hotkey;
mod i18n;
mod settings;
mod state;
mod theme;
mod tray;

use anyhow::Result;
use app::{GlobalConfig, GlobalTokioHandle, VoxInk};
use config::VoxInkConfig;
use gpui::{
    prelude::*, px, size, App, Bounds, Entity, TitlebarOptions, WindowBounds, WindowOptions,
};
use gpui_component::Root;
use gpui_component_assets::Assets;
use tracing_subscriber::EnvFilter;

// 多语言词典（编译期嵌入 crate 根 `locales/`），缺省回退简体中文（M11 任务 11.3）。
rust_i18n::i18n!("locales", fallback = "zh-CN");

fn init_tracing() {
    // 默认：应用自身 INFO；屏蔽 gpui Windows 后端的伪错误噪声 ——
    // `gpui_windows::events` 会把 GetLastError==0（"操作成功完成。 0x0"）当成 ERROR 打印；
    // `gpui_windows::window` / `gpui::window` 会在窗口关闭时打印句柄失效日志。均非真实错误。
    // 需要查看完整日志时用 RUST_LOG 覆盖（如 `RUST_LOG=debug`）。
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,gpui_windows::events=off,gpui_windows::window=off,gpui::window=off")
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn main() -> Result<()> {
    init_tracing();

    // 创建 Tokio 多线程运行时，供音频 I/O、网络等耗时任务调度。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("无法创建 Tokio 运行时");
    let _runtime_guard = runtime.enter();
    // 句柄供 GPUI 处理器把网络任务派发到 Tokio 运行时执行（reqwest 需要 reactor）。
    let tokio_handle = runtime.handle().clone();

    tracing::info!("VoxInk 启动中……");

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

    let app = gpui_platform::application().with_assets(Assets);
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
                    titlebar: Some(TitlebarOptions {
                        title: Some("VoxInk".into()),
                        ..Default::default()
                    }),
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

        // 启动最小化到托盘（首次运行仍显示主窗口，便于初次使用）。
        if config.general.start_minimized && !first_run {
            let _ = window.update(cx, |_, win, _| tray::hide_to_tray(win));
            tracing::info!("按配置启动最小化到托盘");
        }
    });

    Ok(())
}
