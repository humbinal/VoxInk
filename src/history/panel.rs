//! 历史面板 UI（M10 任务 10.2/10.3）。
//!
//! 作为子视图渲染在 gpui-component 的 Sheet（右侧抽屉）中：会话切换/新建/删除、
//! 搜索（实时，FTS5）、历史列表（时间 + 模式图标 + 预览）、点击载入编辑器、删除单条/清空、
//! 导出 JSON。数据库经 [`crate::history::GlobalHistory`] 访问；载入文本经持有的主视图回写。

use chrono::{DateTime, Local};
use gpui::{
    div, prelude::*, px, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription,
    Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    h_flex, v_flex, ActiveTheme, WindowExt,
};

use crate::app::VoxInk;
use crate::history::db::TranscriptionRecord;
use crate::history::GlobalHistory;
use crate::session::SessionRecord;

pub struct HistoryPanel {
    /// 主视图：用于把历史文本载入编辑器、读取/切换当前会话。
    main: Entity<VoxInk>,
    /// 搜索输入（实时过滤）。
    search: Entity<InputState>,
    /// 新建会话名称输入。
    new_session: Entity<InputState>,
    sessions: Vec<SessionRecord>,
    records: Vec<TranscriptionRecord>,
    /// 当前会话 id 的本地镜像。**不**在此处 `read` 主视图取它——`refresh` 可能在主视图
    /// 正被 update 时被调用（如主界面点「历史」按钮的 listener 内），读主视图会触发双重租借
    /// panic。改由主视图在调用 `refresh_for` 时把 id 传入。
    current_session_id: String,
    _subs: Vec<Subscription>,
}

impl HistoryPanel {
    pub fn new(main: Entity<VoxInk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索历史…"));
        let new_session = cx.new(|cx| InputState::new(window, cx).placeholder("新会话名称…"));

        // 搜索内容变化即重新查询（FTS5）。
        let sub = cx.subscribe(&search, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
            }
        });

        Self {
            main,
            search,
            new_session,
            sessions: Vec::new(),
            records: Vec::new(),
            current_session_id: String::new(),
            _subs: vec![sub],
        }
    }

    /// 设置当前会话 id 后刷新（供主视图调用：主视图是 current_session_id 的真源）。
    pub fn refresh_for(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.current_session_id = session_id;
        self.refresh(cx);
    }

    /// 从数据库重新载入会话列表与（当前会话、当前搜索词下的）转录列表。
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let session_id = self.current_session_id.clone();
        let query = self.search.read(cx).value().to_string();

        let Some(global) = cx.try_global::<GlobalHistory>() else {
            return;
        };
        let db = &global.0;
        let sessions = db.list_sessions().unwrap_or_default();
        let records = db
            .search_transcriptions(&session_id, &query)
            .unwrap_or_default();

        self.sessions = sessions;
        self.records = records;
        cx.notify();
    }

    fn on_switch_session(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.main.update(cx, |main, mcx| {
            main.switch_session(id.clone(), window, mcx);
        });
        self.current_session_id = id;
        self.refresh(cx);
    }

    fn on_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.new_session.read(cx).value().trim().to_string();
        let name = if typed.is_empty() {
            format!("会话 {}", Local::now().format("%m-%d %H:%M"))
        } else {
            typed
        };

        let new_id = {
            let Some(global) = cx.try_global::<GlobalHistory>() else {
                return;
            };
            match global.0.create_session(&name) {
                Ok(rec) => rec.id,
                Err(e) => {
                    tracing::error!("创建会话失败: {e:#}");
                    window.push_notification("创建会话失败", cx);
                    return;
                }
            }
        };

        self.new_session.update(cx, |s, cx| s.set_value("", window, cx));
        self.on_switch_session(new_id, window, cx);
        window.push_notification(format!("已创建会话「{name}」"), cx);
    }

    fn on_delete_current_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_session_id.clone();

        // 删除当前会话后需切到另一个会话（不存在则新建默认）。
        let next_id = {
            let Some(global) = cx.try_global::<GlobalHistory>() else {
                return;
            };
            let db = &global.0;
            if let Err(e) = db.delete_session(&current) {
                tracing::error!("删除会话失败: {e:#}");
                window.push_notification("删除会话失败", cx);
                return;
            }
            match db.ensure_default_session() {
                Ok(rec) => rec.id,
                Err(e) => {
                    tracing::error!("获取默认会话失败: {e:#}");
                    return;
                }
            }
        };

        self.on_switch_session(next_id, window, cx);
        window.push_notification("已删除当前会话", cx);
    }

    fn on_load_record(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.main.update(cx, |main, mcx| {
            main.load_history_text(&text, window, mcx);
        });
        window.close_sheet(cx);
        window.push_notification("已载入到编辑器", cx);
    }

    fn on_delete_record(&mut self, id: String, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(global) = cx.try_global::<GlobalHistory>()
            && let Err(e) = global.0.delete_transcription(&id)
        {
            tracing::error!("删除转录失败: {e:#}");
        }
        self.refresh(cx);
    }

    fn on_clear_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_session_id.clone();
        if let Some(global) = cx.try_global::<GlobalHistory>()
            && let Err(e) = global.0.clear_session_transcriptions(&current)
        {
            tracing::error!("清空会话失败: {e:#}");
        }
        self.refresh(cx);
        window.push_notification("已清空当前会话历史", cx);
    }

    fn on_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<std::path::PathBuf> {
            let global = cx
                .try_global::<GlobalHistory>()
                .ok_or_else(|| anyhow::anyhow!("历史数据库不可用"))?;
            let json = global.0.export_json()?;
            let dir = crate::history::db::HistoryDb::default_path()?
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            let path = dir.join(format!(
                "history_export_{}.json",
                Local::now().format("%Y%m%d_%H%M%S")
            ));
            std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
            Ok(path)
        })();

        match result {
            Ok(path) => {
                tracing::info!("历史已导出: {}", path.display());
                window.push_notification(format!("已导出到 {}", path.display()), cx);
            }
            Err(e) => {
                tracing::error!("导出历史失败: {e:#}");
                window.push_notification("导出失败", cx);
            }
        }
    }

    fn render_sessions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current_session_id.clone();
        let mut row = h_flex().gap_2().flex_wrap();
        for s in self.sessions.clone() {
            let is_current = s.id == current;
            let id = s.id.clone();
            row = row.child(
                Button::new(elem_id("session", &s.id))
                    .when(is_current, |b| b.primary())
                    .when(!is_current, |b| b.outline())
                    .label(s.name.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.on_switch_session(id.clone(), window, cx)
                    })),
            );
        }
        row
    }

    fn render_record(&self, rec: &TranscriptionRecord, cx: &mut Context<Self>) -> impl IntoElement {
        let text = rec.text.clone();
        let id = rec.id.clone();

        h_flex()
            .w_full()
            .gap_2()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().border)
            .items_start()
            .child(
                // 点击主体载入编辑器。
                div()
                    .id(elem_id("record", &rec.id))
                    .flex_1()
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.85))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.on_load_record(text.clone(), window, cx)
                    }))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(mode_icon(&rec.mode))
                                    .child(format_time(&rec.created_at))
                                    .child(format!("{}s", rec.duration_secs)),
                            )
                            .child(div().text_sm().child(preview(&rec.text))),
                    ),
            )
            .child(
                Button::new(elem_id("del", &rec.id))
                    .ghost()
                    .label("✕")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.on_delete_record(id.clone(), window, cx)
                    })),
            )
    }
}

