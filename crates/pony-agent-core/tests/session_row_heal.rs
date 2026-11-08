//! ADR 0094：会话索引行（legacy `sessions` blob）丢失时的自愈回归守卫。
//!
//! 现场：用户按退出提示 `pony resume <id>`，TUI 却回「尚无历史记录，已沿用该 id 开始新会话」，
//! 而库里该会话的轮次/消息/事件全在。`load_store()` 在 legacy phase 只读 `sessions` 表，
//! 行没了就等于会话不存在。
//!
//! 契约：`pony resume <id>` 在 legacy 行缺失时先按 id 从规范化表重建并回写自愈；
//! 显式 `remove_session` 必须连规范化行一起删（否则自愈会把已删会话复活）。
//!
//! 测试只走公开 API：规范化行由 `persist_command(AppendMessage)`（真实事件路径）产生，
//! 天然就是「只有规范化行、没有 legacy 行」的状态，不需要直接改库。
//! 「整包写盘不删快照外会话」的守卫需要 `PersistedStore.sessions`（非公开），
//! 落在 `sqlite_session::tests::full_store_write_keeps_sessions_dropped_from_latest_snapshot`。

use pony_agent_core::agent::session::{
    PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionBackend, SessionStore,
    TurnHistoryMessage,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use std::path::{Path, PathBuf};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-0094-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn db(&self) -> PathBuf {
        self.path.join("sessions.db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

fn open_backend(db: &Path) -> SqliteSessionBackend {
    SqliteSessionBackend::new_with_trace_mode(
        db.to_path_buf(),
        SeparateTraceTableMode::WriteSeparate,
    )
}

/// 只写规范化表（真实事件路径），不写 legacy blob 行。
fn append_message(
    backend: &SqliteSessionBackend,
    session_id: &str,
    ordinal: usize,
    role: &str,
    content: &str,
) {
    let outcome = backend.persist_command(PersistCommand::AppendMessage {
        epoch: 1,
        session_id: session_id.to_string(),
        message: TurnHistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            turn_id: Some("turn-1".to_string()),
            status: None,
            model_name: None,
            token_count: None,
            reasoning_content: None,
        },
        ordinal,
    });
    assert_eq!(
        outcome,
        PersistCommandOutcome::Succeeded,
        "AppendMessage 必须写入规范化表"
    );
}

/// 缺陷 2：legacy 行丢了以后 `pony resume <id>` 谎称没有历史 —— 必须能从规范化表自愈。
#[test]
fn resume_recovers_session_from_normalized_rows_when_legacy_row_is_missing() {
    let dir = TempDir::new("normalized-recover");
    let db = dir.db();
    let backend = open_backend(&db);

    append_message(&backend, "s-rec", 0, "user", "第一问");
    append_message(&backend, "s-rec", 1, "assistant", "第一答");

    // 前提：legacy phase 的加载只读 `sessions` 表 ⇒ 该会话"不存在"（列表里看不到）。
    assert!(
        SessionStore::with_backend(Box::new(open_backend(&db)))
            .list_sessions()
            .iter()
            .all(|overview| overview.conversation_id != "s-rec"),
        "前提：只有规范化行时 legacy 表里没有这条会话（用户现场 existed=false）"
    );

    // 修复点 1：按 id 从规范化表重建单会话。
    let recovered = backend
        .load_normalized_session("s-rec")
        .expect("规范化行存在时必须能重建会话");
    assert_eq!(recovered.history.len(), 2, "重建必须带回真实历史");

    // 修复点 2：store 层自愈（进内存 + 回写 legacy 行），hydrate 才有历史可读。
    let mut store = SessionStore::with_backend(Box::new(open_backend(&db)));
    assert!(
        store
            .list_sessions()
            .iter()
            .all(|overview| overview.conversation_id != "s-rec"),
        "前提：索引行丢失 ⇒ 启动列表里看不到它"
    );
    assert!(
        store.recover_session_from_normalized("s-rec"),
        "ADR 0094：existed=false 时必须先尝试按 id 重建，而不是直接开新会话"
    );
    assert_eq!(
        store.snapshot(Some("s-rec"), &[]).history.len(),
        2,
        "自愈后必须能读回两条历史消息"
    );

    let healed = SessionStore::with_backend(Box::new(open_backend(&db)))
        .list_sessions()
        .into_iter()
        .any(|overview| overview.conversation_id == "s-rec");
    assert!(
        healed,
        "自愈必须回写 legacy 行，否则下次启动又是「缺失」状态"
    );
}

/// 自愈的反面：用户显式删除必须连规范化行一起删，否则下次 `pony resume <id>` 会复活。
#[test]
fn explicit_delete_removes_normalized_rows_so_resume_cannot_resurrect() {
    let dir = TempDir::new("normalized-delete");
    let db = dir.db();
    let backend = open_backend(&db);
    append_message(&backend, "s-del", 0, "user", "待删除");

    let mut store = SessionStore::with_backend(Box::new(open_backend(&db)));
    assert!(
        store.recover_session_from_normalized("s-del"),
        "前提：先让它进内存，再走真实删除路径"
    );
    store.remove_session("s-del");

    assert!(
        open_backend(&db).load_normalized_session("s-del").is_none(),
        "ADR 0094：显式删除必须连规范化行一起删，否则 resume 会把已删会话复活"
    );
}
