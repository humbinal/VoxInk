//! 本地历史数据库（M10 任务 10.1）。
//!
//! 📐 表结构契约见 §2.8：`sessions` / `transcriptions` / FTS5 `transcriptions_fts`。
//! 用 `rusqlite`（bundled，编译期含 FTS5）。连接在 GPUI 主线程持有（`!Sync`，但本地
//! SQLite 操作极快，不阻塞 UI；不为此引入后台 DB 线程，避免过度设计）。
//!
//! 📝 落地说明（与 §2.8 的关系）：契约只规定 3 张表。为让外部内容 FTS5 与
//! `transcriptions` 保持同步，需配套 INSERT/DELETE/UPDATE 触发器（实现细节）。
//! 另外 FTS5 默认 unicode61 分词器对中文（无空格）几乎不分词，子串检索无效；故 FTS 表
//! 采用 `tokenize='trigram'`，使中文也能子串全文检索——语义不变（仍是 transcriptions 的全文索引）。

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use directories::BaseDirs;
use rusqlite::{params, Connection};
use serde::Serialize;
use uuid::Uuid;

use crate::session::{SessionRecord, DEFAULT_SESSION_NAME};

/// 转录记录（对应 §2.8 `transcriptions` 表）。
#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionRecord {
    pub id: String,
    pub session_id: String,
    /// "streaming" | "offline" | "local"（§2.8）。
    pub mode: String,
    pub duration_secs: u32,
    pub text: String,
    /// RFC3339 UTC 时间戳。
    pub created_at: String,
}

/// 历史数据库句柄。
pub struct HistoryDb {
    conn: Connection,
}

impl HistoryDb {
    /// 默认路径：`{平台配置目录}/VoxInk/history.db`（与 config.toml 同目录）。
    pub fn default_path() -> Result<PathBuf> {
        let base = BaseDirs::new().context("无法定位用户配置目录")?;
        Ok(base.config_dir().join("VoxInk").join("history.db"))
    }

    /// 在默认路径打开数据库（自动建表）。
    pub fn open() -> Result<Self> {
        let path = Self::default_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建数据目录失败: {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("打开历史数据库失败: {}", path.display()))?;
        Self::from_conn(conn)
    }

    /// 从已有连接构造（供测试用内存库）。
    pub fn from_conn(conn: Connection) -> Result<Self> {
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// 建表 + FTS5 + 同步触发器（§2.8；幂等）。
    fn init_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS sessions (
                    id          TEXT PRIMARY KEY,
                    name        TEXT NOT NULL,
                    created_at  TEXT NOT NULL,
                    updated_at  TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS transcriptions (
                    id            TEXT PRIMARY KEY,
                    session_id    TEXT NOT NULL,
                    mode          TEXT NOT NULL,
                    duration_secs INTEGER NOT NULL,
                    text          TEXT NOT NULL,
                    created_at    TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id)
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS transcriptions_fts
                    USING fts5(text, content=transcriptions, content_rowid=rowid, tokenize='trigram');

                CREATE TRIGGER IF NOT EXISTS transcriptions_ai AFTER INSERT ON transcriptions BEGIN
                    INSERT INTO transcriptions_fts(rowid, text) VALUES (new.rowid, new.text);
                END;
                CREATE TRIGGER IF NOT EXISTS transcriptions_ad AFTER DELETE ON transcriptions BEGIN
                    INSERT INTO transcriptions_fts(transcriptions_fts, rowid, text)
                        VALUES ('delete', old.rowid, old.text);
                END;
                CREATE TRIGGER IF NOT EXISTS transcriptions_au AFTER UPDATE ON transcriptions BEGIN
                    INSERT INTO transcriptions_fts(transcriptions_fts, rowid, text)
                        VALUES ('delete', old.rowid, old.text);
                    INSERT INTO transcriptions_fts(rowid, text) VALUES (new.rowid, new.text);
                END;
                "#,
            )
            .context("初始化历史数据库表结构失败")?;
        Ok(())
    }

    // ───────────────────────────── 会话（任务 10.3）─────────────────────────────

