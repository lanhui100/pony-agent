//! ADR 0099 阶段 0：SQLite 会话库的**提交边界实作**（CAS 守门 / 单实例锁 / busy 判定 /
//! 孤儿回填 / 规范化 trace 源）。
//!
//! 契约真源：`.dev-team/contracts/stage0-write-model.md` §5（E1–E8）与 §3（I1–I5）。
//!
//! 为什么独立成文件：`sqlite_session.rs` 与 `session/store.rs` 已计入
//! `scripts/file-size-baseline.json` 的文件行数棘轮（净值只许减、不许增），阶段 0 的
//! 新增实作落在这里（新文件无基线条目，且远低于 3000 行阈值），`sqlite_session.rs`
//! 只保留薄壳调用点。模块声明见 `sqlite_session.rs` 顶部
//! `#[path = "sqlite_session_commit.rs"] mod commit;`。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use rusqlite::{params, Connection, OptionalExtension};

use super::SqliteSessionBackend;
use crate::agent::session::{
    BackfillReport, CommitOutcome, PersistedStore, SessionState, SessionVersion, SessionVersionMap,
    TraceMigrationState, TurnTraceRecord,
};

/// ADR 0099 阶段 0 / I5：busy / locked 判定**只看** `rusqlite::ErrorCode`。
///
/// 现网死分支（`sqlite_session.rs` 原两处按错误消息子串匹配 "busy" 的判定）：SQLite 的
/// 锁冲突消息是 `database is locked`（`SQLITE_BUSY` / `SQLITE_LOCKED`），消息里没有
/// "busy" 这个词，字符串启发式永不命中，于是并发写被当成致命错误直接 `Failed`。
pub fn is_busy_error(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// ADR 0099 阶段 0 / I5 / NFR `retry`：busy 退避 50→100→200ms（`attempt` 从 0 起，
/// 上限 3 次由调用方的重试计数保证）。
pub(crate) fn busy_backoff_ms(attempt: u32) -> u64 {
    50u64 << attempt.min(2)
}

// ---------------------------------------------------------------------------
// E3：`sessions.version` 列（存量库幂等迁移）
// ---------------------------------------------------------------------------

/// E3 / I2：`sessions` 表新增 `version INTEGER NOT NULL DEFAULT 0`。
///
/// `CREATE TABLE IF NOT EXISTS` 不补列（同 PA-093 既有做法），存量库走
/// `ALTER TABLE ... ADD COLUMN`，列已存在时忽略错误 ⇒ 迁移幂等（重复执行 0 报错）。
/// 迁移失败（视图库等无法加列）**不得静默吞掉**：落日志 + 置降级标记，CAS 明确关闭。
pub(crate) fn ensure_version_column(conn: &Connection) -> Result<(), String> {
    match version_column_state(conn) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        // 查询本身失败（busy/locked/schema 锁）≠ 列不存在：上抛，由调用方 fail loud。
        Err(error) => return Err(error),
    }
    match conn.execute(
        "ALTER TABLE sessions ADD COLUMN version INTEGER NOT NULL DEFAULT 0",
        [],
    ) {
        Ok(_) => Ok(()),
        Err(error) => {
            // 阶段 0 只落一条日志（版本守门的降级标记由阶段 1 的写者租约承担）。
            eprintln!(
                "[pony-agent][session] commit_boundary_degraded：sessions.version 迁移失败，\
                 本次会话库退化为无版本整包写（墓碑判定不可用）：{error}"
            );
            Ok(())
        }
    }
}

