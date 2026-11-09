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
    BackfillReport, CommitOutcome, PersistedStore, SessionBackend, SessionState, SessionVersion,
    SessionVersionMap, TraceMigrationState, TurnTraceRecord,
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

// ---------------------------------------------------------------------------
// ADR 0099 阶段 2-D：停写整包 blob（有日志的会话只写「轻量索引行」）
// ---------------------------------------------------------------------------
//
// ## 契约（本文件是唯一落地点，所有 `sessions` 行写入口都经 `upsert_session_row_bump`）
//
// - **有日志文件**的会话：`sessions.session_data` 只写**索引字段**（会话标识 + 标题 +
//   时间 + `list_sessions` 真正消费的派生字段），不再写整包 `SessionState`。真源是日志，
//   blob 每轮重写 5 MB 的写放大由此消失。
// - **无日志文件**的会话（存量 grandfather）：**原样写整包 blob**，行为零变化。
//
// ## `session_data NOT NULL` 的处置：选择「哨兵值」而不是「放宽为可空」
//
// - `ALTER TABLE … ALTER COLUMN` 在 SQLite 里**不可用**（只能靠表重建），而表重建要复制
//   `sessions` 全表并重挂视图（PA-089 6a 的 `sessions` 是视图形态），迁移失败面远大于收益；
// - 更关键的是**旧读者安全**：放宽为 `NULL` 后，既有 `SELECT session_data` 路径会在运行中
//   撞上 `row.get::<_, String>(1)` 的类型错误（`InvalidColumnType`），那是真崩溃；
// - 哨兵方案写的是**一条合法 JSON**，且额外只加一个**未知键**
//   `"__index_only": true`（`SessionState` 未声明该字段，serde 默认忽略未知字段）⇒
//   任何既有「读 blob 反序列化」的路径都照常解析成功，只是拿到一份**只有索引字段**的状态，
//   不会 panic、不会 `Err`、不会把会话从列表里弄丢。
//
// 因此本阶段**不改 DDL**（`NOT NULL` 保留），迁移兼容性问题归零：老库（`NOT NULL` 形态）
// 打开后写入新会话不报错，新库也不存在「列可空」这一分叉。

/// 轻量索引行的标记键（`session_data` JSON 里额外的一个**未知字段**）。
pub(crate) const INDEX_ONLY_ROW_MARKER: &str = "__index_only";

/// 该 `session_data` 是否是「已停写 blob 的轻量索引行」。
///
/// 只看标记键（不整份反序列化）：索引行本身的解析开销要留在读路径之外。
pub(crate) fn is_index_only_row(raw: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => value
            .get(INDEX_ONLY_ROW_MARKER)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        // 解析不了的行不是「轻量索引行」——它是坏行（另一类问题，交给既有路径处理）。
        Err(_) => false,
    }
}

/// 轻量索引行的 payload：**只保留索引字段**，内容字段全部清空。
///
/// 字段取舍＝「谁真的在消费」：
/// - `title` / `title_override` / `summary` / `turn_count` / `last_referenced_file` /
///   `updated_at_ms` / `archived` / `workspace_id`：`list_sessions()`（侧边栏）逐字段消费；
/// - `trace_migration_state`：读路径的**投影形态**（日志里没有承载它的命令），
///   `fold::apply_index_projection` 会从索引行继承；
/// - 其余（history / traces / nodes / branches / cursor / memory / transcript）**一律不写**：
///   它们随轮次增长，正是本阶段要消灭的增长源，且日志真源已经承载。
fn index_only_row_payload(session: &SessionState) -> Result<String, String> {
    let row = SessionState {
        conversation_id: session.conversation_id.clone(),
        title: session.title.clone(),
        summary: session.summary.clone(),
        // 内容字段一律清空（见函数 doc：它们随轮次增长，日志真源已承载）。
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: session.trace_migration_state.clone(),
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: session.turn_count,
        last_referenced_file: session.last_referenced_file.clone(),
        updated_at_ms: session.updated_at_ms,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: Default::default(),
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: session.workspace_id.clone(),
        title_override: session.title_override.clone(),
        archived: session.archived,
    };
    // 标记键刻意放在**未知字段**位（`SessionState` 没有它）：老读者 serde 忽略，
    // 新读者用它区分「整包 blob」与「轻量索引行」。
    let mut value =
        serde_json::to_value(&row).map_err(|e| format!("serialize index row: {e}"))?;
    match &mut value {
        serde_json::Value::Object(map) => {
            map.insert(
                INDEX_ONLY_ROW_MARKER.to_string(),
                serde_json::Value::Bool(true),
            );
        }
        _ => return Err("index row is not a JSON object".to_string()),
    }
    serde_json::to_string(&value).map_err(|e| format!("encode index row: {e}"))
}

