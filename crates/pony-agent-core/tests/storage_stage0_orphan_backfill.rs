//! ADR 0099 阶段 0 · 契约 4：孤儿会话回填（源只能是 `turn_events ∪ normalized_*`）。
//!
//! 现网事实（ADR 0099 背景表）：`sessions` blob 行 71 / `normalized_sessions` 121 ⇒
//! **50 个会话（41%）只有规范化行与事件、没有 blob 行**，重启后不可见。阶段 0 第 4 条
//! 要求处置这批孤儿，且**回填源只能是 `turn_events ∪ normalized_*`，绝不能以 `sessions`
//! 表为源**（以 blob 为源会把残缺/截断内容固化为权威，且孤儿根本没有 blob 行）。
//!
//! 冻结契约（`.dev-team/contracts/stage0-write-model.md` §1，I4）：
//! `SqliteSessionBackend::backfill_orphan_sessions(&self) -> Result<BackfillReport, String>`，
//! `BackfillReport { backfilled: Vec<String>, retired: Vec<String>, checksum_mismatch: Vec<String> }`；
//! 判据 = 回填后 `sessions` 行存在且消息条数/校验和与 `normalized_messages` 一致；
//! 门禁 = 孤儿双向集合差 = 0。
//!
//! ⚠️ 交付状态：**入口函数尚未落码**（属阶段 0 Executor 任务，`sqlite_session.rs`）。
//! 本文件因此当前**编译不过**（`no function or associated item named backfill_orphan_sessions`），
//! 这是「依赖但尚不存在的 API」的显式登记，不是测试自身缺陷；单独成文件以免连带打断
//! 其余 5 个可运行用例。入口落码后本文件应直接由红转绿，无需改动断言。
//!
//! 隔离：临时目录 + 临时 db；绝不触碰 `~/.local/share/PonyAgent/sessions.db`。

use pony_agent_core::agent::session::{
    BackfillReport, PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionBackend,
    SessionStore, TurnHistoryMessage,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use pony_agent_core::agent::turn_event::TurnEvent;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-0099-stage0-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
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
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn scalar_count(db: &Path, sql: &str, session_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(sql, [session_id], |row| row.get::<_, i64>(0))
        .expect("count rows")
}

fn session_id_set(db: &Path, sql: &str) -> BTreeSet<String> {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    let mut stmt = conn.prepare(sql).expect("prepare id query");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query ids");
    rows.filter_map(Result::ok).collect()
}

fn append_message(
    backend: &SqliteSessionBackend,
    session_id: &str,
    turn_id: &str,
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
            turn_id: Some(turn_id.to_string()),
            ..Default::default()
        },
        ordinal,
    });
    assert_eq!(
        outcome,
        PersistCommandOutcome::Succeeded,
        "夹具：AppendMessage 必须写入规范化表"
    );
}

