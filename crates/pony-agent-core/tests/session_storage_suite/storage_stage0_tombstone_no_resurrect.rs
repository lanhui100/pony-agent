//! ADR 0099 阶段 0 · 契约 3：不复活已删会话（墓碑优先于旧快照）。
//!
//! 现网事实（ADR 0099 背景缺陷 2/3）：移除 DELETE 后（ADR 0090 的对称代价），整包
//! `INSERT OR REPLACE` 的语义是「把我知道的写下去」；于是一个**启动时装载、包含已删
//! 会话的第二实例**，只要做任何一次全库落盘（建工作区、改标题、归档……），就会把用户
//! 已经删掉的会话重新 `INSERT` 回去。
//!
//! 冻结契约（`.dev-team/contracts/stage0-write-model.md`，I3）：显式删除后，任何持有旧
//! 快照的实例提交都必须**跳过**该会话（墓碑优先于快照），`sessions` / `normalized_sessions`
//! 表与 `list_sessions()` 都不得再出现它。
//!
//! 边界语义（契约已冻结，防误判）：墓碑/已删除会话在 `commit_boundary` 内是「跳过且永不
//! INSERT」，**不改变整包 outcome** —— 因此「删除后旧快照再 commit」的期望是
//! `CommitOutcome::Committed`（跳过该会话），**不是** `StaleVersion`、也不是 `Failed`。
//!
//! 本用例走**真实公开路径**复现现场：
//! 1. store A 写入会话 `s-ghost`；
//! 2. store B（第二个实例，独立连接）装载快照，内存里含 `s-ghost`；
//! 3. store A 显式 `remove_session("s-ghost")`（真实删除，删 blob + legacy trace + 规范化行）；
//! 4. store B 做一次无关的全库落盘（`create_workspace` ⇒ `save_to_backend`）；
//! 5. 断言 `s-ghost` 在 `sessions` 表与 `list_sessions()` 里都不存在，且这次提交**确实
//!    发生过**（`store_metadata.workspaces` 已落库）——否则「什么都没写」也会让用例假绿。
//!
//! 隔离：临时目录 + 临时 db（含 workspace root 也在临时目录内），绝不触碰真实库。
//!
//! 红相（交付时）：`write_full_store` 对快照内所有会话无条件 `INSERT OR REPLACE`
//! （`sqlite_session.rs:611`）⇒ `s-ghost` 被复活 ⇒ 断言失败。

use pony_agent_core::agent::session::{SeparateTraceTableMode, SessionStore};
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

fn session_row_count(db: &Path, session_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE conversation_id = ?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .expect("count sessions row")
}