impl SqliteSessionBackend {
    /// 该会话是否**已有真源日志文件**（决定索引行写整包 blob 还是轻量索引行）。
    ///
    /// 直接看文件存在性，而不是看内存里的写者缓存：**跨进程/重启后**日志是否已存在
    /// 只能由磁盘回答；本批 `persist_commands_batch` 也是「先写日志（含 fsync）、
    /// 后写 SQLite 索引」，所以索引写入时点文件一定已经落盘。
    pub(crate) fn session_has_log(&self, session_id: &str) -> bool {
        // **必须跨 cwd 分组查**：分组是创建时的目录标签，用进程 cwd 会把「别的分组里
        // 已有日志」的会话误判成 grandfather，进而给它继续写整包 blob（阶段 2-D 的
        // 停写 blob 就此在跨工作区会话上失效）——ADR 0099 阶段 3-B 实测缺陷。
        self.session_log_sink.find_log_path(session_id).is_some()
    }

    /// `session_has_log` 的**公开只读**形态（运维/测试断言用；判据同上，跨 cwd 分组）。
    pub fn has_session_log(&self, session_id: &str) -> bool {
        self.session_has_log(session_id)
    }

    /// 单会话索引行写入：内容按 `session` 覆盖，`version` 单调 +1（CAS 的版本前进点）。
    ///
    /// 内容形态见模块顶部「阶段 2-D」段：有日志 ⇒ 轻量索引行；无日志 ⇒ 整包 blob
    /// （grandfather，行为与停写前逐字节一致）。
    ///
    /// `sessions` 无 `version` 列（PA-089 6a 视图形态）时退化为既有 4 列 `INSERT OR REPLACE`。
    pub(crate) fn upsert_session_row_bump(
        &self,
        conn: &Connection,
        session_id: &str,
        session: &SessionState,
    ) -> Result<(), String> {
        if self.session_has_log(session_id) {
            let payload = index_only_row_payload(session)?;
            return self.write_session_row(conn, session_id, session, &payload);
        }
        let payload =
            serde_json::to_string(session).map_err(|e| format!("serialize session: {e}"))?;
        self.write_session_row(conn, session_id, session, &payload)
    }

    /// **显式整包 blob** 写行：只给「一次性迁移 / 回填」入口用（目前是
    /// `backfill_orphan_sessions`，契约 I4 要求回填后的 blob 行与 `normalized_*` 一致）。
    ///
    /// 常规写路径**不得**调用它：停写 blob 的意义就是不让每轮 turn 重写整包快照；
    /// 回填是「把旧数据搬进索引」的维护动作，不是热路径，故显式保留整包语义。
    pub(crate) fn upsert_session_row_bump_with_full_blob(
        &self,
        conn: &Connection,
        session_id: &str,
        session: &SessionState,
    ) -> Result<(), String> {
        let payload =
            serde_json::to_string(session).map_err(|e| format!("serialize session: {e}"))?;
        self.write_session_row(conn, session_id, session, &payload)
    }