fn flush_events(backend: &SqliteSessionBackend, session_id: &str, turn_id: &str) {
    let outcome = backend.persist_command(PersistCommand::FlushEvents {
        epoch: 1,
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        branch_id: "main".to_string(),
        events: vec![
            TurnEvent::UserMessage {
                turn_id: turn_id.to_string(),
                text: "事件侧用户消息".to_string(),
                attachments: Vec::new(),
            },
            TurnEvent::AssistantMessage {
                turn_id: turn_id.to_string(),
                step: 0,
                text: "事件侧助手消息".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ],
    });
    assert_eq!(
        outcome,
        PersistCommandOutcome::Succeeded,
        "夹具：FlushEvents 必须写入 turn_events"
    );
}

/// 只有 `normalized_*` + `turn_events`、没有 `sessions` blob 行的孤儿会话，回填后
/// `sessions` 表必须出现该行，且消息条数与 `normalized_messages` 一致；门禁是孤儿归零。
#[test]
fn orphan_sessions_are_backfilled_from_normalized_rows_and_events() {
    // Arrange：两个孤儿会话（AppendMessage 只写规范化表；FlushEvents 只写事件表）。
    let dir = TempDir::new("orphan");
    let db = dir.db();
    let backend = SqliteSessionBackend::new_with_trace_mode(
        db.clone(),
        SeparateTraceTableMode::WriteSeparate,
    );

    append_message(&backend, "s-orphan-a", "t-a", 0, "user", "规范化-问-A");
    append_message(&backend, "s-orphan-a", "t-a", 1, "assistant", "规范化-答-A");
    flush_events(&backend, "s-orphan-a", "t-a");

    append_message(&backend, "s-orphan-b", "t-b", 0, "user", "规范化-问-B");
    append_message(&backend, "s-orphan-b", "t-b", 1, "assistant", "规范化-答-B");
    append_message(&backend, "s-orphan-b", "t-b", 2, "user", "规范化-再问-B");

    // 前提：两个会话都没有 legacy blob 行（真实孤儿形态）。
    for session_id in ["s-orphan-a", "s-orphan-b"] {
        assert_eq!(
            scalar_count(
                &db,
                "SELECT COUNT(*) FROM sessions WHERE conversation_id = ?1",
                session_id
            ),
            0,
            "前提：{session_id} 没有 sessions blob 行（孤儿）"
        );
    }
    assert_eq!(
        scalar_count(
            &db,
            "SELECT COUNT(*) FROM turn_events WHERE session_id = ?1",
            "s-orphan-a"
        ),
        2,
        "前提：s-orphan-a 的事件行由真实 FlushEvents 路径写入"
    );

    // Act：执行孤儿回填。
    let report: BackfillReport = backend
        .backfill_orphan_sessions()
        .expect("孤儿回填不得失败（回填源：turn_events ∪ normalized_*）");

    // Assert ①：报告显式登记了两个孤儿。
    assert!(
        report.backfilled.contains(&"s-orphan-a".to_string())
            && report.backfilled.contains(&"s-orphan-b".to_string()),
        "回填报告必须登记两个孤儿会话，实际 {report:?}"
    );
    assert!(
        report.checksum_mismatch.is_empty(),
        "回填校验和必须一致（不得出现 quarantine），实际 {report:?}"
    );

    // Assert ②：sessions 表出现该行，消息条数 == normalized_messages 条数。
    for (session_id, expected_messages) in [("s-orphan-a", 2i64), ("s-orphan-b", 3i64)] {
        let normalized = scalar_count(
            &db,
            "SELECT COUNT(*) FROM normalized_messages WHERE session_id = ?1",
            session_id,
        );
        assert_eq!(
            normalized, expected_messages,
            "前提：{session_id} 规范化消息条数"
        );

        let blob_rows = scalar_count(
            &db,
            "SELECT COUNT(*) FROM sessions WHERE conversation_id = ?1",
            session_id,
        );
        assert_eq!(
            blob_rows, 1,
            "ADR 0099 阶段 0 / I4：回填后 sessions 表必须出现 {session_id} 行（当前实现从不回填 blob 行）"
        );

        let data: String = rusqlite::Connection::open(&db)
            .expect("open sqlite read-back")
            .query_row(
                "SELECT session_data FROM sessions WHERE conversation_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .expect("read backfilled blob");
        let value: serde_json::Value = serde_json::from_str(&data).expect("blob 必须是 JSON");
        let history_len = value
            .get("history")
            .and_then(|history| history.as_array())
            .map(|history| history.len())
            .unwrap_or_default() as i64;
        assert_eq!(
            history_len, normalized,
            "ADR 0099 阶段 0 / I4：回填后的消息条数必须与 normalized_messages 一致\
             （回填源只能是 normalized_* / turn_events，不得以 sessions 表为源）"
        );
    }

    // Assert ③：孤儿归零（门禁：双向集合差 = 0）。
    let normalized_ids = session_id_set(&db, "SELECT session_id FROM normalized_sessions");
    let blob_ids = session_id_set(&db, "SELECT conversation_id FROM sessions");
    let missing: Vec<&String> = normalized_ids.difference(&blob_ids).collect();
    assert!(
        missing.is_empty(),
        "ADR 0099 阶段 0 门禁：孤儿必须归零（双向集合差 = 0），仍缺 blob 行：{missing:?}"
    );

    // Assert ④：用户可见面恢复（重启后不再「没有该会话」）。
    let reloaded = SessionStore::with_backend(Box::new(SqliteSessionBackend::new_with_trace_mode(
        db.clone(),
        SeparateTraceTableMode::WriteSeparate,
    )));
    for session_id in ["s-orphan-a", "s-orphan-b"] {
        assert!(
            reloaded
                .list_sessions()
                .iter()
                .any(|overview| overview.conversation_id == session_id),
            "回填后 {session_id} 必须出现在 list_sessions() 里"
        );
    }
}
