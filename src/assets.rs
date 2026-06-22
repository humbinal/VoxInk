//! 资源源（2026-06-16 界面美化）：在 gpui-component 内置图标之上叠加 VoxInk 自有图标。
//!
//! gpui-component 的 `IconName` 仅覆盖其内置 SVG 集（无麦克风图标）。这里实现一个组合
//! [`AssetSource`]：优先返回项目自带图标（编译期内嵌），其余路径回退到内置资源。
//! 通过 `Icon::empty().path("icons/mic.svg")` 即可使用自有图标。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component_assets::Assets as ComponentAssets;

/// VoxInk 自有麦克风图标（录音按钮 / 迷你条指示用）。
const MIC_SVG: &[u8] = include_bytes!("../assets/icons/mic.svg");
/// VoxInk 自有停止图标（片段回放停止按钮用，内置图标集无 stop/square）。
const STOP_SVG: &[u8] = include_bytes!("../assets/icons/stop.svg");
/// 主界面标题栏品牌 logo（编译期由 build.rs 程序化渲染，与 exe/托盘/任务栏图标同源）。
const LOGO_PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/voxink_logo.png"));

/// 组合资源源：项目图标优先，其余回退 gpui-component 内置资源。
pub struct VoxInkAssets;

impl AssetSource for VoxInkAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "icons/mic.svg" => Ok(Some(Cow::Borrowed(MIC_SVG))),
            "icons/stop.svg" => Ok(Some(Cow::Borrowed(STOP_SVG))),
            "icons/logo.png" => Ok(Some(Cow::Borrowed(LOGO_PNG))),
            _ => ComponentAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        ComponentAssets.list(path)
    }
}
