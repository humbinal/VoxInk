//! 识别记录历史子系统（M10）：单表 records 的 SQLite 存储（§2.8）。
//! 左栏 UI 直接在主视图 `app.rs` 中渲染（2026-06-14 重设计，无独立 panel.rs）。

pub mod db;

/// 以全局形式承载历史数据库句柄，供主视图读写。
/// gpui `Global` 无 `Send`/`Sync` 约束，故持有 `!Sync` 的 rusqlite 连接可行（同 `GlobalTray`）。
pub struct GlobalHistory(pub db::HistoryDb);

impl gpui::Global for GlobalHistory {}
