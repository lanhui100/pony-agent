//! ADR 0099 阶段 0 · 契约 6：`recover`（`pony resume`）不得丢 trace。
//!
//! 现网事实（ADR 0099 背景缺陷 4）：`pony resume` 从规范化表恢复时，trace 取自 **legacy**
//! `session_turn_traces` 表（`sqlite_session.rs::read_all_session_traces` 只查它），而 50 个
//! 孤儿会话里有 29 个**只有 `normalized_turn_traces`** ⇒ 恢复即半残；随后自愈回写还会把
//! 残缺固化成「权威」。
//!
//! 冻结契约（`.dev-team/contracts/stage0-write-model.md` §5 E7）：
//! `SqliteSessionBackend::load_normalized_session` 内部 trace 必须取
//! `normalized_turn_traces`，**不得**再走 legacy `session_turn_traces`。
//!
//! 夹具构造（复现孤儿现场，只动临时库）：
//! 1. 用真实写入路径 `persist_command(AppendMessage ×2 / AppendTrace ×2)` 造出规范化行；
//! 2. 删掉 legacy `sessions` blob 行与 legacy `session_turn_traces` 行 ⇒ 只剩
//!    `normalized_*` + `turn_events`（正是 ADR 里 29 个孤儿之一）；
//! 3. 执行恢复，断言 `turn_trace_history` 非空且条数 == `normalized_turn_traces` 行数。
//!
//! 隔离：临时目录 + 临时 db；绝不触碰 `~/.local/share/PonyAgent/sessions.db`。
//!
//! 红相（交付时）：`read_all_session_traces` 只查 legacy 表 ⇒ 恢复出的 `turn_trace_history`
//! 为空（0 条），断言 `== 2` 失败。