    /// 行写入本体（`version` CAS 前进点 + 4 列退化路径）。payload 由调用方决定。
    fn write_session_row(
        &self,
        conn: &Connection,
        session_id: &str,
        session: &SessionState,
        payload: &str,
    ) -> Result<(), String> {
        let data = payload;
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

    /// 与 [`Self::upsert_session_row_bump`] 同义（行写入本体见 `write_session_row`）。

    /// 把索引侧的 **observation 投影**补到日志 fold 出的 trace 行上（PA-095 #7 / 阶段 2-D）。
    ///
    /// 背景：`buildContextObservation` 的外置是「flush 与回填同事务」写在**索引行**上的
    /// （`buildContextObservations` 表 + trace 行的 `buildContextObservationRef`），
    /// 日志命令流里没有承载它的命令。停写 blob 之后，日志 fold 出的 trace 行不带 ref，
    /// 重启后 trace 面板会读不到这次调用的上下文载荷 —— 那是真数据丢失，不是「索引退为索引」。
    ///
    /// 注意方向：这里**只补索引独有的投影字段**（`buildContextObservationRef` /
    /// `buildContextObservation`），内容字段一个都不覆盖（trace 内容仍以日志为准）。
    pub(crate) fn attach_index_trace_projections(
        &self,
        sessions: &mut HashMap<String, SessionState>,
        targets: &HashSet<String>,
    ) {
        if targets.is_empty() {
            return;
        }
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!(
                    "[pony-agent][session] trace_projection_skipped: 连接不可用（{error}）"
                );
                return;
            }
        };
        let Some(conn) = slot.as_ref() else {
            return;
        };
        for session_id in targets {
            let Some(session) = sessions.get_mut(session_id.as_str()) else {
                continue;
            };
            let needs_projection = session
                .turn_trace_history
                .iter()
                .any(|trace| {
                    trace.build_context_observation_ref.is_none()
                        && trace.build_context_observation.is_none()
                });
            if !needs_projection {
                continue;
            }
            let Ok(index_traces) = self.read_session_traces(conn, session_id) else {
                continue;
            };
            if index_traces.is_empty() {
                continue;
            }
            let by_turn: HashMap<&str, &TurnTraceRecord> = index_traces
                .iter()
                .map(|trace| (trace.turn_id.as_str(), trace))
                .collect();
            for trace in &mut session.turn_trace_history {
                let Some(indexed) = by_turn.get(trace.turn_id.as_str()) else {
                    continue;
                };
                if trace.build_context_observation_ref.is_none() {
                    trace.build_context_observation_ref =
                        indexed.build_context_observation_ref.clone();
                }
                if trace.build_context_observation.is_none() {
                    trace.build_context_observation =
                        indexed.build_context_observation.clone();
                }
            }
        }
    }

    /// 索引里「已停写 blob 的轻量行」的会话 id（`instr` 前缀过滤，不整份反序列化）。
    fn index_only_row_ids(conn: &Connection) -> Result<Vec<String>, String> {
        let mut statement = conn
            .prepare(
                "SELECT conversation_id FROM sessions WHERE instr(session_data, ?1) > 0",
            )
            .map_err(|e| format!("prepare index-only row scan: {e}"))?;
        let needle = format!("\"{INDEX_ONLY_ROW_MARKER}\"");
        let rows = statement
            .query_map(params![needle], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query index-only rows: {e}"))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| format!("read index-only row id: {e}"))?);
        }
        Ok(ids)
    }

    /// 用**索引自己的 `normalized_*` 投影**补全轻量索引行的内容。
    ///
    /// ## 不变量（硬约束，改动前先读）
    ///
    /// 本函数**只读 SQLite 索引**：不打开日志、不 fold、不碰 `session_log_root()`。
    /// 「索引原样视图」必须保持**独立观察者**语义——否则 `rebuild_index` 的双读比对会退化成
    /// 「fold 与 fold 比」的恒零，`load_store` 的日志合并也失去可证伪性。
    ///
    /// ## 为什么需要它
    ///
    /// 停写 blob 之后，有日志会话的索引行只剩索引字段；但索引侧**本来就有一份完整投影**
    /// （`persist_commands_batch` 的 `apply_persist_command_tx` 写 `normalized_*`）。
    /// 因此「索引可读」这条承诺仍然成立：轻量行 + 规范化投影 = 完整会话。
    ///
    /// ## 何时调用（调用点决定语义，改动时同步核对）
    ///
    /// - `load_store` 的**维护/只读句柄**（`new_read_only`：诊断、`--dry-run`、测试夹具）；
    /// - `load_store` 的**降级分支**（日志根不可读 / 折叠超预算 / 单会话 fold 失败）——
    ///   「日志读不到」必须回落到一份**有内容**的索引，而不是回落到空壳行；
    /// - `rebuild_index` 的双读比对基准（`fold.rs`）。
    ///
    /// `covered` = 本轮**已由日志 fold 提供**的会话（内容以日志为准，不得被索引覆盖）；
    /// 投影里没有内容（无 `normalized_*` 行）的会话**保持轻量行**，不回退成空会话。
    pub(crate) fn fill_index_only_rows_from_index_projection(
        &self,
        sessions: &mut HashMap<String, SessionState>,
        covered: &HashSet<String>,
    ) {
        if sessions.is_empty() {
            return;
        }
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!(
                    "[pony-agent][session] index_projection_skipped: 连接不可用（{error}）；轻量索引行保持原样"
                );
                return;
            }
        };
        let Some(conn) = slot.as_ref() else {
            return;
        };
        let ids = match Self::index_only_row_ids(conn) {
            Ok(ids) => ids,
            Err(error) => {
                eprintln!(
                    "[pony-agent][session] index_projection_skipped: 轻量行扫描失败（{error}）"
                );
                return;
            }
        };
        let targets: Vec<String> = ids
            .into_iter()
            .filter(|session_id| sessions.contains_key(session_id) && !covered.contains(session_id))
            .collect();
        if targets.is_empty() {
            return;
        }
        let Some(projection) = self.load_store_normalized(conn) else {
            eprintln!(
                "[pony-agent][session] index_projection_skipped: 规范化投影不可读；{} 个轻量索引行保持原样",
                targets.len()
            );
            return;
        };
        let mut filled = 0usize;
        for session_id in targets {
            let Some(state) = projection.sessions.get(&session_id) else {
                continue;
            };
            // 投影侧是空会话（没有任何 normalized 行）⇒ 保留轻量行（它有标题/时间，
            // 比一条空会话更有用；空会话会让侧边栏多出一条「新对话」）。
            if state.history.is_empty()
                && state.history_nodes.is_empty()
                && state.turn_trace_history.is_empty()
            {
                continue;
            }
            sessions.insert(session_id, state.clone());
            filled += 1;
        }
        if filled > 0 {
            eprintln!(
                "[pony-agent][session] index_projection_filled: {filled} 个轻量索引行由 normalized_* 投影补全"
            );
        }
    }

    /// PA-174 P1：**投影侧全空**（`__index_only` 轻量行 + 零 `normalized_*` 行）的会话，
    /// 历史只能来自 durable `turn_events`——否则装载后 `history: []`，重启即「历史为零」
    /// （真实事故 `pa_01a1ba70`：`turn_events` 13098 行 / 48 turn 仍在，投影却零消息）。
    ///
    /// 口径（PA-174 契约冻结）：
    /// - 只处理**折叠历史为空**的会话；已有投影内容的会话一字不改；
    /// - `turn_events` 无消息事件 ⇒ 保持空（不造消息、不伪造成功）；
    /// - 事件读取复用后端 `load_turn_events`（seq 升序；坏 payload 降级为墓碑；IO 失败
    ///   contained 空），本函数只做「空壳 → 重建」这一步。
    ///
    /// PA-174 B1（审查整改）：重建按**活动血缘**过滤，不得合并全分支。`turn_events` 同时
    /// 容纳 main 与 forsaken fork 的事件；全量取会把被放弃的 fork 轮次复活，而
    /// `refresh_session_metadata` 会把结果写回索引（错误被持久化）。血缘判据复用
    /// `history_from_branch_events`（内部即 `projection::fold_all_with_branches`）。
    /// **拿不到血缘信息时宁可不重建**（保持空 + 结构化 warn），不猜测、不并集。
    pub(crate) fn rebuild_history_from_turn_events_when_projection_empty(
        &self,
        sessions: &mut HashMap<String, SessionState>,
    ) {
        if sessions.is_empty() {
            return;
        }
        let empty_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, state)| state.history.is_empty())
            .map(|(session_id, _)| session_id.clone())
            .collect();
        if empty_ids.is_empty() {
            return;
        }
        let mut rebuilt = 0usize;
        for session_id in empty_ids {
            let Some(state) = sessions.get(session_id.as_str()) else {
                continue;
            };
            // 血缘信息：cursor 的活动分支 + 分支表 + 节点→分支映射（均来自索引行/投影）。
            let active_branch = state.history_cursor.active_branch_id.clone();
            let has_lineage = active_branch.is_some() || !state.history_branches.is_empty();
            let initial_branch = active_branch.clone().unwrap_or_else(|| "main".to_string());
            let branches = state.history_branches.clone();
            let node_branch: HashMap<&str, &str> = state
                .history_nodes
                .iter()
                .map(|node| (node.node_id.as_str(), node.branch_id.as_str()))
                .collect();
            // 有元数据却解析不出活动分支 ⇒ 血缘不可判定（宁可不重建）。
            if let Some(active) = active_branch.as_deref() {
                if !branches.is_empty() && !branches.iter().any(|b| b.branch_id == active) {
                    eprintln!(
                        "[pony-agent][session] history_rebuild_skipped: session={session_id} reason=unknown_active_branch active_branch={active}"
                    );
                    continue;
                }
            }
            let raw_events = self.load_turn_events(session_id.as_str(), None);
            if raw_events.is_empty() {
                continue;
            }
            let history = if has_lineage {
                // 空 branch_id 是旧形态（无分支概念）⇒ 归入活动血缘。
                let events: Vec<(u64, String, crate::agent::turn_event::TurnEvent)> = raw_events
                    .into_iter()
                    .map(|(seq, branch_id, event)| {
                        let branch_id = if branch_id.is_empty() {
                            initial_branch.clone()
                        } else {
                            branch_id
                        };
                        (seq, branch_id, event)
                    })
                    .collect();
                crate::agent::session::history_from_branch_events(
                    &events,
                    &initial_branch,
                    &node_branch,
                    &branches,
                )
            } else {
                // 无血缘元数据：单分支（旧形态）照常重建；**多分支事件流无法判定**活动血缘
                // ⇒ 保持空 + warn（绝不合并全分支）。
                let distinct: std::collections::HashSet<&str> = raw_events
                    .iter()
                    .map(|(_seq, branch_id, _event)| branch_id.as_str())
                    .filter(|branch_id| !branch_id.is_empty())
                    .collect();
                if distinct.len() > 1 {
                    eprintln!(
                        "[pony-agent][session] history_rebuild_skipped: session={session_id} reason=no_branch_lineage branches={distinct:?}"
                    );
                    continue;
                }
                let events: Vec<crate::agent::turn_event::TurnEvent> = raw_events
                    .into_iter()
                    .map(|(_seq, _branch_id, event)| event)
                    .collect();
                crate::agent::session::history_from_turn_events(&events)
            };
            if history.is_empty() {
                continue;
            }
            if let Some(state) = sessions.get_mut(session_id.as_str()) {
                state.history = history;
                rebuilt += 1;
            }
        }
        if rebuilt > 0 {
            eprintln!(
                "[pony-agent][session] history_rebuilt_from_turn_events: {rebuilt} 个投影空会话由事件日志重建历史"
            );
        }
    }
}

