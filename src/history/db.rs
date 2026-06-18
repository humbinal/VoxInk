//! 本地历史数据库（M10，2026-06-14 重设计为单表 records 文档模型）。
//!
//! 📐 表结构契约见 §2.8：单表 `records`（每条即左栏一个可编辑/可续录文档）+ FTS5 `records_fts`。
//! 用 `rusqlite`（bundled，编译期含 FTS5）。连接在 GPUI 主线程持有（`!Sync`，本地 SQLite 操作
//! 极快不阻塞 UI；不为此引入后台 DB 线程，避免过度设计）。
//!
//! 📝 FTS5 落地：外部内容 FTS5 需配套 INSERT/DELETE/UPDATE 触发器与 `records` 同步；并用
//! `tokenize='trigram'`，否则默认 unicode61 对无空格中文几乎不分词、子串检索失效。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use directories::BaseDirs;
use rusqlite::{Connection, params};
use serde::Serialize;
use uuid::Uuid;

/// 新建空记录的默认标题。
pub const NEW_RECORD_TITLE: &str = "新记录";

/// 由正文派生标题时保留的最大字符数（超出加省略号）。
const TITLE_MAX_CHARS: usize = 50;

/// 一条识别记录文档（对应 §2.8 `records` 表）。
#[derive(Debug, Clone, Serialize)]
pub struct Record {
    pub id: String,
    pub title: String,
    pub text: String,
    pub duration_secs: u32,
    /// RFC3339 UTC。
    pub created_at: String,
    pub updated_at: String,
}

