//! ADR 0099 阶段 0 · L3-P 审查回归：CAS 版本语义的三条硬不变量（BLOCKING-1 / -2 / -5）。
//!
//! 这三条是本轮审查判 FAIL 的数据损坏级缺陷，冻结的 7 个套件覆盖不到，故独立成文件：
//!
//! 1. **版本单调（BLOCKING-1）**：任何写 `sessions` 行的路径都必须让 `version` 前进。
//!    历史缺陷：`upsert_session` 用的是 4 列 `INSERT OR REPLACE`（不含 version），REPLACE
//!    会把 version 重置为 DEFAULT 0 ⇒ `expected=0` 的旧快照在单行写之后**再次通过 CAS**
//!    并覆盖新 turn（ABA）。
//! 2. **StaleVersion 不可重试（BLOCKING-2）**：旧快照提交被拒后**不得**换上新版本号重投
//!    （那是 LWW 覆盖，比不重试更糟）。反例：A 写 turn2、B 持 turn1 旧快照提交 ⇒ A 的
//!    turn2 必须仍在（`history` 4 条、`turnCount` 2）。
//! 3. **降级只读零写入（BLOCKING-5）**：抢不到 `sessions.db.lock` 的实例，任何写路径
//!    （含 `pending_asks` 旁路连接）都不得在库里新增行。
//!
//! 隔离：临时目录 + 临时 db；绝不触碰 `~/.local/share/PonyAgent/sessions.db`。

