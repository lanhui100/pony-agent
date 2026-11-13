//! B148 阶段 1 验收：单会话**定向装载**（`load_session_targeted`）必须与**全量装载**
//! （`load_store().sessions[id]`）逐字段一致；轻量元数据索引（`load_session_meta_index`）
//! 与全量装载的会话集合、标题、轮次、可见性一致；缺失会话返回 `NotFound`。
//!
//! 隔离：所有库落在 `std::env::temp_dir()` 下唯一目录，fixture `Drop` 清理，
//! **绝不触碰** `~/.local/share/PonyAgent/`。

use std::collections::HashMap;
use std::path::PathBuf;

use pony_agent_core::agent::session::{
    HistoryCursor, SeparateTraceTableMode, SessionBackend, SessionState, TargetedSessionLoad,
    TraceMigrationState, TurnHistoryMessage, TurnTraceRecord,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "pony-b148-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn db(&self) -> PathBuf {
        self.root.join("sessions.db")
    }

    fn backend(&self) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(self.db(), SeparateTraceTableMode::DualWrite)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn trace(turn_id: &str, title: &str, at: u64) -> TurnTraceRecord {
    TurnTraceRecord {
        turn_id: turn_id.to_string(),
        title: title.to_string(),
        phase: "completed".to_string(),
        updated_at: at,
        ..Default::default()
    }
}

fn user_msg(content: &str, turn_id: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "user".to_string(),
        content: content.to_string(),
        turn_id: Some(turn_id.to_string()),
        ..Default::default()
    }
}

fn rich_session(id: &str, title: &str, at: u64) -> SessionState {
    SessionState {
        conversation_id: id.to_string(),
        title: title.to_string(),
        summary: format!("{title}-summary"),
        history: vec![
            user_msg(&format!("{title}-hello"), "t1"),
            user_msg(&format!("{title}-world"), "t2"),
        ],
        provider_native_transcript: Vec::new(),
        turn_trace_history: vec![trace("t1", "first", at), trace("t2", "second", at)],
        trace_migration_state: TraceMigrationState::default(),
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 2,
        last_referenced_file: None,
        updated_at_ms: at,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: HistoryCursor::default(),
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: None,
        archived: false,
    }
}

fn json_of<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("serialize")
}

/// 写两个有内容的会话（legacy 相位：blob + trace 表），返回可读后端。
fn seed(fx: &Fixture) -> SqliteSessionBackend {
    let backend = fx.backend();
    assert!(
        backend.upsert_session("s1", &rich_session("s1", "alpha", 1000)),
        "upsert s1"
    );
    assert!(
        backend.upsert_session("s2", &rich_session("s2", "beta", 2000)),
        "upsert s2"
    );
    backend
}

#[test]
fn targeted_equals_full_load() {
    let fx = Fixture::new("legacy");
    let backend = seed(&fx);

    let full = backend.load_store().expect("load_store").sessions_view().clone();
    assert!(
        full.contains_key("s1") && full.contains_key("s2"),
        "全量装载必须包含已播种会话，实际 = {:?}",
        full.keys().collect::<Vec<_>>()
    );

    for id in ["s1", "s2"] {
        match backend.load_session_targeted(id) {
            TargetedSessionLoad::Loaded(state) => {
                assert_eq!(
                    json_of(&*state),
                    json_of(&full[id]),
                    "定向装载与全量装载逐字段不一致：{id}"
                );
            }
            other => panic!("会话 {id} 期望 Loaded，实际 {other:?}"),
        }
    }

    assert!(
        backend.supports_targeted_session_load(),
        "SQLite 后端必须声明支持定向装载"
    );
}

#[test]
fn targeted_not_found_for_missing() {
    let fx = Fixture::new("notfound");
    let backend = seed(&fx);
    assert!(
        matches!(
            backend.load_session_targeted("does-not-exist"),
            TargetedSessionLoad::NotFound
        ),
        "不存在的会话必须返回 NotFound"
    );
}

#[test]
fn meta_index_matches_full_load() {
    let fx = Fixture::new("meta");
    let backend = seed(&fx);
    let full = backend.load_store().expect("load_store").sessions_view().clone();

    let meta = backend
        .load_session_meta_index()
        .expect("load_session_meta_index");
    let mut by_id: HashMap<String, _> = HashMap::new();
    for entry in meta {
        by_id.insert(entry.session_id.clone(), entry);
    }

    for (id, session) in &full {
        let entry = by_id
            .get(id)
            .unwrap_or_else(|| panic!("元数据索引缺少会话 {id}"));
        assert_eq!(entry.title, session.title, "title 不一致：{id}");
        assert_eq!(
            entry.turn_count, session.turn_count,
            "turn_count 不一致：{id}"
        );
        assert!(entry.has_content, "有内容会话的 has_content 必须为真：{id}");
    }
}

#[test]
fn load_store_metadata_has_no_sessions() {
    let fx = Fixture::new("metadata");
    let backend = seed(&fx);
    let metadata = backend.load_store_metadata().expect("load_store_metadata");
    assert!(
        metadata.sessions_view().is_empty(),
        "load_store_metadata 不得装载会话内容（稀疏语义），实际 {} 条",
        metadata.sessions_view().len()
    );
}