/// `sessions` 是否带 `version` 列——**结构判定**（`PRAGMA table_info`），不靠错误文本。
///
/// 三态语义（关键，BLOCKING-5）：
/// - `Ok(true)`：有列，版本号可维护（墓碑判定可用）；
/// - `Ok(false)`：列**确实不存在**（PA-089 6a 的 `sessions` 视图形态 / 空表）⇒ 允许显式降级；
/// - `Err(..)`：查询本身失败（`SQLITE_BUSY` / `SQLITE_LOCKED` / schema 锁）⇒ 调用方必须
///   fail loud，**不得**当成"没有 version 列"然后无 CAS 假装成功。
pub(crate) fn version_column_state(conn: &Connection) -> Result<bool, String> {
    let mut statement = conn
        .prepare("PRAGMA table_info(sessions)")
        .map_err(|error| format!("prepare table_info(sessions): {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("query table_info(sessions): {error}"))?;
    for row in rows {
        let name = row.map_err(|error| format!("read table_info(sessions): {error}"))?;
        if name == "version" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// E1：读取版本侧表（`sessions.version`）——`load_store` / `load_store_normalized` 用它
/// 填充 `PersistedStore.versions`，`commit_boundary` 用它作**期望版本**。
pub(crate) fn load_session_versions(conn: &Connection) -> SessionVersionMap {
    let mut versions = SessionVersionMap::new();
    let mut stmt = match conn.prepare("SELECT conversation_id, version FROM sessions") {
        Ok(stmt) => stmt,
        Err(error) => {
            eprintln!(
                "[pony-agent][session] commit_boundary_degraded：版本侧表读取失败（本次快照无期望版本）：{error}"
            );
            return versions;
        }
    };
    let rows = match stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!(
                "[pony-agent][session] commit_boundary_degraded：版本侧表查询失败（本次快照无期望版本）：{error}"
            );
            return versions;
        }
    };
    for (session_id, version) in rows.flatten() {
        versions.insert(session_id, SessionVersion(version.max(0) as u64));
    }
    versions
}

pub(crate) fn read_session_version(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<u64>, String> {
    conn.query_row(
        "SELECT version FROM sessions WHERE conversation_id = ?1",
        params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|row| row.map(|version| version.max(0) as u64))
    .map_err(|error| format!("read session version: {error}"))
}

/// 单会话 blob 行写入：内容按 `session` 覆盖，`version` 单调 +1（CAS 的版本前进点）。
///
/// `sessions` 无 `version` 列（PA-089 6a 视图形态）时退化为既有 4 列 `INSERT OR REPLACE`。
pub(crate) fn upsert_session_row_bump(
    conn: &Connection,
    session_id: &str,
    session: &SessionState,
) -> Result<(), String> {
    let data = serde_json::to_string(session).map_err(|e| format!("serialize session: {e}"))?;
    let title = session.title.as_str();
    let updated_at_ms = session.updated_at_ms as i64;
    if version_column_state(conn)? {
        let updated = conn
            .execute(
                "UPDATE sessions
                    SET title = ?2, updated_at_ms = ?3, session_data = ?4, version = version + 1
                  WHERE conversation_id = ?1",
                params![session_id, title, updated_at_ms, data],
            )
            .map_err(|e| format!("upsert session row: {e}"))?;
        if updated > 0 {
            return Ok(());
        }
    }
    conn.execute(
        "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, title, updated_at_ms, data],
    )
    .map_err(|e| format!("insert session row: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// E5：单实例文件锁（`<db>.lock`，跨进程独占；同进程多实例共享）
// ---------------------------------------------------------------------------

/// 进程内锁登记表：`flock` 按 open file description 记账，同进程重复 `open` 会自锁，
/// 故同一 db 路径在本进程内共享同一把锁（一个进程 = 一个 writer 实例语义）。
///
/// 用 `Weak` 持有：锁的存活由 `SqliteSessionBackend` 侧持有的 `Arc` 决定（backend drop
/// ⇒ 锁释放 ⇒ 登记项自然失效），登记表因此**有界**（每条 acquire 顺带 prune 死项）。
static LOCK_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<std::fs::File>>>> = OnceLock::new();

/// E5 / NFR `concurrency_lock`：尝试独占 `<db>.lock`。
///
/// - `Ok(Some(guard))`：已持有（本进程），调用方必须把 `guard` 存住直到 backend drop；
/// - `Ok(None)`：其他进程独占（或锁文件不可独占）⇒ 调用方**必须**明确降级只读
///   （`PRAGMA query_only=ON` + 结构化日志），不得忙等、不得 panic，也不得继续写；
/// - `Err`：连锁文件都建不出来（路径不可写）⇒ 同样按降级处理由调用方决定。
pub(crate) fn acquire_single_instance_lock(
    db_path: &Path,
) -> Result<Option<Arc<std::fs::File>>, String> {
    let registry = LOCK_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let key = db_path.to_path_buf();
    let mut held = registry
        .lock()
        .map_err(|error| format!("lock registry poisoned: {error}"))?;
    held.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = held.get(&key).and_then(Weak::upgrade) {
        return Ok(Some(existing));
    }
    let lock_path = PathBuf::from(format!("{}.lock", db_path.display()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("open lock file {}: {error}", lock_path.display()))?;
    if try_lock_exclusive(&file) {
        let guard = Arc::new(file);
        held.insert(key, Arc::downgrade(&guard));
        return Ok(Some(guard));
    }
    eprintln!(
        "[pony-agent][session] single_instance_lock_denied: {} 已被其他进程独占；\
         本实例降级只读（不并发写、不静默覆盖）",
        lock_path.display()
    );
    Ok(None)
}

/// 尝试非阻塞独占锁。返回 `true` = 已持有；`false` = 抢不到（含非 EWOULDBLOCK 的异常
/// 错误——一律降级只读并落日志，硬失败会让用户连"只读看历史"都做不到）。
impl SqliteSessionBackend {
    /// E5：降级只读后**重新尝试**拿实例锁（非阻塞一次）。成功 ⇒ 恢复可写。
    ///
    /// `PRAGMA query_only` 本身不可逆，因此恢复必须显式 `OFF`——否则持锁进程退出后本实例
    /// 会永久只读（BLOCKING-6 的第二个洞）。
    pub(crate) fn try_recover_instance_lock(&self) {
        if !self.read_only.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let guard = match acquire_single_instance_lock(&self.db_path) {
            Ok(Some(guard)) => guard,
            _ => return,
        };
        {
            let Ok(mut held) = self.instance_lock.lock() else {
                return;
            };
            *held = Some(guard);
        }
        self.read_only
            .store(false, std::sync::atomic::Ordering::SeqCst);
        // 连接锁单独取，避免与 `connection()` 的锁顺序交叉。
        if let Ok(slot) = self.connection.lock() {
            if let Some(conn) = slot.as_ref() {
                match conn.execute_batch("PRAGMA query_only=OFF;") {
                    Ok(()) => eprintln!(
                        "[pony-agent][session] single_instance_lock_recovered：{} 已恢复可写",
                        self.db_path.display()
                    ),
                    Err(error) => {
                        self.read_only
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        eprintln!(
                            "[pony-agent][session] single_instance_lock_recover_failed：{} 仍只读：{error}",
                            self.db_path.display()
                        );
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File) -> bool {
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return true;
    }
    let error = std::io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(code) if code == libc::EWOULDBLOCK) {
        eprintln!(
            "[pony-agent][session] single_instance_lock_error：flock 异常，按抢不到处理并降级只读：{error}"
        );
    }
    false
}

#[cfg(windows)]
fn try_lock_exclusive(file: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let handle = file.as_raw_handle() as HANDLE;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let flags = LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY;
    // 锁住全部字节区间（0xFFFFFFFF/0xFFFFFFFF = 整个文件）。
    let locked = unsafe { LockFileEx(handle, flags, 0, u32::MAX, u32::MAX, &mut overlapped) };
    if locked != 0 {
        return true;
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_LOCK_VIOLATION as i32) {
        eprintln!(
            "[pony-agent][session] single_instance_lock_error：LockFileEx 异常，按抢不到处理并降级只读：{error}"
        );
    }
    false
}

#[cfg(not(any(unix, windows)))]
fn try_lock_exclusive(_file: &std::fs::File) -> bool {
    true
}

// ---------------------------------------------------------------------------
// E2 / E3 / E4：提交边界（BEGIN IMMEDIATE + 逐行真 CAS + 墓碑跳过 + 版本自增）
// ---------------------------------------------------------------------------

/// 单次 `persist_commands_batch` 尝试的结果：区分「锁冲突（busy，可退避）」与终态。
pub(crate) enum BatchAttempt {
    Done(crate::agent::session::PersistCommandOutcome),
    Busy(String),
}

/// 单次 `persist_commands_batch` 尝试（`BEGIN IMMEDIATE` 内）。busy 与致命错误分开上报。
///
/// 放在本文件（而非 trait impl 里）是为了不撑大 `sqlite_session.rs` 的行数额度。
pub(crate) fn persist_commands_once(
    backend: &super::SqliteSessionBackend,
    conn: &Connection,
    commands: &[crate::agent::session::PersistCommand],
) -> BatchAttempt {
    use crate::agent::session::PersistCommandOutcome;
    let tx = match rusqlite::Transaction::new_unchecked(
        conn,
        rusqlite::TransactionBehavior::Immediate,
    ) {
        Ok(tx) => tx,
        Err(error) => {
            if is_busy_error(&error) {
                return BatchAttempt::Busy(format!("begin tx: {error}"));
            }
            eprintln!("[pony-agent][session] SQLite batch command begin tx error: {error}");
            return BatchAttempt::Done(PersistCommandOutcome::Failed);
        }
    };
    for command in commands {
        if let Err(error) = backend.apply_persist_command_tx(&tx, command) {
            eprintln!("[pony-agent][session] SQLite batch command error: {error}");
            return BatchAttempt::Done(PersistCommandOutcome::Failed);
        }
    }
    match tx.commit() {
        Ok(()) => BatchAttempt::Done(PersistCommandOutcome::Succeeded),
        Err(error) => {
            if is_busy_error(&error) {
                return BatchAttempt::Busy(format!("commit: {error}"));
            }
            eprintln!("[pony-agent][session] SQLite batch command commit error: {error}");
            BatchAttempt::Done(PersistCommandOutcome::Failed)
        }
    }
}

/// 提交边界内部失败：区分「锁冲突（busy，可退避重试）」与「其他（不可重试）」。
struct CommitFailure {
    message: String,
    busy: bool,
}

impl CommitFailure {
    fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            busy: false,
        }
    }
}

impl SqliteSessionBackend {
    /// E1：快照的「元数据尾巴」（6 个 `store_metadata` 读 + 版本侧表）。两个 loader
    /// （legacy `load_store` / `load_store_normalized`）共用，调用方用结构体更新语法覆盖
    /// `sessions`——同时把行数额度留给 file-size 棘轮。
    pub(crate) fn persisted_store_tail(&self, conn: &Connection) -> PersistedStore {
        PersistedStore {
            sessions: HashMap::new(),
            attachment_assets: self.read_metadata(conn, "attachment_assets"),
            session_attachment_index: self.read_metadata(conn, "session_attachment_index"),
            mcp_source_snapshots: self.read_metadata(conn, "mcp_source_snapshots"),
            skill_source_snapshots: self.read_metadata(conn, "skill_source_snapshots"),
            workspaces: self.read_metadata(conn, "workspaces"),
            path_authorizations: self.read_metadata(conn, "path_authorizations.v1"),
            versions: load_session_versions(conn),
        }
    }

    /// E2：`SessionBackend::commit_boundary` 的实作（薄壳在 trait impl 内调用它）。
    ///
    /// - 整包写在一个 `BEGIN IMMEDIATE` 事务里（原子性收益）；
    /// - busy 退避在**不持连接锁**的前提下进行：每次尝试单独取连接、失败即释放，再 sleep；
    /// - 单次尝试的本地写锁等待预算 5000ms（`busy_timeout`），重试上限 3 次。
    ///
    /// 为什么不是 3000ms：NFR `external_call_timeout_ms=3000` 约束的是**外部调用**
    /// （网络 / HTTP / 跨进程 RPC）。SQLite 的 `busy_timeout` 是**本地写锁等待**——无网络
    /// IO、失败可恢复；压到 3000 会让"慢盘 / 备份进程占锁"变成用户可见的保存失败。
    pub(crate) fn commit_boundary_stage0(&self, store: &PersistedStore) -> CommitOutcome {
        self.try_recover_instance_lock();
        const LOCK_WAIT_BUDGET_MS: u64 = 5000;
        let started = std::time::Instant::now();
        let mut attempt = 0u32;
        loop {
            let attempt_result = {
                let slot = match self.connection() {
                    Ok(slot) => slot,
                    Err(error) => {
                        eprintln!(
                            "[pony-agent][session] commit_boundary_failed: open error: {error}"
                        );
                        return CommitOutcome::Failed;
                    }
                };
                let conn = slot.as_ref().expect("connection initialized");
                let _ = conn.busy_timeout(std::time::Duration::from_millis(LOCK_WAIT_BUDGET_MS));
                self.commit_store_once(conn, store)
            };
            match attempt_result {
                Ok(outcome) => return outcome,
                Err(failure) if failure.busy => {
                    // NFR `retry.max_attempts=3`：最多 3 次尝试（attempt 0..=2）。
                    if attempt >= 2 {
                        eprintln!(
                            "[pony-agent][session] commit_boundary_failed: busy 重试耗尽（attempt={attempt}，共 {}ms）：{}",
                            elapsed_ms(&started),
                            failure.message
                        );
                        return CommitOutcome::Failed;
                    }
                    let backoff = busy_backoff_ms(attempt);
                    eprintln!(
                        "[pony-agent][session] busy_retry: commit_boundary 第 {} 次重试（{backoff}ms 退避）：{}",
                        attempt + 1,
                        failure.message
                    );
                    std::thread::sleep(std::time::Duration::from_millis(backoff));
                    attempt += 1;
                }
                Err(failure) => {
                    eprintln!(
                        "[pony-agent][session] commit_boundary_failed: {}",
                        failure.message
                    );
                    return CommitOutcome::Failed;
                }
            }
        }
    }

    /// 一次提交尝试：`BEGIN IMMEDIATE` + 逐行真 CAS（`UPDATE … WHERE version = ?expected
    /// RETURNING version`），全部成功才 `COMMIT`。
    ///
    /// DEFERRED 事务在这里是**错的**：先读版本再写会跨在"读"与"升级为写锁"之间，
    /// 并发写者一插入就 `SQLITE_BUSY_SNAPSHOT`，整包被静默判 `Failed`。
    fn commit_store_once(
        &self,
        conn: &Connection,
        store: &PersistedStore,
    ) -> Result<CommitOutcome, CommitFailure> {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| CommitFailure {
                    message: format!("begin immediate tx: {error}"),
                    busy: is_busy_error(&error),
                })?;
        let has_version = version_column_state(&tx).map_err(CommitFailure::other)?;
        if !has_version {
            // 列**确实不存在**（PA-089 6a 的 sessions 视图形态）：退化为无版本整包写，留痕。
            eprintln!(
                "[pony-agent][session] commit_boundary_degraded：sessions 无 version 列（视图形态），\
                 本次整包写不做墓碑判定"
            );
            self.write_store_tx(&tx, store, &HashSet::new())
                .map_err(CommitFailure::other)?;
            tx.commit().map_err(|error| CommitFailure {
                message: format!("commit: {error}"),
                busy: is_busy_error(&error),
            })?;
            return Ok(CommitOutcome::Committed);
        }
        // 阶段 0 语义（Lead 裁决：版本 CAS 整体移交阶段 1）：**不做版本否决**，最后写入者胜；
        // 版本侧表只用于一件事——判定「装载时存在、现在行已不存在」的会话是**墓碑**，
        // 跳过且永不 INSERT（I3：旧快照不得复活已删会话）。
        let mut tombstoned: HashSet<String> = HashSet::new();
        for (session_id, session) in &store.sessions {
            if store.versions.contains_key(session_id)
                && !session_row_exists(&tx, session_id).map_err(CommitFailure::other)?
            {
                tombstoned.insert(session_id.clone());
                eprintln!(
                    "[pony-agent][session] commit_boundary_tombstone_skip: session={session_id}（装载时存在、库内行已删除）"
                );
                continue;
            }
            let prepared = prepare_session_for_row(session_id, session);
            upsert_session_row_bump(&tx, session_id, &prepared).map_err(CommitFailure::other)?;
        }
        self.write_traces_and_metadata_tx(&tx, store, &tombstoned)
            .map_err(CommitFailure::other)?;
        tx.commit().map_err(|error| CommitFailure {
            message: format!("commit: {error}"),
            busy: is_busy_error(&error),
        })?;
        Ok(CommitOutcome::Committed)
    }

    /// 全库落盘主体（原 `write_full_store` 函数体；ADR 0099 阶段 0 迁入本文件以守住
    /// `sqlite_session.rs` 的文件行数棘轮）。**无版本守门**：`skip` = 墓碑/已删会话集合。
    pub(crate) fn write_store_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        store: &PersistedStore,
        skip: &HashSet<String>,
    ) -> Result<(), String> {
        for (session_id, session) in &store.sessions {
            if skip.contains(session_id) {
                continue;
            }
            let prepared = prepare_session_for_row(session_id, session);
            upsert_session_row_bump(tx, session_id, &prepared)?;
        }
        self.write_traces_and_metadata_tx(tx, store, skip)
    }

    /// trace 表 + `store_metadata` 写入（`write_store_tx` 与 CAS 提交路径共用）。
    fn write_traces_and_metadata_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        store: &PersistedStore,
        skip: &HashSet<String>,
    ) -> Result<(), String> {
        for (session_id, session) in &store.sessions {
            if skip.contains(session_id) {
                continue;
            }
            // 始终写 trace 表，保持 blob 与 trace 表同步（PA-088：authoritative 会话跳过
            // ——其 blob 已被 `session_state_for_backend` 剥离 trace，此处若用剥离后的空
            // trace 替换表会清空数据；表已由 `save_to_backend` 在剥离前写入）。
            if session.trace_migration_state != TraceMigrationState::TraceTableAuthoritative {
                self.replace_session_traces_tx(tx, session_id, &session.turn_trace_history)?;
            }
        }
        // ADR 0094：**不再**在这里删除「快照里没有」的会话（显式删除只走 `remove_session`）。
        self.write_store_metadata_tx(tx, store)
    }

    fn write_store_metadata_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        store: &PersistedStore,
    ) -> Result<(), String> {
        let metadata_entries: [(&str, Option<String>); 6] = [
            (
                "attachment_assets",
                serde_json::to_string(&store.attachment_assets).ok(),
            ),
            (
                "session_attachment_index",
                serde_json::to_string(&store.session_attachment_index).ok(),
            ),
            (
                "mcp_source_snapshots",
                serde_json::to_string(&store.mcp_source_snapshots).ok(),
            ),
            (
                "skill_source_snapshots",
                serde_json::to_string(&store.skill_source_snapshots).ok(),
            ),
            ("workspaces", serde_json::to_string(&store.workspaces).ok()),
            (
                "path_authorizations.v1",
                serde_json::to_string(&store.path_authorizations).ok(),
            ),
        ];
        let mut statement = tx
            .prepare("INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?1, ?2)")
            .map_err(|e| format!("prepare meta insert: {e}"))?;
        for (key, value) in &metadata_entries {
            if let Some(value) = value {
                statement
                    .execute(params![key, value])
                    .map_err(|e| format!("insert metadata: {e}"))?;
            }
        }
        Ok(())
    }
}

/// blob 写前的 id 归一（行键 = `conversation_id`），与既有 `write_full_store` 行为一致。
fn prepare_session_for_row(session_id: &str, session: &SessionState) -> SessionState {
    let mut normalized = session.clone();
    if normalized.conversation_id != session_id {
        eprintln!(
            "[pony-agent][session] SQLite save_store normalizing conversation_id: key={} payload={}",
            session_id, normalized.conversation_id
        );
        normalized.conversation_id = session_id.to_string();
    }
    normalized
}

/// `sessions` 是否已有该会话行（墓碑判定的唯一依据）。
fn session_row_exists(conn: &Connection, session_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE conversation_id = ?1)",
        params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|error| format!("check session row: {error}"))
}

/// 已耗时（毫秒），用于把 busy 重试收敛到 NFR 总预算。
fn elapsed_ms(started: &std::time::Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

// ---------------------------------------------------------------------------
// E7：resume / 规范化读路径的 trace 源 = `normalized_turn_traces`
// ---------------------------------------------------------------------------

impl SqliteSessionBackend {
    /// E7 / I4：规范化读路径的 trace 主源是 `normalized_turn_traces`（`raw_json` 列即整行
    /// trace）。legacy `session_turn_traces` 只对**规范化侧无行**的会话兜底，不再作为
    /// resume 的主源——否则「只有 normalized_turn_traces 的孤儿会话」恢复即丢 trace。
    pub(crate) fn read_traces_normalized(
        &self,
        conn: &Connection,
    ) -> Result<HashMap<String, Vec<TurnTraceRecord>>, String> {
        let mut traces = self.read_all_normalized_traces(conn)?;
        for (session_id, legacy_rows) in self.read_all_session_traces(conn)? {
            traces.entry(session_id).or_insert(legacy_rows);
        }
        Ok(traces)
    }

    fn read_all_normalized_traces(
        &self,
        conn: &Connection,
    ) -> Result<HashMap<String, Vec<TurnTraceRecord>>, String> {
        let mut statement = conn
            .prepare(
                "SELECT session_id, raw_json FROM normalized_turn_traces
                 ORDER BY session_id, trace_order ASC, updated_at_ms ASC, turn_id ASC",
            )
            .map_err(|e| format!("prepare normalized trace load: {e}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| format!("query normalized trace load: {e}"))?;
        let mut by_session: HashMap<String, Vec<TurnTraceRecord>> = HashMap::new();
        for row in rows {
            let (session_id, raw) = row.map_err(|e| format!("read normalized trace row: {e}"))?;
            let Some(raw) = raw else { continue };
            match serde_json::from_str::<TurnTraceRecord>(&raw) {
                Ok(trace) => by_session
                    .entry(session_id.clone())
                    .or_default()
                    .push(self.hydrate_trace_observation(conn, &session_id, trace)),
                Err(error) => eprintln!(
                    "[pony-agent][session] malformed normalized trace row for session {}: {}",
                    session_id, error
                ),
            }
        }
        Ok(by_session)
    }
}

// ---------------------------------------------------------------------------
// E8：孤儿会话回填（源只能是 `turn_events ∪ normalized_*`）
// ---------------------------------------------------------------------------

impl SqliteSessionBackend {
    /// E8 / I4：把「只有 `normalized_*` / `turn_events`、没有 legacy `sessions` blob 行」的
    /// 孤儿会话回填成 blob 行。
    ///
    /// 硬约束：回填源**只能**是规范化表与事件流，**绝不**以 `sessions` 表为源（blob 是待
    /// 废弃的快照，用它回填等于把残缺固化为权威）。无法回填者只标 `retired`，不删行。
    pub fn backfill_orphan_sessions(&self) -> Result<BackfillReport, String> {
        self.backfill_orphans(false)
    }

    /// E8 的**只读预演**：与 [`Self::backfill_orphan_sessions`] 同判定口径，但**不写库**
    /// （维护入口 `ponycli sessions backfill --dry-run`）。
    pub fn backfill_orphan_sessions_dry_run(&self) -> Result<BackfillReport, String> {
        self.backfill_orphans(true)
    }

    fn backfill_orphans(&self, dry_run: bool) -> Result<BackfillReport, String> {
        let slot = self.connection()?;
        let conn = slot
            .as_ref()
            .ok_or_else(|| "connection not initialized".to_string())?;
        let sessions = self
            .load_store_normalized(conn)
            .ok_or_else(|| "normalized load failed".to_string())?
            .sessions;
        let mut report = BackfillReport::default();
        let mut rebuilt: Vec<(String, SessionState)> = Vec::new();
        for (session_id, session) in &sessions {
            let blob_rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE conversation_id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("count blob rows: {e}"))?;
            if blob_rows > 0 {
                continue;
            }
            let normalized_messages: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM normalized_messages WHERE session_id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("count normalized messages: {e}"))?;
            let event_rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM turn_events WHERE session_id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("count turn events: {e}"))?;
            if normalized_messages == 0 && event_rows == 0 {
                // 无源可回填：显式退役（I4：只标记不删行），由 UI 明示。
                report.retired.push(session_id.clone());
                continue;
            }
            if session.history.len() as i64 != normalized_messages {
                // 校验和不一致：进报告（不静默覆盖语义），仍以规范化行为准写回。
                report.checksum_mismatch.push(session_id.clone());
            }
            rebuilt.push((session_id.clone(), session.clone()));
        }
        if !dry_run && !rebuilt.is_empty() {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for (session_id, session) in &rebuilt {
                let mut session = session.clone();
                session.conversation_id = session_id.clone();
                // 走统一单行写：新行 version 从 0 起，且绝不让既有行版本回退。
                upsert_session_row_bump(&tx, session_id, &session)?;
            }
            tx.commit().map_err(|e| format!("commit: {e}"))?;
        }
        report.backfilled = rebuilt
            .into_iter()
            .map(|(session_id, _)| session_id)
            .collect();
        eprintln!(
            "[pony-agent][session] orphan_backfill_summary: dry_run={dry_run} backfilled={} retired={} checksum_mismatch={}",
            report.backfilled.len(),
            report.retired.len(),
            report.checksum_mismatch.len()
        );
        Ok(report)
    }
}