/// 一段录音片段（对应 §2.8 `segments` 表，2026-06-16）。
/// 一条 [`Record`] 可挂多段（多次续录）；`file_path` 存**绝对路径**，
/// 故更改音频根目录不影响已有片段（§4.2.2 决策）。
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub id: String,
    pub record_id: String,
    /// 音频文件绝对路径。
    pub file_path: String,
    /// 录制模式 "streaming" | "offline"。
    pub mode: String,
    /// 该段产生的转写文本（用于音频↔文字对照、重转写）。
    pub text: String,
    pub duration_secs: u32,
    pub byte_size: u64,
    /// RFC3339 UTC。
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
        // 开启外键约束（rusqlite 连接默认关闭）：删 record 级联删其 segments 行。
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("启用外键约束失败")?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// 建表 + FTS5 + 同步触发器（§2.8；幂等）。
    fn init_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS records (
                    id            TEXT PRIMARY KEY,
                    title         TEXT NOT NULL,
                    text          TEXT NOT NULL,
                    duration_secs INTEGER NOT NULL,
                    created_at    TEXT NOT NULL,
                    updated_at    TEXT NOT NULL
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS records_fts
                    USING fts5(text, content=records, content_rowid=rowid, tokenize='trigram');

                CREATE TRIGGER IF NOT EXISTS records_ai AFTER INSERT ON records BEGIN
                    INSERT INTO records_fts(rowid, text) VALUES (new.rowid, new.text);
                END;
                CREATE TRIGGER IF NOT EXISTS records_ad AFTER DELETE ON records BEGIN
                    INSERT INTO records_fts(records_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
                END;
                CREATE TRIGGER IF NOT EXISTS records_au AFTER UPDATE ON records BEGIN
                    INSERT INTO records_fts(records_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
                    INSERT INTO records_fts(rowid, text) VALUES (new.rowid, new.text);
                END;

                -- 录音片段：一条 record 可挂多段（多次续录）。删 record 级联删本表行（需外键开启）。
                CREATE TABLE IF NOT EXISTS segments (
                    id            TEXT PRIMARY KEY,
                    record_id     TEXT NOT NULL,
                    file_path     TEXT NOT NULL,
                    mode          TEXT NOT NULL,
                    text          TEXT NOT NULL DEFAULT '',
                    duration_secs INTEGER NOT NULL DEFAULT 0,
                    byte_size     INTEGER NOT NULL DEFAULT 0,
                    created_at    TEXT NOT NULL,
                    FOREIGN KEY(record_id) REFERENCES records(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_segments_record ON segments(record_id, created_at);
                "#,
            )
            .context("初始化历史数据库表结构失败")?;

        // 迁移：移除历史遗留的 records.mode 列（不再使用；录制模式仍由各 segment 自行记录）。
        // 老库该列为 NOT NULL 且无默认值，若保留会让省略它的 INSERT 失败，故主动删除。
        let has_mode = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('records') WHERE name = 'mode'")?
            .exists([])?;
        if has_mode {
            self.conn
                .execute_batch("ALTER TABLE records DROP COLUMN mode;")
                .context("移除历史遗留的 records.mode 列失败")?;
        }
        Ok(())
    }

    // ───────────────────────────── 记录 CRUD ─────────────────────────────

    /// 新建一条空记录文档，返回其记录。
    pub fn create_record(&self) -> Result<Record> {
        let now = Utc::now().to_rfc3339();
        let rec = Record {
            id: Uuid::new_v4().to_string(),
            title: NEW_RECORD_TITLE.to_string(),
            text: String::new(),
            duration_secs: 0,
            created_at: now.clone(),
            updated_at: now,
        };
        self.conn
            .execute(
                "INSERT INTO records (id, title, text, duration_secs, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    rec.id,
                    rec.title,
                    rec.text,
                    rec.duration_secs,
                    rec.created_at,
                    rec.updated_at
                ],
            )
            .context("创建记录失败")?;
        Ok(rec)
    }

    /// 保存记录正文（更新 text/title/duration/updated_at）。
    /// 标题由正文首行派生；空正文回退到 [`NEW_RECORD_TITLE`]。
    pub fn save_record(&self, id: &str, text: &str, duration_secs: u32) -> Result<()> {
        let title = derive_title(text);
        self.conn
            .execute(
                "UPDATE records SET text = ?2, title = ?3, duration_secs = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![id, text, title, duration_secs, Utc::now().to_rfc3339()],
            )
            .context("保存记录失败")?;
        Ok(())
    }

    /// 读取单条记录。
    pub fn get_record(&self, id: &str) -> Result<Option<Record>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, text, duration_secs, created_at, updated_at
             FROM records WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], map_record)?;
        match rows.next() {
            Some(r) => Ok(Some(r.context("读取记录失败")?)),
            None => Ok(None),
        }
    }

    /// 列出全部记录，按 `created_at` 倒序（最新创建的在前；顺序稳定，不因编辑/续录而跳变）。
    pub fn list_records(&self) -> Result<Vec<Record>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, text, duration_secs, created_at, updated_at
             FROM records ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], map_record)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("查询记录列表失败")?;
        Ok(rows)
    }

    /// 最近一条记录（用于启动默认打开）。
    pub fn most_recent(&self) -> Result<Option<Record>> {
        Ok(self.list_records()?.into_iter().next())
    }

    /// 删除单条记录。
    pub fn delete_record(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM records WHERE id = ?1", params![id])
            .context("删除记录失败")?;
        Ok(())
    }

    /// 全文检索记录（FTS5 trigram）。短于 3 字符的查询退化为 LIKE 子串匹配
    /// （trigram 至少需 3 字符成一组）。空查询返回全部。结果按 `created_at` 倒序。
    pub fn search_records(&self, query: &str) -> Result<Vec<Record>> {
        let q = query.trim();
        if q.is_empty() {
            return self.list_records();
        }

        if q.chars().count() < 3 {
            let like = format!("%{}%", escape_like(q));
            let mut stmt = self.conn.prepare(
                "SELECT id, title, text, duration_secs, created_at, updated_at
                 FROM records WHERE text LIKE ?1 ESCAPE '\\' ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map(params![like], map_record)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("搜索记录失败")?;
            return Ok(rows);
        }

        // 将用户输入整体作为一个字符串字面量（转义内部双引号），避免 FTS5 查询语法报错。
        let match_query = format!("\"{}\"", q.replace('"', "\"\""));
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.title, r.text, r.duration_secs, r.created_at, r.updated_at
             FROM records_fts
             JOIN records r ON r.rowid = records_fts.rowid
             WHERE records_fts MATCH ?1
             ORDER BY r.created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![match_query], map_record)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("全文检索记录失败")?;
        Ok(rows)
    }

    // ───────────────────────────── 保留与导出（任务 10.4）─────────────────────────────

    /// 删除 `updated_at` 早于 `days` 天的记录；返回删除条数。`days == 0` 表示不清理。
    pub fn purge_older_than(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        let n = self
            .conn
            .execute("DELETE FROM records WHERE updated_at < ?1", params![cutoff])
            .context("清理过期记录失败")?;
        Ok(n)
    }

    // ───────────────────────────── 录音片段（segments，2026-06-16）─────────────────────────────

    /// 新增一段录音片段，返回其记录。
    pub fn add_segment(
        &self,
        record_id: &str,
        file_path: &Path,
        mode: &str,
        text: &str,
        duration_secs: u32,
        byte_size: u64,
    ) -> Result<Segment> {
        let seg = Segment {
            id: Uuid::new_v4().to_string(),
            record_id: record_id.to_string(),
            file_path: file_path.to_string_lossy().to_string(),
            mode: mode.to_string(),
            text: text.to_string(),
            duration_secs,
            byte_size,
            created_at: Utc::now().to_rfc3339(),
        };
        self.conn
            .execute(
                "INSERT INTO segments (id, record_id, file_path, mode, text, duration_secs, byte_size, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    seg.id,
                    seg.record_id,
                    seg.file_path,
                    seg.mode,
                    seg.text,
                    seg.duration_secs,
                    seg.byte_size as i64,
                    seg.created_at,
                ],
            )
            .context("新增录音片段失败")?;
        Ok(seg)
    }

    /// 列出某记录的全部片段（按创建时间升序）。
    pub fn list_segments(&self, record_id: &str) -> Result<Vec<Segment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, record_id, file_path, mode, text, duration_secs, byte_size, created_at
             FROM segments WHERE record_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![record_id], map_segment)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("查询录音片段失败")?;
        Ok(rows)
    }

    /// 某记录全部片段的音频文件路径（删除记录前取出，供应用层删文件）。
    pub fn audio_paths_for_record(&self, record_id: &str) -> Result<Vec<PathBuf>> {
        Ok(self
            .list_segments(record_id)?
            .into_iter()
            .map(|s| PathBuf::from(s.file_path))
            .collect())
    }

    /// 删除单段片段，返回其音频文件路径（供应用层删文件）。
    pub fn delete_segment(&self, id: &str) -> Result<Option<PathBuf>> {
        let path: Option<String> = self
            .conn
            .query_row(
                "SELECT file_path FROM segments WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .ok();
        self.conn
            .execute("DELETE FROM segments WHERE id = ?1", params![id])
            .context("删除片段失败")?;
        Ok(path.map(PathBuf::from))
    }

    /// 重转写后回填某段的文本与转写模式（圆点据此变色）。
    /// 重转写固定走离线后端，故 `mode` 由调用方传 `"offline"`。
    pub fn update_segment_transcription(&self, id: &str, text: &str, mode: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE segments SET text = ?2, mode = ?3 WHERE id = ?1",
                params![id, text, mode],
            )
            .context("更新片段文本失败")?;
        Ok(())
    }

    /// 全部片段的音频文件路径（用于启动时孤儿文件对账）。
    pub fn all_segment_paths(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT file_path FROM segments")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("查询片段路径失败")?;
        Ok(rows.into_iter().map(PathBuf::from).collect())
    }

    /// 删除 `created_at` 早于 `days` 天的片段行，返回被删片段的音频文件路径（供应用层删文件）。
    /// `days == 0` 表示不清理。仅删音频（保留 record 文本）。
    pub fn purge_audio_older_than(&self, days: u32) -> Result<Vec<PathBuf>> {
        if days == 0 {
            return Ok(Vec::new());
        }
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        let paths: Vec<PathBuf> = {
            let mut stmt = self
                .conn
                .prepare("SELECT file_path FROM segments WHERE created_at < ?1")?;
            stmt.query_map(params![cutoff], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("查询过期片段失败")?
                .into_iter()
                .map(PathBuf::from)
                .collect()
        };
        self.conn
            .execute(
                "DELETE FROM segments WHERE created_at < ?1",
                params![cutoff],
            )
            .context("清理过期片段失败")?;
        Ok(paths)
    }

    /// 导出全部记录为 JSON 值（任务 10.4）。
    pub fn export_json(&self) -> Result<serde_json::Value> {
        let records = self.list_records()?;
        Ok(serde_json::json!({
            "exported_at": Utc::now().to_rfc3339(),
            "records": records,
        }))
    }
}