impl SqliteSessionBackend {
    /// ADR 0099 阶段 3-D：`ponycli sessions rebuild-index --explain <id>` 的只读诊断入口。
    ///
    /// 返回**逐字段摘要**（`字段 | fold: … | index: …`；摘要 = 条数 + 首条前 80 字符，
    /// 绝不打印大字段全文）；无差异时返回恰好一行 `no difference`。
    /// 比较口径与 `rebuild-index` 的 `mismatched` **同源**（`fold::session_parity_differences`），
    /// 所以「解释」与「结论」不会分叉。
    pub fn explain_session_parity(&self, session_id: &str) -> Result<Vec<String>, String> {
        crate::agent::session::fold::explain_session_parity(self, session_id)
    }

    /// 跨真源比对的**豁免清单**（只读投影；唯一定义仍是
    /// `fold::LOG_UNCARRIED_STATE_FIELDS`，`migrate-logs` 侧是它的别名）。
    ///
    /// 暴露出来的目的：运维/测试可以断言「rebuild-index 与 migrate-logs 用的是同一份清单」，
    /// 而不是靠读代码相信。
    pub fn parity_ignored_state_fields() -> [&'static str; 2] {
        crate::agent::session::fold::LOG_UNCARRIED_STATE_FIELDS
    }
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
// ADR 0099 阶段 1-B：日志文件真源（写入先行 / 删除后置）
// ---------------------------------------------------------------------------

/// 真源优先：把本批命令追加进会话日志文件（含 `fsync`）。
///
/// 放在 commit 边界旁边而不是 `sqlite_session.rs` 里：后者已计入 file-size 棘轮。
///
/// - 成功 ⇒ 调用方才允许写 SQLite 索引；
/// - 失败 ⇒ 调用方整批判 `Failed`，**SQLite 一行都不改**（真源不允许「部分成功」）；
/// - 降级只读实例（阶段 0 E5：抢不到单实例锁）**不写日志**——它本来就不可能提交
///   SQLite，真源与索引必须一起不动，否则回放会把只读实例的意图当成事实。
pub(crate) fn append_session_log_to_file(
    backend: &super::SqliteSessionBackend,
    commands: &[crate::agent::session::PersistCommand],
) -> Result<(), String> {
    use crate::agent::session::SessionBackend;
    if backend.is_read_only() {
        return Ok(());
    }
    backend.session_log_sink.append_commands(commands)
}

/// `RemoveSession` 的日志目录清理：**只在 SQLite 提交成功之后**执行。
///
/// 顺序有意义：先删索引行、再删目录 ⇒ 最坏情况剩「日志还在但索引已删」；反过来
/// 则会留下「索引行还在、真源已丢」的复活源（比脏文件更危险）。
pub(crate) fn delete_session_logs_after_commit(
    backend: &super::SqliteSessionBackend,
    commands: &[crate::agent::session::PersistCommand],
    outcome: &crate::agent::session::PersistCommandOutcome,
) {
    if !matches!(
        outcome,
        crate::agent::session::PersistCommandOutcome::Succeeded
    ) {
        return;
    }
    for command in commands {
        if let crate::agent::session::PersistCommand::RemoveSession { session_id, .. } = command {
            delete_session_log_dir(backend, session_id);
        }
    }
}

/// 删除一个会话的日志目录；失败只记日志（不制造二次回滚）。
pub(crate) fn delete_session_log_dir(backend: &super::SqliteSessionBackend, session_id: &str) {
    if let Err(error) = backend.session_log_sink.delete_session(session_id) {
        eprintln!(
            "[pony-agent][session] session log 目录删除失败（session={session_id}）：{error}"
        );
    }
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
            self.upsert_session_row_bump(&tx, session_id, &prepared)
                .map_err(CommitFailure::other)?;
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
            self.upsert_session_row_bump(tx, session_id, &prepared)?;
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
                self.upsert_session_row_bump_with_full_blob(&tx, session_id, &session)?;
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

// ---------------------------------------------------------------------------
// ADR 0099 阶段 2-A：索引重建（日志 = 真源 → fold 结果覆盖索引）
// ---------------------------------------------------------------------------

impl SqliteSessionBackend {
    /// 会话日志根目录（`<db 目录>/session-logs/`）。
    pub(crate) fn session_log_root(&self) -> std::path::PathBuf {
        self.session_log_sink.root().to_path_buf()
    }

    /// 索引里现有的会话 id（`sessions` blob ∪ `normalized_sessions`）——「无日志会话」的计数源。
    pub(crate) fn session_index_ids(&self) -> Result<Vec<String>, String> {
        let slot = self.connection()?;
        let conn = slot
            .as_ref()
            .ok_or_else(|| "connection not initialized".to_string())?;
        let mut ids = std::collections::BTreeSet::new();
        for sql in [
            "SELECT conversation_id FROM sessions",
            "SELECT session_id FROM normalized_sessions",
        ] {
            let mut statement = conn.prepare(sql).map_err(|e| format!("prepare `{sql}`: {e}"))?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("query `{sql}`: {e}"))?;
            for row in rows {
                ids.insert(row.map_err(|e| format!("read `{sql}` row: {e}"))?);
            }
        }
        Ok(ids.into_iter().collect())
    }

    /// 用 fold 结果**覆盖**某会话的索引行（单事务）。
    pub(crate) fn rebuild_folded_session_index(&self, state: &SessionState) -> Result<(), String> {
        self.try_recover_instance_lock();
        let slot = self.connection()?;
        let conn = slot
            .as_ref()
            .ok_or_else(|| "connection not initialized".to_string())?;
        let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| format!("begin rebuild tx: {e}"))?;
        let session_id = state.conversation_id.as_str();
        // 覆盖语义：先清本会话的索引行（否则日志里已删掉的内容会残留在索引里）。
        for table in [
            "normalized_messages",
            "normalized_turns",
            "normalized_turn_traces",
            "normalized_trace_steps",
            "normalized_trace_timeline",
            "normalized_tool_activities",
            "normalized_history_nodes",
            "normalized_history_cursor",
            "normalized_history_branches",
            "session_turn_traces",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE session_id = ?1"),
                params![session_id],
            )
            .map_err(|e| format!("clear index table `{table}`: {e}"))?;
        }
        tx.execute(
            "DELETE FROM normalized_sessions WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| format!("clear normalized session: {e}"))?;
        // 重放 fold 出的规范命令：与 live 写路径**同一套**索引语义（不另写一套 SQL）。
        let epoch = rebuild_epoch_floor(&tx);
        for command in crate::agent::session::fold::canonical_commands(state, epoch) {
            self.apply_persist_command_tx(&tx, &command)?;
        }
        // 没有对应命令的会话级派生字段：同事务补写，避免两条读路径分叉。
        tx.execute(
            "UPDATE normalized_sessions SET provider_native_transcript_json = ?2,
                turn_trace_refs_json = ?3, memory_json = ?4, history_state_evidence_json = ?5,
                workspace_id = ?6
             WHERE session_id = ?1",
            params![
                session_id,
                derived_json(&state.provider_native_transcript)?,
                derived_json(&state.turn_trace_refs)?,
                derived_json(&serde_json::json!({
                    "longTermMemoryEntries": state.long_term_memory_entries,
                    "memoryWriteEvidence": state.memory_write_evidence,
                    "memoryWriteHookTraceRecords": state.memory_write_hook_trace_records,
                }))?,
                derived_json(&state.history_state_evidence)?,
                state.workspace_id,
            ],
        )
        .map_err(|e| format!("update normalized session derivations: {e}"))?;
        self.upsert_session_row_bump(&tx, session_id, state)?;
        tx.commit().map_err(|e| format!("commit rebuild tx: {e}"))
    }
}