use pony_agent_core::agent::session::{
    PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionBackend,
    TurnHistoryMessage, TurnTraceRecord,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
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

fn row_count(db: &Path, table: &str, session_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    // 表名来自本文件内的常量字面量，不含外部输入。
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?1"),
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .expect("count rows")
}

/// legacy `sessions` 表的主键列是 `conversation_id`（不是 `session_id`）。
fn legacy_blob_row_count(db: &Path, session_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE conversation_id = ?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .expect("count legacy blob rows")
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

fn append_trace(
    backend: &SqliteSessionBackend,
    session_id: &str,
    turn_id: &str,
    trace_order: usize,
) {
    let outcome = backend.persist_command(PersistCommand::AppendTrace {
        epoch: 1,
        session_id: session_id.to_string(),
        trace: TurnTraceRecord {
            turn_id: turn_id.to_string(),
            title: format!("turn {turn_id}"),
            phase: "completed".to_string(),
            updated_at: 1_700_000_000_000,
            ..Default::default()
        },
        trace_order,
    });
    assert_eq!(
        outcome,
        PersistCommandOutcome::Succeeded,
        "夹具：AppendTrace 必须写入规范化 trace 表"
    );
}

#[test]
fn recover_from_normalized_rows_keeps_turn_traces() {
    // Arrange：只留 normalized_* + 事件行的孤儿会话。
    let dir = TempDir::new("recover-trace");
    let db = dir.db();
    let backend = SqliteSessionBackend::new_with_trace_mode(
        db.clone(),
        SeparateTraceTableMode::WriteSeparate,
    );

    append_message(&backend, "s-tr", "t1", 0, "user", "第一问");
    append_message(&backend, "s-tr", "t1", 1, "assistant", "第一答");
    append_trace(&backend, "s-tr", "t1", 0);
    append_trace(&backend, "s-tr", "t2", 1);

    // 夹具手术：删 legacy blob 行与 legacy trace 行（模拟真实孤儿的存储形态）。
    {
        let conn = rusqlite::Connection::open(&db).expect("open sqlite fixture");
        conn.execute("DELETE FROM sessions WHERE conversation_id = 's-tr'", [])
            .expect("drop legacy blob row");
        conn.execute(
            "DELETE FROM session_turn_traces WHERE session_id = 's-tr'",
            [],
        )
        .expect("drop legacy trace rows");
    }

    let normalized_traces = row_count(&db, "normalized_turn_traces", "s-tr");
    let normalized_messages = row_count(&db, "normalized_messages", "s-tr");
    assert_eq!(
        normalized_traces, 2,
        "夹具：normalized_turn_traces 应有 2 行"
    );
    assert_eq!(
        normalized_messages, 2,
        "夹具：normalized_messages 应有 2 行"
    );
    assert_eq!(
        legacy_blob_row_count(&db, "s-tr"),
        0,
        "夹具：孤儿会话没有 legacy blob 行"
    );
    assert_eq!(
        row_count(&db, "session_turn_traces", "s-tr"),
        0,
        "夹具：只有 normalized trace，没有 legacy trace 行"
    );

    // Act：从规范化表恢复会话。
    let recovered = backend
        .load_normalized_session("s-tr")
        .expect("规范化行存在 ⇒ 必须能恢复出会话");

    // Assert ①：恢复出的历史消息条数与规范化表一致（保证恢复路径本身有效，非空转）。
    assert_eq!(
        recovered.history.len(),
        normalized_messages as usize,
        "前提：恢复必须带回全部规范化消息（否则 trace 断言无意义）"
    );

    // Assert ②：trace 不得丢失，且条数 == normalized_turn_traces 行数。
    assert!(
        !recovered.turn_trace_history.is_empty(),
        "ADR 0099 阶段 0（E7：resume trace 源）：recover 丢 trace —— 只有 normalized_turn_traces 的孤儿会话\
         恢复后 turn_trace_history 为空（现实现 read_all_session_traces 只读 legacy \
         session_turn_traces）"
    );
    assert_eq!(
        recovered.turn_trace_history.len(),
        normalized_traces as usize,
        "ADR 0099 阶段 0（E7）：恢复出的 trace 条数必须等于 normalized_turn_traces 行数"
    );
}

/// 反向守卫（B047）：**legacy 独有**的 trace 仍必须被 E7 兜底读回。
///
/// `read_traces_normalized` 的兜底此前是「先整表批量读 `session_turn_traces`，再逐会话
/// `or_insert`」——真实库上 222MB / 645ms 读完即弃（该库「有 legacy 行但规范化侧无 trace」
/// 的会话数 = 0）。收窄为「只回读规范化侧给不出 trace 的会话」后，**语义不得变**：
/// 这条用例钉的就是「legacy 独有的会话依然能恢复出 trace」。
#[test]
fn legacy_only_traces_are_still_recovered_by_the_narrowed_fallback() {
    let dir = TempDir::new("recover-legacy-only");
    let db = dir.db();
    let backend = SqliteSessionBackend::new_with_trace_mode(
        db.clone(),
        SeparateTraceTableMode::WriteSeparate,
    );

    append_message(&backend, "s-lo", "t1", 0, "user", "第一问");
    append_trace(&backend, "s-lo", "t1", 0);
    append_trace(&backend, "s-lo", "t2", 1);

    // 夹具手术：删掉规范化侧 trace 行，只留 legacy `session_turn_traces`
    // （这正是 E7 兜底口子存在的唯一理由）。
    {
        let conn = rusqlite::Connection::open(&db).expect("open sqlite fixture");
        conn.execute(
            "DELETE FROM normalized_turn_traces WHERE session_id = 's-lo'",
            [],
        )
        .expect("drop normalized trace rows");
    }
    assert_eq!(
        row_count(&db, "normalized_turn_traces", "s-lo"),
        0,
        "夹具：规范化侧必须没有该会话的 trace 行"
    );
    assert_eq!(
        row_count(&db, "session_turn_traces", "s-lo"),
        2,
        "夹具：legacy 侧必须仍有 2 行"
    );

    let recovered = backend
        .load_normalized_session("s-lo")
        .expect("legacy trace 行存在 ⇒ 必须能恢复出会话");

    assert_eq!(
        recovered.turn_trace_history.len(),
        2,
        "legacy 独有 trace 必须由 E7 兜底补回；否则「收窄兜底」= 静默丢 trace"
    );
}
