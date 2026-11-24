//! ADR 0099 阶段 0 · 契约 5（行为面）：`database is locked` 必须命中 busy 重试。
//!
//! 现网缺陷（ADR 0099 阶段 0 第 5 条）：`sqlite_session.rs:2661` / `:2687` 的退避重试
//! 用 `error.to_string().contains("busy")` 判定。SQLite 的实际错误是
//! **`database is locked`**（`ErrorCode::DatabaseBusy`/`DatabaseLocked`，message 里没有
//! "busy" 这个词）⇒ 判定分支**永不命中**，并发写直接 `Failed`，锁竞争被当成致命错误。
//!
//! 判定方式（行为面，不依赖尚未落码的判定函数）：用**第二个独立连接**持有写锁，
//! 让被测后端在真实 `SQLITE_BUSY` 中阻塞到 `busy_timeout`（5000ms）后收到
//! `database is locked`；随后释放锁。若 busy 判定正确，退避重试应命中并在锁释放后
//! 提交成功；若仍用 `contains("busy")`，第一次 busy 错误就会返回 `Failed`。
//!
//! 确定性说明（非盲等）：
//! - 锁持有者是真实连接上的 `BEGIN IMMEDIATE`（真实写锁），不是 sleep 猜测；
//! - 主线程等待 `BUSY_TIMEOUT + 600ms` 后才 `ROLLBACK`，保证「首次 busy 错误已发生」；
//! - 释放后重试窗口（50ms/次 × 3）覆盖 `ROLLBACK` 之后的可用写窗口。
//!
//! 精确判定函数（`is_busy_error(&rusqlite::Error) -> bool`）的单元用例见
//! `storage_stage0_busy_classification.rs`（该文件依赖尚未落码的 API）。
//!
//! 隔离：临时目录 + 临时 db；绝不触碰 `~/.local/share/PonyAgent/sessions.db`。
//!
//! 红相（交付时）：`contains("busy")` 对 `database is locked` 不匹配 ⇒ 直接
//! `PersistCommandOutcome::Failed`，断言失败。

use pony_agent_core::agent::session::{
    PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionBackend,
    TurnHistoryMessage,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `sqlite_session.rs` 连接初始化时设置的 `PRAGMA busy_timeout`（毫秒）。
/// 测试必须等过这个时长，才能确保「首次 SQLITE_BUSY 已经发生」。
const BACKEND_BUSY_TIMEOUT_MS: u64 = 5_000;
const LOCK_HOLD_MARGIN_MS: u64 = 600;

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

fn normalized_message_count(db: &std::path::Path, session_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(
        "SELECT COUNT(*) FROM normalized_messages WHERE session_id = ?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .expect("count normalized_messages")
}

#[test]
fn database_locked_is_retried_and_commit_eventually_succeeds() {
    // Arrange：后端先初始化（建 schema + WAL），再让第二个连接抢走写锁。
    let dir = TempDir::new("busy");
    let db = dir.db();
    let backend = Arc::new(SqliteSessionBackend::new_with_trace_mode(
        db.clone(),
        SeparateTraceTableMode::WriteSeparate,
    ));
    assert!(
        backend.load_store().is_some(),
        "前提：临时库可初始化（schema 就绪）"
    );

    let holder = rusqlite::Connection::open(&db).expect("open lock-holder connection");
    holder
        .busy_timeout(Duration::from_millis(0))
        .expect("holder busy_timeout=0（抢锁失败即返回，不阻塞）");
    holder
        .execute_batch(
            "BEGIN IMMEDIATE;
             INSERT OR REPLACE INTO store_metadata (key, value) VALUES ('stage0.busy.probe', '1');",
        )
        .expect("holder 必须抢到写锁（BEGIN IMMEDIATE + 真实写）");

    // Act：在被锁住的窗口内提交一条规范化命令。
    let (tx, rx) = mpsc::channel();
    let worker_backend = Arc::clone(&backend);
    let started = Instant::now();
    let worker = std::thread::spawn(move || {
        let outcome = worker_backend.persist_commands_batch(vec![PersistCommand::AppendMessage {
            epoch: 1,
            session_id: "s-busy".to_string(),
            message: TurnHistoryMessage {
                role: "user".to_string(),
                content: "busy 重试探针".to_string(),
                turn_id: Some("turn-busy".to_string()),
                ..Default::default()
            },
            ordinal: 0,
        }]);
        let _ = tx.send(outcome);
    });

    // 保证「首次 SQLITE_BUSY 已经发生」后再释放写锁。
    std::thread::sleep(Duration::from_millis(
        BACKEND_BUSY_TIMEOUT_MS + LOCK_HOLD_MARGIN_MS,
    ));
    holder
        .execute_batch("ROLLBACK;")
        .expect("release write lock");

    // 阻塞等待 worker（不再有盲等：结果必须回来）。
    let outcome = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("worker 必须在 30s 内返回提交结果");
    worker.join().expect("worker thread must not panic");
    let elapsed = started.elapsed();

    // Assert ①：锁释放后重试必须命中 ⇒ 提交成功。
    assert_eq!(
        outcome,
        PersistCommandOutcome::Succeeded,
        "ADR 0099 阶段 0：`database is locked`（SQLITE_BUSY）必须命中 busy 退避重试并在锁释放后提交成功；\
         实际 {outcome:?}（耗时 {elapsed:?}）——现实现用 `to_string().contains(\"busy\")` 判定，\
         对 `database is locked` 永不匹配"
    );

    // Assert ②：重试确实把数据写进去了（物理落盘，非「返回成功但没写」）。
    assert_eq!(
        normalized_message_count(&db, "s-busy"),
        1,
        "重试成功后命令必须真实落盘（normalized_messages 有 1 行）"
    );
}