fn map_record(row: &rusqlite::Row) -> rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        title: row.get(1)?,
        text: row.get(2)?,
        duration_secs: row.get::<_, i64>(3)? as u32,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

/// 8 位短随机串（取 UUID v4 前 8 个十六进制位），用于录音文件名去重。
pub fn short_uuid() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn map_segment(row: &rusqlite::Row) -> rusqlite::Result<Segment> {
    Ok(Segment {
        id: row.get(0)?,
        record_id: row.get(1)?,
        file_path: row.get(2)?,
        mode: row.get(3)?,
        text: row.get(4)?,
        duration_secs: row.get::<_, i64>(5)? as u32,
        byte_size: row.get::<_, i64>(6)? as u64,
        created_at: row.get(7)?,
    })
}

/// 由正文派生标题：折叠换行/连续空白为单空格，取开头 [`TITLE_MAX_CHARS`] 个字符（超出加省略号）；
/// 空正文回退默认标题。
fn derive_title(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return NEW_RECORD_TITLE.to_string();
    }
    let title: String = flat.chars().take(TITLE_MAX_CHARS).collect();
    if flat.chars().count() > TITLE_MAX_CHARS {
        format!("{title}…")
    } else {
        title
    }
}

/// 转义 LIKE 通配符，配合 `ESCAPE '\'`。
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> HistoryDb {
        HistoryDb::from_conn(Connection::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn create_save_get_record() {
        let db = mem_db();
        let rec = db.create_record().unwrap();
        assert_eq!(rec.title, NEW_RECORD_TITLE);
        assert!(rec.text.is_empty());

        db.save_record(&rec.id, "帮我写一段周报\n第二行", 12)
            .unwrap();
        let got = db.get_record(&rec.id).unwrap().unwrap();
        assert_eq!(got.text, "帮我写一段周报\n第二行");
        assert_eq!(got.title, "帮我写一段周报 第二行"); // 折叠换行后取开头 50 字
        assert_eq!(got.duration_secs, 12);
    }

    #[test]
    fn migrates_old_db_dropping_mode_column() {
        // 模拟旧库：records 含历史遗留的 NOT NULL `mode` 列且已有数据。
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE records (
                id            TEXT PRIMARY KEY,
                title         TEXT NOT NULL,
                text          TEXT NOT NULL,
                mode          TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                created_at    TEXT NOT NULL,
                updated_at    TEXT NOT NULL
            );
            INSERT INTO records (id, title, text, mode, duration_secs, created_at, updated_at)
            VALUES ('r1', '旧标题', '旧正文', 'streaming', 7,
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
            "#,
        )
        .unwrap();

        // from_conn → init_schema 触发迁移，删除 mode 列。
        let db = HistoryDb::from_conn(conn).unwrap();

        // 旧数据保留且可正常读取。
        let got = db.get_record("r1").unwrap().unwrap();
        assert_eq!(got.text, "旧正文");
        assert_eq!(got.duration_secs, 7);

        // mode 列已不存在。
        let has_mode = db
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('records') WHERE name = 'mode'")
            .unwrap()
            .exists([])
            .unwrap();
        assert!(!has_mode);

        // 迁移后仍可写入：省略原 NOT NULL `mode` 列的 INSERT 不再失败。
        let rec = db.create_record().unwrap();
        db.save_record(&rec.id, "新内容", 3).unwrap();
        assert_eq!(db.list_records().unwrap().len(), 2);
    }

    #[test]
    fn title_capped_at_50_chars() {
        let db = mem_db();
        let r = db.create_record().unwrap();
        let long: String = "字".repeat(60);
        db.save_record(&r.id, &long, 0).unwrap();
        let title = db.get_record(&r.id).unwrap().unwrap().title;
        assert_eq!(title.chars().count(), TITLE_MAX_CHARS + 1); // 50 + 省略号
        assert!(title.ends_with('…'));
    }

    #[test]
    fn list_orders_by_created_desc_stable() {
        let db = mem_db();
        let a = db.create_record().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = db.create_record().unwrap();

        // 按 created_at 倒序：后建的 b 在前。
        let list = db.list_records().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
        assert_eq!(db.most_recent().unwrap().unwrap().id, b.id);

        // 编辑 a（更新 updated_at）不应改变顺序——仍按 created_at 排。
        std::thread::sleep(std::time::Duration::from_millis(5));
        db.save_record(&a.id, "later edit", 0).unwrap();
        let list2 = db.list_records().unwrap();
        assert_eq!(list2[0].id, b.id);
        assert_eq!(list2[1].id, a.id);
    }

    #[test]
    fn delete_record_works() {
        let db = mem_db();
        let a = db.create_record().unwrap();
        let b = db.create_record().unwrap();
        db.delete_record(&a.id).unwrap();
        assert!(db.get_record(&a.id).unwrap().is_none());
        assert_eq!(db.list_records().unwrap().len(), 1);
        assert_eq!(db.list_records().unwrap()[0].id, b.id);
    }

    #[test]
    fn fts_search_chinese_substring() {
        let db = mem_db();
        let a = db.create_record().unwrap();
        let b = db.create_record().unwrap();
        db.save_record(&a.id, "帮我写一段提示词", 1).unwrap();
        db.save_record(&b.id, "今天天气不错", 1).unwrap();

        let hits = db.search_records("提示词").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("提示词"));
        // 短查询（<3 字符）走 LIKE。
        assert_eq!(db.search_records("天气").unwrap().len(), 1);
        // 空查询返回全部。
        assert_eq!(db.search_records("  ").unwrap().len(), 2);
    }

    #[test]
    fn segments_crud_and_cascade() {
        let db = mem_db();
        let r = db.create_record().unwrap();
        let p1 = std::path::Path::new("/tmp/voxink/r1/a.wav");
        let p2 = std::path::Path::new("/tmp/voxink/r1/b.wav");
        db.add_segment(&r.id, p1, "offline", "第一段", 5, 100)
            .unwrap();
        db.add_segment(&r.id, p2, "streaming", "第二段", 7, 200)
            .unwrap();

        let segs = db.list_segments(&r.id).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "第一段"); // created_at 升序
        assert_eq!(segs[1].byte_size, 200);

        let paths = db.audio_paths_for_record(&r.id).unwrap();
        assert_eq!(paths.len(), 2);

        // 删 record → 外键级联删 segments 行。
        db.delete_record(&r.id).unwrap();
        assert_eq!(db.list_segments(&r.id).unwrap().len(), 0);
    }

    #[test]
    fn purge_audio_returns_paths() {
        let db = mem_db();
        let r = db.create_record().unwrap();
        db.add_segment(
            &r.id,
            std::path::Path::new("/tmp/x.wav"),
            "offline",
            "",
            1,
            1,
        )
        .unwrap();
        // days=0 不清理；刚建的片段在 30 天内也不过期。
        assert_eq!(db.purge_audio_older_than(0).unwrap().len(), 0);
        assert_eq!(db.purge_audio_older_than(30).unwrap().len(), 0);
        assert_eq!(db.list_segments(&r.id).unwrap().len(), 1);
    }

    #[test]
    fn purge_and_export() {
        let db = mem_db();
        let a = db.create_record().unwrap();
        db.save_record(&a.id, "内容", 1).unwrap();
        assert_eq!(db.purge_older_than(0).unwrap(), 0); // 不清理
        assert_eq!(db.purge_older_than(30).unwrap(), 0); // 刚更新的不过期

        let json = db.export_json().unwrap();
        assert!(json["records"].is_array());
        assert_eq!(json["records"][0]["text"], "内容");
    }
}