    /// 确保存在默认会话；返回应作为"当前会话"的会话（最近更新的，否则默认）。
    pub fn ensure_default_session(&self) -> Result<SessionRecord> {
        let existing = self.list_sessions()?;
        if let Some(latest) = existing.into_iter().next() {
            return Ok(latest);
        }
        self.create_session(DEFAULT_SESSION_NAME)
    }

    /// 创建命名会话，返回新记录。
    pub fn create_session(&self, name: &str) -> Result<SessionRecord> {
        let now = Utc::now().to_rfc3339();
        let rec = SessionRecord {
            id: new_id(),
            name: name.to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        self.conn
            .execute(
                "INSERT INTO sessions (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![rec.id, rec.name, rec.created_at, rec.updated_at],
            )
            .context("创建会话失败")?;
        Ok(rec)
    }

    /// 列出全部会话，按更新时间倒序（最近的在前）。
    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, created_at, updated_at FROM sessions ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("查询会话列表失败")?;
        Ok(rows)
    }

    /// 删除会话及其全部转录。
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM transcriptions WHERE session_id = ?1",
                params![session_id],
            )
            .context("删除会话转录失败")?;
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
            .context("删除会话失败")?;
        Ok(())
    }

    /// 更新会话 updated_at 为当前时间（新增转录时调用，使其排到最前）。
    pub fn touch_session(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
                params![session_id, Utc::now().to_rfc3339()],
            )
            .context("更新会话时间失败")?;
        Ok(())
    }

    // ───────────────────────────── 转录（任务 10.1）─────────────────────────────

    /// 插入一条转录记录，返回其生成的 id。
    pub fn insert_transcription(
        &self,
        session_id: &str,
        mode: &str,
        duration_secs: u32,
        text: &str,
    ) -> Result<String> {
        let id = new_id();
        let created_at = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO transcriptions (id, session_id, mode, duration_secs, text, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, session_id, mode, duration_secs, text, created_at],
            )
            .context("保存转录记录失败")?;
        Ok(id)
    }

    /// 列出某会话的转录，按创建时间倒序。
    pub fn list_transcriptions(&self, session_id: &str) -> Result<Vec<TranscriptionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, mode, duration_secs, text, created_at
             FROM transcriptions WHERE session_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![session_id], map_transcription)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("查询转录列表失败")?;
        Ok(rows)
    }

    /// 在某会话内全文检索转录（FTS5 trigram）。短于 3 字符的查询退化为 LIKE 子串匹配
    /// （trigram 至少需 3 字符成一组），保证任意长度查询都有结果。
    pub fn search_transcriptions(
        &self,
        session_id: &str,
        query: &str,
    ) -> Result<Vec<TranscriptionRecord>> {
        let q = query.trim();
        if q.is_empty() {
            return self.list_transcriptions(session_id);
        }

        if q.chars().count() < 3 {
            let like = format!("%{}%", escape_like(q));
            let mut stmt = self.conn.prepare(
                "SELECT id, session_id, mode, duration_secs, text, created_at
                 FROM transcriptions
                 WHERE session_id = ?1 AND text LIKE ?2 ESCAPE '\\'
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map(params![session_id, like], map_transcription)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("搜索转录失败")?;
            return Ok(rows);
        }

        // 将用户输入整体作为一个字符串字面量（转义内部双引号），避免 FTS5 查询语法报错。
        let match_query = format!("\"{}\"", q.replace('"', "\"\""));
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.session_id, t.mode, t.duration_secs, t.text, t.created_at
             FROM transcriptions_fts
             JOIN transcriptions t ON t.rowid = transcriptions_fts.rowid
             WHERE transcriptions_fts MATCH ?1 AND t.session_id = ?2
             ORDER BY t.created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![match_query, session_id], map_transcription)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("全文检索转录失败")?;
        Ok(rows)
    }

    /// 删除单条转录。
    pub fn delete_transcription(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM transcriptions WHERE id = ?1", params![id])
            .context("删除转录失败")?;
        Ok(())
    }

    /// 清空某会话的全部转录。
    pub fn clear_session_transcriptions(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM transcriptions WHERE session_id = ?1",
                params![session_id],
            )
            .context("清空会话转录失败")?;
        Ok(())
    }

    // ───────────────────────────── 保留与导出（任务 10.4）─────────────────────────────

    /// 删除早于 `days` 天的转录；返回删除条数。`days == 0` 表示不清理。
    pub fn purge_older_than(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        let n = self
            .conn
            .execute(
                "DELETE FROM transcriptions WHERE created_at < ?1",
                params![cutoff],
            )
            .context("清理过期历史失败")?;
        Ok(n)
    }

    /// 导出全部会话及其转录为 JSON 值（任务 10.4）。
    pub fn export_json(&self) -> Result<serde_json::Value> {
        let sessions = self.list_sessions()?;
        let mut out = Vec::with_capacity(sessions.len());
        for s in sessions {
            let transcriptions = self.list_transcriptions(&s.id)?;
            out.push(serde_json::json!({
                "id": s.id,
                "name": s.name,
                "created_at": s.created_at,
                "updated_at": s.updated_at,
                "transcriptions": transcriptions,
            }));
        }
        Ok(serde_json::json!({
            "exported_at": Utc::now().to_rfc3339(),
            "sessions": out,
        }))
    }
}