/// 重建重放命令的 epoch 下界：不得低于库内迁移 barrier（否则 `apply_persist_command_tx` 拒收）。
fn rebuild_epoch_floor(tx: &rusqlite::Transaction<'_>) -> u64 {
    tx.query_row(
        "SELECT value FROM store_metadata WHERE key = 'storage.normalized.v1.epoch'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|raw| raw.parse::<u64>().ok())
    .unwrap_or(0)
    .max(1)
}

fn derived_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("serialize derived field: {e}"))
}

// ---------------------------------------------------------------------------
// ADR 0099 阶段 2-B（A）：`AppendHookRecords` 的**追加**口径
// ---------------------------------------------------------------------------

impl SqliteSessionBackend {
    /// 把一批 hook 记录**追加**进 legacy `session_turn_traces.trace_data` 的
    /// `hookTraceRecords` 数组。
    ///
    /// ## 为什么必须追加（不能再用 `sync_legacy_trace_update_tx` 的逐键替换）
    ///
    /// 同一条命令在三个地方落位，语义必须一致：
    /// - **日志命令流**（真源）：`append_hook_records` 逐条追加，fold 后累加；
    /// - **内存态**（`SessionStore`）：`trace.hook_trace_records.extend(...)`；
    /// - **索引**：`normalized_turn_traces.extension_json` 已是追加。
    ///
    /// 只有 legacy `trace_data` 走的是「按键替换」：两批命令之后 fold 得 2 条、索引得 1 条
    /// （只剩最后一批）。裁决以日志为准 ⇒ 索引侧改成追加，`session_parsed` 的
    /// 「hook 口径一致性」用例才可能真绿。
    ///
    /// 行缺失时与 [`Self::sync_legacy_trace_update_tx`] 同口径：从
    /// `normalized_turn_traces.raw_json` 修起；两边都没有 ⇒ no-op（不凭空造 trace 行，
    /// 与 fold「找不到 trace 行则跳过」一致）。
    pub(crate) fn append_legacy_hook_records_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        turn_id: &str,
        hook_trace_records: &[crate::agent::hooks::HookTraceRecord],
        updated_at: u64,
    ) -> Result<(), String> {
        let legacy: Option<(String, i64)> = tx
            .query_row(
                "SELECT trace_data, trace_order FROM session_turn_traces
                 WHERE session_id = ?1 AND turn_id = ?2",
                params![session_id, turn_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok();
        let (raw, trace_order) = match legacy {
            Some(row) => row,
            None => {
                let repaired: Option<(String, i64)> = tx
                    .query_row(
                        "SELECT raw_json, trace_order FROM normalized_turn_traces
                         WHERE session_id = ?1 AND turn_id = ?2",
                        params![session_id, turn_id],
                        |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .ok()
                    .and_then(|(raw, order)| raw.map(|raw| (raw, order)));
                // 两边都没有 trace 行 ⇒ 无可追加（fold 侧同样是 no-op）。
                let Some(row) = repaired else {
                    return Ok(());
                };
                row
            }
        };
        let mut trace: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse legacy trace: {e}"))?;
        let mut records = trace
            .get("hookTraceRecords")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        for record in hook_trace_records {
            records.push(serde_json::to_value(record).map_err(|e| format!("encode hook record: {e}"))?);
        }
        trace["hookTraceRecords"] = serde_json::Value::Array(records);
        trace["updatedAt"] = serde_json::json!(updated_at);
        tx.execute(
            "INSERT OR REPLACE INTO session_turn_traces
             (session_id, turn_id, updated_at_ms, trace_order, trace_data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                turn_id,
                updated_at as i64,
                trace_order,
                serde_json::to_string(&trace).map_err(|e| format!("serialize legacy trace: {e}"))?,
            ],
        )
        .map_err(|e| format!("append legacy hook records: {e}"))?;
        Ok(())
    }
}
