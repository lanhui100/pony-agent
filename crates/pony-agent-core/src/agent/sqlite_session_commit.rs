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
use std::sync::{Arc, Mutex, OnceLock};

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
pub(crate) fn ensure_version_column(conn: &Connection) -> Result<(), String> {
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN version INTEGER NOT NULL DEFAULT 0",
        [],
    );
    Ok(())
}

/// `sessions` 是否带 `version` 列。PA-089 6a 的 `sessions` 视图只有 4 列，写/读路径需降级。
fn has_version_column(conn: &Connection) -> bool {
    conn.prepare("SELECT version FROM sessions LIMIT 0").is_ok()
}

/// E1：读取版本侧表（`sessions.version`）——`load_store` / `load_store_normalized` 用它
/// 填充 `PersistedStore.versions`，`commit_boundary` 用它作**期望版本**。
pub(crate) fn load_session_versions(conn: &Connection) -> SessionVersionMap {
    let mut versions = SessionVersionMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT conversation_id, version FROM sessions") else {
        return versions;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) else {
        return versions;
    };
    for (session_id, version) in rows.flatten() {
        versions.insert(session_id, SessionVersion(version.max(0) as u64));
    }
    versions
}

fn read_session_version(conn: &Connection, session_id: &str) -> Result<Option<u64>, String> {
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
    if has_version_column(conn) {
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
static LOCK_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<std::fs::File>>>> = OnceLock::new();

/// E5 / NFR `concurrency_lock`：尝试独占 `<db>.lock`。
///
/// 返回 `Ok(true)` = 已持有（本进程）；`Ok(false)` = 其他进程独占 ⇒ 调用方**必须**明确
/// 降级只读（`PRAGMA query_only=ON` + 结构化日志），不得忙等、不得 panic，也不得继续写。
pub(crate) fn acquire_single_instance_lock(db_path: &Path) -> Result<bool, String> {
    let registry = LOCK_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let key = db_path.to_path_buf();
    let mut held = registry
        .lock()
        .map_err(|error| format!("lock registry poisoned: {error}"))?;
    if held.contains_key(&key) {
        return Ok(true);
    }
    let lock_path = PathBuf::from(format!("{}.lock", db_path.display()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("open lock file {}: {error}", lock_path.display()))?;
    if try_lock_exclusive(&file)? {
        held.insert(key, Arc::new(file));
        return Ok(true);
    }
    eprintln!(
        "[pony-agent][session] single_instance_lock_denied: {} 已被其他进程独占；\
         本实例降级只读（不并发写、不静默覆盖）",
        lock_path.display()
    );
    Ok(false)
}

#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File) -> Result<bool, String> {
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(code) if code == libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(format!("flock failed: {error}"))
}

#[cfg(windows)]
fn try_lock_exclusive(file: &std::fs::File) -> Result<bool, String> {
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
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        return Ok(false);
    }
    Err(format!("LockFileEx failed: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn try_lock_exclusive(_file: &std::fs::File) -> Result<bool, String> {
    Ok(true)
}

// ---------------------------------------------------------------------------
// E2 / E3 / E4：提交边界（单事务 + 逐会话 CAS + 墓碑跳过 + 版本自增）
// ---------------------------------------------------------------------------

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
    pub(crate) fn commit_boundary_stage0(&self, store: &PersistedStore) -> CommitOutcome {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] commit_boundary_failed: open error: {error}");
                return CommitOutcome::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        match self.commit_store_tx(conn, store) {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("[pony-agent][session] commit_boundary_failed: {error}");
                CommitOutcome::Failed
            }
        }
    }

    /// 单一提交边界主体。
    ///
    /// 逐会话比对 `sessions.version` 与 `store.versions`：
    /// - 不匹配 ⇒ 直接返回 `StaleVersion`，事务 `drop` 即回滚 ⇒ **整包零写入**（I2）；
    /// - 期望版本存在而库内**行已不存在** ⇒ 墓碑（其他实例已显式删除）⇒ 跳过、永不
    ///   INSERT（I3），且不改变整包 outcome（仍是 `Committed`）；
    /// - 匹配 / 无期望版本 ⇒ 写入并 `version + 1`。
    pub(crate) fn commit_store_tx(
        &self,
        conn: &Connection,
        store: &PersistedStore,
    ) -> Result<CommitOutcome, String> {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;
        if !has_version_column(&tx) {
            // PA-089 6a 的 sessions 视图形态无 version 列：无 CAS 能力，退化为旧整包语义。
            self.write_store_tx(&tx, store, &HashSet::new())?;
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            return Ok(CommitOutcome::Committed);
        }
        let mut tombstoned: HashSet<String> = HashSet::new();
        for session_id in store.sessions.keys() {
            let expected = store.versions.get(session_id).map(|version| version.0);
            let actual = read_session_version(&tx, session_id)?;
            match (expected, actual) {
                (Some(expected), Some(actual)) if expected != actual => {
                    // I2：整包零写入——不覆盖新 turn、不删除、不复活。
                    drop(tx);
                    eprintln!(
                        "[pony-agent][session] commit_boundary_stale_version: \
                         session={session_id} expected={expected} actual={actual}"
                    );
                    return Ok(CommitOutcome::StaleVersion { expected, actual });
                }
                (Some(_), None) => {
                    tombstoned.insert(session_id.clone());
                }
                _ => {}
            }
        }
        self.write_store_tx(&tx, store, &tombstoned)?;
        tx.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(CommitOutcome::Committed)
    }

    /// 全库落盘主体（原 `write_full_store` 函数体；ADR 0099 阶段 0 迁入本文件以守住
    /// `sqlite_session.rs` 的文件行数棘轮，语义不变）。
    ///
    /// `skip` = 墓碑/已删会话集合（I3：跳过、永不 INSERT）。会话行按
    /// [`upsert_session_row_bump`] 写入 ⇒ `version` 单调 +1。
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
            let mut normalized_session = session.clone();
            if normalized_session.conversation_id != *session_id {
                eprintln!(
                    "[pony-agent][session] SQLite save_store normalizing conversation_id: key={} payload={}",
                    session_id, normalized_session.conversation_id
                );
                normalized_session.conversation_id = session_id.clone();
            }
            upsert_session_row_bump(tx, session_id, &normalized_session)?;
            // 始终写 trace 表，保持 blob 与 trace 表同步（PA-088：authoritative 会话跳过
            // ——其 blob 已被 `session_state_for_backend` 剥离 trace，此处若用剥离后的空
            // trace 替换表会清空数据；表已由 `save_to_backend` 在剥离前写入）。
            if normalized_session.trace_migration_state
                != TraceMigrationState::TraceTableAuthoritative
            {
                self.replace_session_traces_tx(
                    tx,
                    session_id,
                    &normalized_session.turn_trace_history,
                )?;
            }
        }
        // ADR 0094：**不再**在这里删除「快照里没有」的会话（整包写盘的语义是「把我知道的
        // 写下去」，不是「把库对齐成我知道的样子」）。显式删除只走 `remove_session`。
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
        if !rebuilt.is_empty() {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for (session_id, session) in &rebuilt {
                let mut session = session.clone();
                session.conversation_id = session_id.clone();
                insert_backfilled_blob_row(&tx, session_id, &session)?;
            }
            tx.commit().map_err(|e| format!("commit: {e}"))?;
        }
        report.backfilled = rebuilt
            .into_iter()
            .map(|(session_id, _)| session_id)
            .collect();
        eprintln!(
            "[pony-agent][session] orphan_backfill_summary: backfilled={} retired={} checksum_mismatch={}",
            report.backfilled.len(),
            report.retired.len(),
            report.checksum_mismatch.len()
        );
        Ok(report)
    }
}

/// 回填写回：版本从 0 起（重建产物不继承任何已死版本），`sessions` 无 version 列时降级。
fn insert_backfilled_blob_row(
    conn: &Connection,
    session_id: &str,
    session: &SessionState,
) -> Result<(), String> {
    let data = serde_json::to_string(session).map_err(|e| format!("serialize session: {e}"))?;
    let sql = if has_version_column(conn) {
        "INSERT OR REPLACE INTO sessions
            (conversation_id, title, updated_at_ms, session_data, version)
         VALUES (?1, ?2, ?3, ?4, 0)"
    } else {
        "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
         VALUES (?1, ?2, ?3, ?4)"
    };
    conn.execute(
        sql,
        params![
            session_id,
            session.title,
            session.updated_at_ms as i64,
            data
        ],
    )
    .map_err(|e| format!("insert backfilled blob row: {e}"))?;
    Ok(())
}