use pony_agent_core::agent::session::{
    CommitOutcome, SeparateTraceTableMode, SessionBackend, SessionStore,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 隔离夹具
// ---------------------------------------------------------------------------

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

/// 读库内 `sessions.version`（物理真值，不信内存态）。
fn db_version(db: &Path, session_id: &str) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    conn.query_row(
        "SELECT version FROM sessions WHERE conversation_id = ?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .expect("sessions 行必须存在（版本断言）")
}

/// 读库内该会话 blob 的 `history` 条数。
fn blob_history_len(db: &Path, session_id: &str) -> usize {
    blob_json(db, session_id)
        .get("history")
        .and_then(|history| history.as_array())
        .map(|history| history.len())
        .expect("session blob 必须带 history 数组")
}

/// 读库内该会话 blob 的 `turnCount`。
fn blob_turn_count(db: &Path, session_id: &str) -> u64 {
    blob_json(db, session_id)
        .get("turnCount")
        .and_then(|count| count.as_u64())
        .expect("session blob 必须带 turnCount")
}

fn blob_json(db: &Path, session_id: &str) -> serde_json::Value {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    let data: String = conn
        .query_row(
            "SELECT session_data FROM sessions WHERE conversation_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .expect("sessions 行必须存在");
    serde_json::from_str(&data).expect("session_data 必须是 JSON")
}

/// 全库行数合计（用于「降级只读零写入」的门禁：任何表新增行都会被抓到）。
fn total_rows(db: &Path) -> i64 {
    let conn = rusqlite::Connection::open(db).expect("open sqlite read-back");
    let tables: Vec<String> = {
        let mut statement = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .expect("prepare table list");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query table list");
        rows.filter_map(Result::ok).collect()
    };
    tables
        .iter()
        .map(|table| {
            conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0)
        })
        .sum()
}

/// 在**本进程之外的身份**下抢住 `<db>.lock`：直接 `flock` 一个裸 fd，绕过后端内部的
/// 进程内锁登记表 ⇒ 后端随后 acquire 必然拿到 EWOULDBLOCK，等价于"另一进程已独占"。
fn hold_instance_lock_externally(db: &Path) -> std::fs::File {
    let lock_path = PathBuf::from(format!("{}.lock", db.display()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .expect("open lock file");
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(result, 0, "测试必须能抢到 lock 文件（否则用例前提不成立）");
    }
    file
}

// ---------------------------------------------------------------------------
// BLOCKING-1：版本单调 + 旧版本号不可复通过
// ---------------------------------------------------------------------------

#[test]
fn sessions_version_is_monotonic_and_old_snapshot_cannot_recommit() {
    // Arrange
    let dir = TempDir::new("monotonic");
    let db = dir.db();
    let mut store = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    store.append_turn(Some("s-mono"), "第一问", "第一答", None, Vec::new());

    let reader = open_sqlite(&db);
    let stale = reader.load_store().expect("旧快照必须可装载");
    let v_after_first = db_version(&db, "s-mono");

    // Act ①：第二轮 turn 走单行写路径（persist_session_with_trace_mutation）。
    store.append_turn(Some("s-mono"), "第二问", "第二答", None, Vec::new());
    let v_after_second = db_version(&db, "s-mono");

    // Assert ①：单行写必须让 version 严格前进。
    assert!(
        v_after_second > v_after_first,
        "BLOCKING-1：单行写（append_turn 路径）必须让 sessions.version 前进，\
         实际 {v_after_first} → {v_after_second}（版本停更/回退 = 旧快照可再次通过 CAS）"
    );

    // Act ②：直接打 trait 的 `upsert_session`（历史缺陷正是这条 4 列 INSERT OR REPLACE）。
    let session = reader
        .load_normalized_session("s-mono")
        .expect("规范化行存在 ⇒ 必须能重建会话");
    assert!(
        reader.upsert_session("s-mono", &session),
        "upsert_session 必须成功"
    );
    let v_after_upsert = db_version(&db, "s-mono");

    // Assert ②：upsert_session 同样不得让版本回退（旧实现 REPLACE 会重置为 0 ⇒ ABA）。
    assert!(
        v_after_upsert > v_after_second,
        "BLOCKING-1：`upsert_session` 不得用不含 version 的 INSERT OR REPLACE（会把版本\
         重置为 0 造成 ABA）；实际 {v_after_second} → {v_after_upsert}"
    );

    // Assert ③：持旧版本号的快照提交必须被拒，且库内内容不变。
    match reader.commit_boundary(&stale) {
        CommitOutcome::StaleVersion { expected, actual } => assert!(
            expected < actual,
            "StaleVersion 必须携带 expected < actual（实际 {expected} / {actual}）"
        ),
        other => panic!(
            "BLOCKING-1：旧版本号快照提交必须返回 StaleVersion，实际 {other:?}；\
             expected=0 若被重置成库内版本就会「复通过」并覆盖新 turn"
        ),
    }
    assert_eq!(
        blob_history_len(&db, "s-mono"),
        4,
        "StaleVersion 必须零写入：旧快照的 2 条消息不得覆盖库内 4 条"
    );
}

// ---------------------------------------------------------------------------
// BLOCKING-2：StaleVersion 不重投旧快照（反例：A 的 turn2 必须活下来）
// ---------------------------------------------------------------------------

#[test]
fn stale_snapshot_commit_never_overwrites_newer_turn() {
    // Arrange
    let dir = TempDir::new("lww");
    let db = dir.db();
    let workspace_root = dir.path.join("workspace-root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");

    // A 写 turn1
    let mut store_a = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    store_a.append_turn(Some("s-lww"), "第一问", "第一答", None, Vec::new());
    assert_eq!(blob_history_len(&db, "s-lww"), 2, "前提：turn1 已落盘");

    // B 在 A 写 turn2 **之前**装载旧快照（含 turn1）
    let mut store_b = SessionStore::with_backend(Box::new(open_sqlite(&db)));

    // A 写 turn2 ⇒ 库内版本前进
    store_a.append_turn(Some("s-lww"), "第二问", "第二答", None, Vec::new());
    assert_eq!(blob_history_len(&db, "s-lww"), 4, "前提：turn2 已落盘");
    assert_eq!(blob_turn_count(&db, "s-lww"), 2, "前提：turnCount 推进到 2");

    // Act：B 用旧快照做一次全库落盘（create_workspace ⇒ save_to_backend ⇒ 提交边界）
    store_b
        .create_workspace("lww-ws", workspace_root.to_str().expect("utf-8 root"))
        .expect("create_workspace 本身不报错（提交结果由提交边界裁决）");

    // Assert：A 的 turn2 必须仍在——旧快照不得以任何方式重投。
    assert_eq!(
        blob_history_len(&db, "s-lww"),
        4,
        "BLOCKING-2：旧快照提交不得覆盖 A 的 turn2（history 必须仍是 4 条）"
    );
    assert_eq!(
        blob_turn_count(&db, "s-lww"),
        2,
        "BLOCKING-2：turnCount 不得回退到 1"
    );
}

// ---------------------------------------------------------------------------
// BLOCKING-5：降级只读实例零写入（含 ask store 旁路连接）
// ---------------------------------------------------------------------------

#[test]
fn lock_denied_instance_writes_nothing_including_ask_store() {
    // Arrange：先正常建库（短命 backend，drop 后锁释放）。
    let dir = TempDir::new("readonly");
    let db = dir.db();
    let workspace_root = dir.path.join("workspace-root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    {
        let backend = open_sqlite(&db);
        assert!(backend.load_store().is_some(), "前提：临时库可初始化");
    }
    let rows_before = total_rows(&db);

    // 外部持有实例锁 ⇒ 之后的实例必然降级只读。
    let _holder = hold_instance_lock_externally(&db);

    // Act：降级实例尝试所有写路径。
    let mut store = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    assert!(
        store.ask_store().is_none(),
        "BLOCKING-5：降级只读实例不得打开 ask store（同库第二条写连接会击穿单实例锁）"
    );
    store.append_turn(Some("s-ro"), "只读探针", "只读探针", None, Vec::new());
    let _ = store.create_workspace("ro-ws", workspace_root.to_str().expect("utf-8 root"));

    // Assert：库内零新增行（任何表、任何旁路连接）。
    let rows_after = total_rows(&db);
    assert_eq!(
        rows_after, rows_before,
        "BLOCKING-5：抢不到单实例锁的实例必须零写入，实际行数 {rows_before} → {rows_after}"
    );
}

// ---------------------------------------------------------------------------
// BLOCKING-6 附带：降级只读可在锁释放后**恢复可写**（`query_only` 本身不可逆）
// ---------------------------------------------------------------------------

#[test]
fn read_only_instance_recovers_after_lock_is_released() {
    // Arrange：建库 + 外部持锁 ⇒ 新实例降级只读。
    let dir = TempDir::new("recover");
    let db = dir.db();
    {
        let backend = open_sqlite(&db);
        assert!(backend.load_store().is_some(), "前提：临时库可初始化");
    }
    let holder = hold_instance_lock_externally(&db);
    let mut store = SessionStore::with_backend(Box::new(open_sqlite(&db)));
    store.append_turn(Some("s-rec"), "只读期提问", "只读期回答", None, Vec::new());
    let rows_while_degraded = total_rows(&db);

    // Act：释放外部锁后，同一个实例再走一次写路径（应自行恢复 flock + query_only=OFF）。
    drop(holder);
    store.append_turn(Some("s-rec"), "恢复期提问", "恢复期回答", None, Vec::new());

    // Assert：恢复后确实写进去了（同一实例、不是新建 backend）。
    let rows_after_recovery = total_rows(&db);
    assert!(
        rows_after_recovery > rows_while_degraded,
        "BLOCKING-6：锁释放后同一实例必须能恢复可写（query_only 需显式 OFF），\
         实际行数 {rows_while_degraded} → {rows_after_recovery}"
    );
    assert!(
        blob_history_len(&db, "s-rec") >= 2,
        "恢复后该会话必须真的落盘（history ≥ 2）"
    );
}
