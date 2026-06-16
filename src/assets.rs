//! 资源源（2026-06-16 界面美化）：在 gpui-component 内置图标之上叠加 VoxInk 自有图标。
//!
//! gpui-component 的 `IconName` 仅覆盖其内置 SVG 集（无麦克风图标）。这里实现一个组合
//! [`AssetSource`]：优先返回项目自带图标（编译期内嵌），其余路径回退到内置资源。
//! 通过 `Icon::empty().path("icons/mic.svg")` 即可使用自有图标。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};
use gpui_component_assets::Assets as ComponentAssets;

/// VoxInk 自有麦克风图标（品牌徽标用）。
const MIC_SVG: &[u8] = include_bytes!("../assets/icons/mic.svg");

/// 组合资源源：项目图标优先，其余回退 gpui-component 内置资源。
pub struct VoxInkAssets;

impl AssetSource for VoxInkAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "icons/mic.svg" => Ok(Some(Cow::Borrowed(MIC_SVG))),
            _ => ComponentAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        ComponentAssets.list(path)
    }
}