fn store_metadata_contains(db: &Path, key: &str, needle: &str) -> bool {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(
        "SELECT value FROM store_metadata WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .map(|value| value.contains(needle))
    .unwrap_or(false)
}

#[test]
fn deleted_session_is_not_resurrected_by_stale_second_instance_commit() {
    // Arrange
    let dir = TempDir::new("tombstone");
    let db = dir.db();
    let workspace_root = dir.path.join("workspace-root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");

    // ① A 端创建会话（真实 turn 写入路径）。
    let mut store_a = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    store_a.append_turn(
        Some("s-ghost"),
        "待删除会话：用户消息",
        "待删除会话：助手回复",
        None,
        Vec::new(),
    );
    assert_eq!(
        session_row_count(&db, "s-ghost"),
        1,
        "前提：会话已落盘，sessions 表有 1 行"
    );

    // ② B 端（第二个实例、独立连接）装载旧快照——内存里带着 s-ghost。
    let mut store_b = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    assert!(
        store_b
            .list_sessions()
            .iter()
            .any(|overview| overview.conversation_id == "s-ghost"),
        "前提：B 端快照里必须含 s-ghost（否则复现不了旧快照复活）"
    );

    // ③ A 端显式删除，并确认删除已生效。
    store_a.remove_session("s-ghost");
    assert_eq!(
        session_row_count(&db, "s-ghost"),
        0,
        "前提：显式删除必须落库（sessions 行已删）"
    );

    // Act：B 端做一次无关的全库落盘（持有旧快照的第二实例提交）。
    store_b
        .create_workspace("tombstone-ws", workspace_root.to_str().expect("utf-8 root"))
        .expect("create_workspace 必须成功");

    // Assert ①：这次提交真的发生了（否则「零写入」会造成假绿）。
    assert!(
        store_metadata_contains(&db, "workspaces", "tombstone-ws"),
        "前提防线：B 端这次提交必须真的落盘（store_metadata.workspaces 含新工作区），\
         否则后面的「没复活」可能只是因为它什么都没写"
    );

    // Assert ②：已删会话仍未出现在 sessions 表。
    assert_eq!(
        session_row_count(&db, "s-ghost"),
        0,
        "ADR 0099 阶段 0 / I3：显式删除后，任何旧快照提交都不得把 s-ghost 重新 INSERT 回 sessions 表"
    );

    // Assert ③：规范化表侧同样不得复活（合同 §3 I3：sessions / normalized_sessions 皆不存在）。
    let normalized_rows: i64 = rusqlite::Connection::open(&db)
        .expect("open sqlite read-back")
        .query_row(
            "SELECT COUNT(*) FROM normalized_sessions WHERE session_id = ?1",
            ["s-ghost"],
            |row| row.get(0),
        )
        .expect("count normalized_sessions row");
    assert_eq!(
        normalized_rows, 0,
        "ADR 0099 阶段 0 / I3：已删会话的 normalized_sessions 行不得被旧快照提交重建"
    );

    // Assert ④：新实例的 list_sessions() 里也看不到它（用户可见面）。
    let reloaded = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    assert!(
        reloaded
            .list_sessions()
            .iter()
            .all(|overview| overview.conversation_id != "s-ghost"),
        "ADR 0099 阶段 0 / I3：已删会话不得在 list_sessions() 里复活"
    );
}

// ---------------------------------------------------------------------------
// 契约 3-b：`commit_boundary` 对已删会话「跳过且永不 INSERT」，整包 outcome 保持 Committed
// ---------------------------------------------------------------------------

/// 删除后旧快照提交：必须 `Committed`（跳过该会话），而不是 `StaleVersion` / `Failed`。
///
/// 红相（交付时）：`commit_boundary` 仍是默认 `Unsupported`，且底层 `write_full_store`
/// 无条件 `INSERT OR REPLACE` ⇒ 既拿不到 `Committed`，会话也会被复活。
#[test]
fn commit_skips_deleted_session_and_reports_committed() {
    use pony_agent_core::agent::session::{CommitOutcome, SessionBackend};

    // Arrange：A 端建会话；B 端（独立连接）在删除前读走快照。
    let dir = TempDir::new("tombstone-boundary");
    let db = dir.db();
    let mut store_a = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    store_a.append_turn(
        Some("s-tomb"),
        "墓碑用例：用户消息",
        "墓碑用例：助手回复",
        None,
        Vec::new(),
    );
    assert_eq!(session_row_count(&db, "s-tomb"), 1, "前提：会话已落盘");

    let reader = open_sqlite(&db);
    let stale = reader.load_store().expect("旧快照必须可装载");

    // A 端显式删除。
    store_a.remove_session("s-tomb");
    assert_eq!(session_row_count(&db, "s-tomb"), 0, "前提：删除已落库");

    // Act：旧快照提交。
    let outcome = reader.commit_boundary(&stale);

    // Assert ①：跳过语义 ⇒ 整包 outcome 仍是 Committed（墓碑不把整包变成 Stale/Failed）。
    assert_eq!(
        outcome,
        CommitOutcome::Committed,
        "ADR 0099 阶段 0 / I3：删除后旧快照提交必须返回 Committed（墓碑会话被跳过、永不 INSERT），\
         实际 {outcome:?}"
    );

    // Assert ②：会话在两表里都不存在（跳过 = 零 INSERT）。
    assert_eq!(
        session_row_count(&db, "s-tomb"),
        0,
        "ADR 0099 阶段 0 / I3：跳过语义下 sessions 行不得被重建"
    );
    let normalized_rows: i64 = rusqlite::Connection::open(&db)
        .expect("open sqlite read-back")
        .query_row(
            "SELECT COUNT(*) FROM normalized_sessions WHERE session_id = ?1",
            ["s-tomb"],
            |row| row.get(0),
        )
        .expect("count normalized_sessions row");
    assert_eq!(
        normalized_rows, 0,
        "ADR 0099 阶段 0 / I3：跳过语义下 normalized_sessions 行不得被重建"
    );
}