impl Render for HistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let records = self.records.clone();

        v_flex()
            .size_full()
            .gap_3()
            // 会话区：切换 + 新建 + 删除当前。
            .child(self.render_sessions(cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.new_session)))
                    .child(
                        Button::new("new-session")
                            .label("＋ 新建")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new_session(window, cx)
                            })),
                    )
                    .child(
                        Button::new("del-session")
                            .danger()
                            .label("🗑")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_delete_current_session(window, cx)
                            })),
                    ),
            )
            // 搜索。
            .child(Input::new(&self.search))
            // 操作：导出 / 清空当前会话。
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("export")
                            .label("⬇ 导出 JSON")
                            .on_click(cx.listener(|this, _, window, cx| this.on_export(window, cx))),
                    )
                    .child(
                        Button::new("clear-session")
                            .danger()
                            .label("清空本会话")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_clear_session(window, cx)
                            })),
                    ),
            )
            // 历史列表（可滚动）。
            .child(if records.is_empty() {
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("暂无历史记录")
                    .into_any_element()
            } else {
                let mut list = v_flex()
                    .id("history-records")
                    .flex_1()
                    .gap_2()
                    .overflow_y_scroll();
                for rec in &records {
                    list = list.child(self.render_record(rec, cx));
                }
                list.into_any_element()
            })
    }
}

/// 由前缀 + 记录 id 派生稳定且唯一的 ElementId（避免同列表内 id 冲突）。
fn elem_id(prefix: &str, id: &str) -> gpui::SharedString {
    gpui::SharedString::from(format!("{prefix}-{id}"))
}

fn mode_icon(mode: &str) -> &'static str {
    match mode {
        "streaming" => "🎤",
        "offline" => "📄",
        "local" => "💻",
        _ => "•",
    }
}

/// RFC3339 UTC → 本地 "MM-DD HH:MM"；解析失败则原样截断显示。
fn format_time(rfc3339: &str) -> String {
    match DateTime::parse_from_rfc3339(rfc3339) {
        Ok(dt) => dt
            .with_timezone(&Local)
            .format("%m-%d %H:%M")
            .to_string(),
        Err(_) => rfc3339.chars().take(16).collect(),
    }
}

/// 单行预览：去换行 + 前 50 字符，超出加省略号。
fn preview(text: &str) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = one_line.chars().take(50).collect();
    if one_line.chars().count() > 50 {
        out.push('…');
    }
    out
}
