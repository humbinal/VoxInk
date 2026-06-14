//! 会话领域类型（M10 任务 10.3）。
//!
//! 会话用于把多次转录归组到独立的文本上下文（§4.3.3）。会话的持久化 CRUD 在
//! [`crate::history::db`]；这里只放领域类型与常量，供 DB 层与历史面板 UI 共用。

use serde::Serialize;

/// 首次启动自动创建的默认会话名称。
pub const DEFAULT_SESSION_NAME: &str = "默认会话";

/// 会话记录（对应 §2.8 `sessions` 表）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    /// RFC3339 UTC 时间戳。
    pub created_at: String,
    pub updated_at: String,
}
