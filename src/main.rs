//! VoxInk 应用入口 —— M1 任务 1.3。
//!
//! 职责：初始化日志、创建 Tokio 运行时、启动 GPUI 应用并打开主窗口（480×600）。

mod app;
mod state;

use anyhow::Result;
use app::VoxInk;
use gpui::{Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size};
use gpui_component::Root;
use gpui_component_assets::Assets;
use tracing_subscriber::EnvFilter;

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

    // 创建 Tokio 多线程运行时，供后续里程碑（音频 I/O、网络、本地推理）调度耗时任务。
    // 进入运行时上下文后 `tokio::spawn` 可在 GPUI 主线程触发的回调中使用。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .build()
        .expect("无法创建 Tokio 运行时");
    let _runtime_guard = runtime.enter();

    tracing::info!("VoxInk 启动中……");

    let app = gpui_platform::application().with_assets(Assets);
    app.run(|cx| {
        // 初始化 gpui-component（主题、输入、菜单等子系统）。
        gpui_component::init(cx);

        // 将 gpui-component 内置文案设为简体中文（右键菜单剪切/复制/粘贴、对话框按钮等）。
        // 其 locale 默认 "en" 且与系统语言无关；M11 将改为跟随配置项 general.language。
        gpui_component::set_locale("zh-CN");

        let bounds = Bounds::centered(None, size(px(480.), px(600.)), cx);
        cx.open_window(
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
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("无法创建主窗口");

        cx.activate(true);
        tracing::info!("主窗口已打开");
    });

    Ok(())
}
