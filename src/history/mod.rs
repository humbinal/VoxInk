//! 文本历史子系统（M10）：SQLite 存储（§2.8）+ 历史面板 UI。

pub mod db;
pub mod panel;

/// 以全局形式承载历史数据库句柄，供主视图与历史面板共享读写。
/// gpui `Global` 无 `Send`/`Sync` 约束，故持有 `!Sync` 的 rusqlite 连接可行（同 `GlobalTray`）。
pub struct GlobalHistory(pub db::HistoryDb);

impl gpui::Global for GlobalHistory {}
