//! ADR 0099 阶段 0 · 契约 2：CAS 守门（过期 `version` 必须 `StaleVersion` 且零写入）。
//!
//! 现网事实（ADR 0099 背景）：`sessions` 表没有任何 session 级 `version` 列，
//! 整包 `INSERT OR REPLACE` 使「旧快照覆盖新 turn」成为常规路径；`store_metadata` 里
//! `storage.normalized.v1.epoch` 键根本不存在 ⇒ 所谓乐观锁从未生效。
//!
//! 冻结契约（`.dev-team/contracts/stage0-write-model.md` §1/§3，I2）：
//! - 唯一提交入口 `SessionBackend::commit_boundary(&PersistedStore) -> CommitOutcome`；
//! - `CommitOutcome::StaleVersion { expected, actual }` ⇒ **整包零写入**
//!   （不覆盖、不删除、不复活）；
//! - `CommitOutcome::Committed` ⇒ 本快照按 CAS 落盘（各受限会话版本 +1）。
//!
//! 版本载体：`PersistedStore.versions: SessionVersionMap`（`session_id -> SessionVersion`），
//! 由 `load_store()` 从库内读入、由 `commit_boundary` 与库内实际版本比对。
//!
//! 用例设计（防两种假绿）：
//! ① `StaleVersion` 用例：喂**旧快照**必须被拒绝且数据库读回不变；
//! ② `Committed → StaleVersion` 双段用例：同一份快照第一次提交必须 `Committed`、
//!    第二次必须 `StaleVersion`。「永远 StaleVersion」与「永远 Committed」两种假绿
//!    同时被否。
//!
//! 隔离：临时目录 + 临时 db；绝不触碰 `~/.local/share/PonyAgent/sessions.db`。
//!
//! 红相（交付时）：`PersistedStore.versions` 与 `SqliteSessionBackend::commit_boundary`
//! 尚未落码 ⇒ 默认实现返回 `CommitOutcome::Unsupported`，两条用例都在 outcome 断言处失败
//! ——失败原因正是「CAS 守门未实现」，不是编译错误。

use pony_agent_core::agent::session::{
    CommitOutcome, PersistedStore, SeparateTraceTableMode, SessionBackend, SessionStore,
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

fn open_sqlite(db: &Path) -> SqliteSessionBackend {
    SqliteSessionBackend::new_with_trace_mode(
        db.to_path_buf(),
        SeparateTraceTableMode::WriteSeparate,
    )
}

/// 读回 `sessions` 表内该会话 blob 的 history 条数（数据库真实内容，不信内存态）。
fn blob_history_len(db: &Path, session_id: &str) -> usize {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    let data: String = conn
        .query_row(
            "SELECT session_data FROM sessions WHERE conversation_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .expect("sessions 行必须存在（读回校验）");
    let value: serde_json::Value = serde_json::from_str(&data).expect("session_data 必须是 JSON");
    value
        .get("history")
        .and_then(|history| history.as_array())
        .map(|history| history.len())
        .expect("session blob 必须带 history 数组")
}

/// 读回 `normalized_messages` 行数（第二真源侧，防「只写 blob 不写规范化表」式假绿）。
fn normalized_message_count(db: &Path, session_id: &str) -> u64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(
        "SELECT COUNT(*) FROM normalized_messages WHERE session_id = ?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .expect("count normalized_messages")
    .max(0) as u64
}

fn load_snapshot(reader: &SqliteSessionBackend) -> PersistedStore {
    reader
        .load_store()
        .expect("快照必须可装载（临时 db 由前面的写入初始化）")
}

// ---------------------------------------------------------------------------
// 契约 2-a：过期 version ⇒ StaleVersion + 数据库内容不变
// ---------------------------------------------------------------------------

#[test]
fn stale_version_commit_is_rejected_and_database_unchanged() {
    // Arrange：A 端写入第一轮，随后第二个实例读走「旧快照」。
    let dir = TempDir::new("cas-stale");
    let db = dir.db();
    let mut writer_store = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    writer_store.append_turn(Some("s-cas"), "第一问", "第一答", None, Vec::new());
    assert_eq!(
        blob_history_len(&db, "s-cas"),
        2,
        "前提：第一轮 turn 已落盘（2 条消息）"
    );

    let stale_reader = open_sqlite(&db);
    let stale = load_snapshot(&stale_reader);

    // A 端继续写入第二轮 ⇒ 库内版本前进，stale 快照的期望版本落后。
    writer_store.append_turn(Some("s-cas"), "第二问", "第二答", None, Vec::new());
    assert_eq!(
        blob_history_len(&db, "s-cas"),
        4,
        "前提：第二轮 turn 已落盘（4 条消息）"
    );

    // Act：用旧快照提交。
    let outcome = stale_reader.commit_boundary(&stale);

    // Assert 1：必须被 CAS 守门拒绝，且携带 expected < actual。
    match outcome {
        CommitOutcome::StaleVersion { expected, actual } => {
            assert!(
                expected < actual,
                "StaleVersion 必须携带 expected < actual（实际 expected={expected}, actual={actual}）"
            );
        }
        other => panic!(
            "ADR 0099 阶段 0 / I2：过期 version 提交必须返回 StaleVersion，实际 {other:?}；\
             当前实现（commit_boundary 默认 Unsupported + sessions 表无 version 列）会让旧快照\
             直接覆盖新 turn"
        ),
    }

    // Assert 2：数据库内容不变（读回校验，不信返回值）。
    assert_eq!(
        blob_history_len(&db, "s-cas"),
        4,
        "StaleVersion 必须零写入：旧快照的 2 条消息不得覆盖库内 4 条"
    );
    assert_eq!(
        normalized_message_count(&db, "s-cas"),
        4,
        "StaleVersion 必须零写入：规范化表 side 也不得被回滚成 2 条"
    );
}

// ---------------------------------------------------------------------------
// 契约 2-b：Committed → 同一快照再提交 ⇒ StaleVersion（双向防假绿）
// ---------------------------------------------------------------------------

#[test]
fn fresh_snapshot_commits_then_same_snapshot_is_stale() {
    // Arrange
    let dir = TempDir::new("cas-commit");
    let db = dir.db();
    let mut writer_store = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    writer_store.append_turn(Some("s-cas2"), "唯一一问", "唯一一答", None, Vec::new());

    let reader = open_sqlite(&db);
    let fresh = load_snapshot(&reader);

    // Act 1：刚读到的快照 → 必须允许提交（否则「永远 StaleVersion」假绿）。
    let first = reader.commit_boundary(&fresh);
    assert_eq!(
        first,
        CommitOutcome::Committed,
        "ADR 0099 阶段 0：load_store() 之后直接 commit 必须是 Committed（期望版本 = 库内最新），\
         实际 {first:?}"
    );

    // Act 2：同一份（现在已过期的）快照再提交一次。
    let second = reader.commit_boundary(&fresh);
    assert!(
        matches!(second, CommitOutcome::StaleVersion { .. }),
        "提交成功后库内版本 +1，同一份快照再提交必须是 StaleVersion，实际 {second:?}"
    );

    // Assert：两次提交后库内仍只有 2 条消息（Committed 幂等、StaleVersion 零写入）。
    assert_eq!(
        blob_history_len(&db, "s-cas2"),
        2,
        "提交不得把消息条数改坏（Committed 幂等 + StaleVersion 零写入）"
    );
}