fn map_transcription(row: &rusqlite::Row) -> rusqlite::Result<TranscriptionRecord> {
    Ok(TranscriptionRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        mode: row.get(2)?,
        duration_secs: row.get::<_, i64>(3)? as u32,
        text: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// 转义 LIKE 通配符，配合 `ESCAPE '\'`。
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> HistoryDb {
        HistoryDb::from_conn(Connection::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn session_crud_and_default() {
        let db = mem_db();
        let s = db.ensure_default_session().unwrap();
        assert_eq!(s.name, DEFAULT_SESSION_NAME);
        // ensure_default 再次调用返回已存在的会话，不重复创建。
        let again = db.ensure_default_session().unwrap();
        assert_eq!(again.id, s.id);
        assert_eq!(db.list_sessions().unwrap().len(), 1);

        let s2 = db.create_session("工作").unwrap();
        assert_eq!(db.list_sessions().unwrap().len(), 2);
        db.delete_session(&s2.id).unwrap();
        assert_eq!(db.list_sessions().unwrap().len(), 1);
    }

    #[test]
    fn insert_list_delete_transcription() {
        let db = mem_db();
        let s = db.ensure_default_session().unwrap();
        db.insert_transcription(&s.id, "offline", 12, "你好世界")
            .unwrap();
        let id2 = db
            .insert_transcription(&s.id, "streaming", 5, "测试录音")
            .unwrap();
        let list = db.list_transcriptions(&s.id).unwrap();
        assert_eq!(list.len(), 2);
        db.delete_transcription(&id2).unwrap();
        assert_eq!(db.list_transcriptions(&s.id).unwrap().len(), 1);
        db.clear_session_transcriptions(&s.id).unwrap();
        assert!(db.list_transcriptions(&s.id).unwrap().is_empty());
    }

    #[test]
    fn fts_search_chinese_substring() {
        let db = mem_db();
        let s = db.ensure_default_session().unwrap();
        db.insert_transcription(&s.id, "offline", 1, "帮我写一段提示词")
            .unwrap();
        db.insert_transcription(&s.id, "offline", 1, "今天天气不错")
            .unwrap();
        // trigram 子串检索（>=3 字符）。
        let hits = db.search_transcriptions(&s.id, "提示词").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("提示词"));
        // 短查询（<3 字符）走 LIKE。
        let hits2 = db.search_transcriptions(&s.id, "天气").unwrap();
        assert_eq!(hits2.len(), 1);
        // 空查询返回全部。
        assert_eq!(db.search_transcriptions(&s.id, "  ").unwrap().len(), 2);
    }

    #[test]
    fn purge_and_export() {
        let db = mem_db();
        let s = db.ensure_default_session().unwrap();
        db.insert_transcription(&s.id, "offline", 1, "内容").unwrap();
        // days=0 不清理。
        assert_eq!(db.purge_older_than(0).unwrap(), 0);
        // 未来 0 天前的不会被删（记录是刚插入的）。
        assert_eq!(db.purge_older_than(30).unwrap(), 0);

        let json = db.export_json().unwrap();
        assert!(json["sessions"].is_array());
        assert_eq!(json["sessions"][0]["transcriptions"][0]["text"], "内容");
    }
}
