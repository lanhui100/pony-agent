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
use std::sync::Arc;

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
    pub(crate) fn upsert_session_row_bump_with_log_check(
        &self,
        conn: &Connection,
        session_id: &str,
        session: &SessionState,
        has_log: bool,
    ) -> Result<(), String> {
        if has_log {
            let payload = index_only_row_payload(session)?;
            return self.write_session_row(conn, session_id, session, &payload);
        }
        let payload =
            serde_json::to_string(session).map_err(|e| format!("serialize session: {e}"))?;
        self.write_session_row(conn, session_id, session, &payload)
    }

    pub(crate) fn upsert_session_row_bump(
        &self,
        conn: &Connection,
        session_id: &str,
        session: &SessionState,
    ) -> Result<(), String> {
        self.upsert_session_row_bump_with_log_check(
            conn,
            session_id,
            session,
            self.session_has_log(session_id),
        )
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
    /// 预算版（返回值 = 因预算耗尽而**未补投影**的会话数）。
    ///
    /// 启动期路径走 [`Self::run_budgeted_startup_self_heal`]（预算按阶段起算）；
    /// 维护/诊断路径直接传 `None`（不设上限）。
    pub(crate) fn attach_index_trace_projections_with_deadline(
        &self,
        sessions: &mut HashMap<String, SessionState>,
        targets: &HashSet<String>,
        deadline: Option<std::time::Instant>,
    ) -> usize {
        if targets.is_empty() {
            return 0;
        }
        if Self::self_heal_budget_exhausted(deadline) {
            eprintln!(
                "[pony-agent][session] startup_self_heal_skipped: component=trace_projection reason=budget_exhausted targets={} — 本轮跳过索引独有投影补全（下轮启动再补）",
                targets.len()
            );
            return targets.len();
        }
        let total = targets.len();
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!(
                    "[pony-agent][session] trace_projection_skipped: 连接不可用（{error}）"
                );
                return total;
            }
        };
        let Some(conn) = slot.as_ref() else {
            return total;
        };
        let mut skipped = 0usize;
        for session_id in targets {
            if Self::self_heal_budget_exhausted(deadline) {
                skipped += 1;
                continue;
            }
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
        if skipped > 0 {
            eprintln!(
                "[pony-agent][session] startup_self_heal_skipped: component=trace_projection reason=budget_exhausted skipped={skipped} — 剩余会话本轮跳过投影补全（下轮启动再补）"
            );
        }
        skipped
    }

    /// 启动期**预算内**的维护项：索引独有 trace 投影补全 + 空壳会话的事件重建。
    ///
    /// ## 为什么不含索引回填（B047，改这里之前先读）
    ///
    /// 索引投影回填（`fill_index_only_rows_from_index_projection`）**不受预算约束**：它是
    /// 「日志 fold 降级 ⇒ 仍给出完整索引视图」这条既有契约的落点
    /// （`session_read_from_log::fifty_logged_sessions_load_within_budget_and_degrade_keeps_the_index_complete`
    /// 守着它），跳过它会把 50+ 会话读成空壳——那不是提速，是读错。
    ///
    /// 但它同时是真实库上 1.5s 级的一次整库投影读。旧实现把预算 deadline 取在回填**之前**，
    /// 结果维护项**在入口即判过期**（实测每轮启动都是
    /// `history_rebuild reason=budget_exhausted sessions=28`：既没修任何会话，又白跑一遍）。
    /// 因此预算是**按阶段起算**的：`load_store` 在回填之后调
    /// `startup_repair::reset_startup_budget()`，再调本函数。
    ///
    /// ## 边界
    ///
    /// 维护/诊断路径（`new_read_only`、`rebuild_index` 双读比对）**不得**走这里：它们要的
    /// 是完整视图，用各自的单步入口并显式传 `None`（不设上限）。
    pub(crate) fn run_budgeted_startup_self_heal(
        &self,
        sessions: &mut HashMap<String, SessionState>,
        covered: &HashSet<String>,
        deadline: Option<std::time::Instant>,
    ) {
        self.attach_index_trace_projections_with_deadline(sessions, covered, deadline);
        self.rebuild_history_from_turn_events_when_projection_empty_with_deadline(
            sessions, deadline,
        );
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

    /// 启动期自愈总预算的唯一真源在 `session::startup_repair`（那里同时被 `SessionStore`
    /// 的 trace 缓存重建复用；各算一份预算会把「启动自愈上限」变成两倍）。
    ///
    /// 语义：自愈（索引投影补全 / observation 投影 / 事件重建）是**维护动作**——跳过它的代价
    /// 只是「本轮列表里个别会话内容为空」，下一轮启动会再补；而无预算的整库补全会把 TUI 启动
    /// 卡在「读完整库」上（生产库 1.2GB 时是真实事故面）。

    /// 解析自愈预算为**绝对截止时刻**（进程内首次调用冻结）；`None` = 不设上限。
    pub(crate) fn startup_self_heal_deadline() -> Option<std::time::Instant> {
        crate::agent::session::startup_repair::startup_budget_deadline()
    }

    /// 预算是否已耗尽：`None`（不设上限）永不计时；`0ms` 预算在第一次调用即到期。
    pub(crate) fn self_heal_budget_exhausted(deadline: Option<std::time::Instant>) -> bool {
        crate::agent::session::startup_repair::budget_exhausted(deadline)
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
        let Some(projection) = self.load_store_normalized(conn, None) else {
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
            // 投影只带内容：索引行的标题/时间/轮次/归属是真值（否则 `updated_at_ms` 归零，
            // 启动修复用 now 兜底 ⇒ 会话「凭空跳到最新」，列表顺序每次启动都漂移）。
            let mut merged = state.clone();
            if let Some(indexed_row) = sessions.get(&session_id) {
                inherit_index_metadata(indexed_row, &mut merged);
            }
            sessions.insert(session_id, merged);
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
        self.rebuild_history_from_turn_events_when_projection_empty_with_deadline(sessions, None);
    }

    /// 预算版（返回值 = 因预算耗尽而**未尝试重建**的会话数）。
    ///
    /// 这是自愈里最贵的一步（逐会话读事件流 + fold），必须**排在索引投影补全之后**：
    /// 投影补全超预算时，拿不到血缘/元数据的空壳不值得再花事件 IO 去试。
    pub(crate) fn rebuild_history_from_turn_events_when_projection_empty_with_deadline(
        &self,
        sessions: &mut HashMap<String, SessionState>,
        deadline: Option<std::time::Instant>,
    ) -> usize {
        if sessions.is_empty() {
            return 0;
        }
        let empty_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, state)| state.history.is_empty())
            .map(|(session_id, _)| session_id.clone())
            .collect();
        if empty_ids.is_empty() {
            return 0;
        }
        if Self::self_heal_budget_exhausted(deadline) {
            eprintln!(
                "[pony-agent][session] startup_self_heal_skipped: component=history_rebuild reason=budget_exhausted sessions={} — 本轮不做事件重建（下轮启动再补）",
                empty_ids.len()
            );
            return empty_ids.len();
        }
        let mut rebuilt = 0usize;
        let mut skipped = 0usize;
        for session_id in empty_ids {
            if Self::self_heal_budget_exhausted(deadline) {
                skipped += 1;
                continue;
            }
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
                // 事件必须带 seq：`history/squash` 的边界判据是 seq（压缩过的会话
                // 一旦丢掉 seq，重建就会把被压缩历史整段复活）。
                let events: Vec<(u64, crate::agent::turn_event::TurnEvent)> = raw_events
                    .into_iter()
                    .map(|(seq, _branch_id, event)| (seq, event))
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
        if skipped > 0 {
            eprintln!(
                "[pony-agent][session] startup_self_heal_skipped: component=history_rebuild reason=budget_exhausted rebuilt={rebuilt} skipped={skipped} — 剩余空壳本轮不重建（下轮启动再补）"
            );
        }
        skipped
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
// ADR 0107 §5 P1：**维护锁**（`<db>.lock`，跨进程独占）
// ---------------------------------------------------------------------------
//
// 语义与旧 E5 单实例锁的区别：
// - 旧语义：日常写路径也抢它，抢不到 ⇒ 整库降级只读（多窗口不可用）；
// - 新语义：**只有破坏性维护**（rebuild-index / migrate / VACUUM / 全库回填）才抢它，
//   抢不到 ⇒ 明确拒绝（fail-closed），绝不并发执行；日常写完全不受它影响，交给
//   SQLite 的 WAL + busy_timeout 做多进程串行化。

/// 维护锁抢不到时的错误（D2：必须显式失败）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceBusy {
    pub lock_path: PathBuf,
}

impl std::fmt::Display for MaintenanceBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "maintenance_lock_busy: {} 已被其他进程持有（rebuild/migrate 不得并发执行）",
            self.lock_path.display()
        )
    }
}

impl std::error::Error for MaintenanceBusy {}

/// 维护锁 guard：Drop 即释放（持有期间独占 `<db>.lock`）。
#[derive(Debug)]
pub struct MaintenanceGuard {
    _file: Arc<std::fs::File>,
}

/// 尝试取维护锁（非阻塞）。抢不到 ⇒ [`MaintenanceBusy`]；调用方必须放弃本次维护。
pub fn acquire_maintenance_lock(db_path: &Path) -> Result<MaintenanceGuard, MaintenanceBusy> {
    let lock_path = PathBuf::from(format!("{}.lock", db_path.display()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|_| MaintenanceBusy {
            lock_path: lock_path.clone(),
        })?;
    if try_lock_exclusive(&file) {
        return Ok(MaintenanceGuard { _file: Arc::new(file) });
    }
    eprintln!(
        "[pony-agent][session] maintenance_lock_denied: {} 已被其他进程持有；\
         本次维护操作拒绝执行（不并发 rebuild/migrate）",
        lock_path.display()
    );
    Err(MaintenanceBusy { lock_path })
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
            "[pony-agent][session] maintenance_lock_error：flock 异常，按抢不到处理并拒绝维护：{error}"
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
            "[pony-agent][session] maintenance_lock_error：LockFileEx 异常，按抢不到处理并拒绝维护：{error}"
        );
    }
    false
}

#[cfg(not(any(unix, windows)))]
fn try_lock_exclusive(_file: &std::fs::File) -> bool {
    true
}

// ---------------------------------------------------------------------------
// B046：库体回收（freelist 空洞）与 WAL 归零
// ---------------------------------------------------------------------------
//
// 背景（2026-11-10 实测，真实库）：`DELETE` 掉一行 3.89MB 的 `session_turn_traces`
// ⇒ `freelist_count` +1066 页（4.2MB），`page_count` 不变（**文件不缩**）；随后插入小行
// **复用 0 页**。库是 `auto_vacuum=OFF`，于是每删一次会话就漏一个洞、永不复用：
// 1.2GB 的库里 455MB（38%）是空洞。
//
// 反证先行（别拿「提速」当立项理由）：空洞**不拖慢**启动。同一份快照 A/B（1.2GB 原样
// vs `VACUUM INTO` 后 730MB），pty 内各 3 轮，首帧前累计读盘字节**完全一致**（3083MB），
// 耗时差落在噪声内——SQLite 全表扫描只走 B-tree 页、从不访问 freelist。本条目是**磁盘
// 占用**项，不是性能项。
//
// 因此这里只提供**显式**运维入口（`ponycli sessions vacuum`）：VACUUM 要独占 + 约 2 倍
// 临时空间，绝不挂在正常启动路径上（那等于把「打开应用」变成「可能失败的破坏性维护」）。
//
// 目标模式是 **`FULL`**，不是 `INCREMENTAL`：后者在真实形态下**收不动**（整表 DELETE 后
// 1956 页空闲，一次 `PRAGMA incremental_vacuum` 只减 1 页），而 FULL 在提交时自己截断文件、
// 200MB 库上只多 4ms。对照表见 [`reclaim_session_store_space`] 的文档。

/// WAL 超过这个体量才尝试 `TRUNCATE`（真实库曾积到 431MB 未回收）。
pub(crate) const WAL_TRUNCATE_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;

/// `-wal` 文件字节数（不存在按 0）。
pub(crate) fn wal_file_bytes(db_path: &Path) -> u64 {
    std::fs::metadata(PathBuf::from(format!("{}-wal", db_path.display())))
        .map(|meta| meta.len())
        .unwrap_or(0)
}

/// 写后 WAL 回收：小 WAL 走 `PASSIVE`，大 WAL 才尝试 `TRUNCATE`（失败**不**影响写入）。
///
/// `TRUNCATE` 需要「此刻没有其他读事务」，且会走 busy 处理器；写路径的 `busy_timeout` 是
/// 5000ms，若直接让它排队就等于「为主进程的回收动作卡 5 秒」。故这里把等待压到 50ms：
/// 抢不到立刻退回 `PASSIVE`（WAL 下轮再收），绝不把回收失败变成保存失败。
pub(crate) fn checkpoint_after_write(conn: &Connection, db_path: &Path) {
    checkpoint_after_write_with_threshold(conn, db_path, WAL_TRUNCATE_THRESHOLD_BYTES);
}

/// 阈值可注入的版本（测试用：不必造 64MB 的 WAL 才能驱动 TRUNCATE 分支）。
fn checkpoint_after_write_with_threshold(conn: &Connection, db_path: &Path, threshold: u64) {
    let wal_bytes = wal_file_bytes(db_path);
    if wal_bytes < threshold {
        if let Err(error) = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);") {
            // checkpoint 失败不影响数据正确性（只是 WAL 回收），保持既有「仅记录」语义。
            tracing::error!(target: "pony_core", error = %error, "[pony-agent][session] SQLite checkpoint error");
        }
        return;
    }
    let _ = conn.busy_timeout(std::time::Duration::from_millis(50));
    let attempt: Result<(i64, i64, i64), rusqlite::Error> = conn.query_row(
        "PRAGMA wal_checkpoint(TRUNCATE);",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );
    // 恢复连接初始化时钉的 5000ms（见 `SqliteSessionBackend::connection`）。
    let _ = conn.busy_timeout(std::time::Duration::from_millis(5000));
    match attempt {
        Ok((0, _log, _checkpointed)) => {
            eprintln!(
                "[pony-agent][session] wal_truncated: before={}MB — WAL 已归零",
                wal_bytes / (1024 * 1024)
            );
        }
        Ok((busy, log, checkpointed)) => {
            eprintln!(
                "[pony-agent][session] wal_truncate_busy: busy={busy} log={log} checkpointed={checkpointed} — 退回 PASSIVE（不阻塞写路径）"
            );
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        }
        Err(error) => {
            eprintln!(
                "[pony-agent][session] wal_truncate_error: {error} — 退回 PASSIVE（不阻塞写路径）"
            );
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        }
    }
}

/// 库体占用快照（`ponycli sessions vacuum --dry-run` 的只读探针）。
///
/// 只读连接 + `query_only`：**不建表、不迁移、不改 journal_mode**，`--dry-run` 对库文件零写入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreFootprint {
    pub page_size_bytes: u64,
    pub page_count: u64,
    pub freelist_pages: u64,
    /// `0` = OFF / `1` = FULL / `2` = INCREMENTAL。
    pub auto_vacuum: i64,
    pub db_bytes: u64,
    pub wal_bytes: u64,
}

impl StoreFootprint {
    /// 空洞占用的字节数（`freelist_count × page_size`）。
    pub fn freelist_bytes(&self) -> u64 {
        self.freelist_pages * self.page_size_bytes
    }

    /// 一行摘要（dry-run 与日志共用同一份措辞）。
    pub fn summary_line(&self) -> String {
        format!(
            "db={}MB page={}KB pages={} freelist={} 页({}MB) auto_vacuum={} wal={}MB",
            self.db_bytes / (1024 * 1024),
            self.page_size_bytes / 1024,
            self.page_count,
            self.freelist_pages,
            self.freelist_bytes() / (1024 * 1024),
            self.auto_vacuum,
            self.wal_bytes / (1024 * 1024)
        )
    }
}

/// 只读读取库体占用（`--dry-run` 用）。库不存在 ⇒ `Err`（与「空库」区分开）。
pub fn inspect_session_store(db_path: &Path) -> Result<StoreFootprint, String> {
    if !db_path.exists() {
        return Err(format!("库文件不存在：{}", db_path.display()));
    }
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("open db (read-only): {error}"))?;
    conn.execute_batch("PRAGMA query_only=ON;")
        .map_err(|error| format!("pragma query_only: {error}"))?;
    Ok(StoreFootprint {
        page_size_bytes: pragma_i64(&conn, "page_size")?.max(0) as u64,
        page_count: pragma_i64(&conn, "page_count")?.max(0) as u64,
        freelist_pages: pragma_i64(&conn, "freelist_count")?.max(0) as u64,
        auto_vacuum: pragma_i64(&conn, "auto_vacuum")?,
        db_bytes: std::fs::metadata(db_path).map(|meta| meta.len()).unwrap_or(0),
        wal_bytes: wal_file_bytes(db_path),
    })
}

/// `ponycli sessions vacuum` 的输出真值（字段即用户看到的那几行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VacuumReport {
    pub db_path: PathBuf,
    pub page_size_bytes: u64,
    /// `0` = OFF / `1` = FULL / `2` = INCREMENTAL。
    pub auto_vacuum_before: i64,
    pub auto_vacuum_after: i64,
    pub freelist_pages_before: u64,
    pub freelist_pages_after: u64,
    pub db_bytes_before: u64,
    pub db_bytes_after: u64,
    /// `true` = 本次把库从 OFF/INCREMENTAL 切到了 `FULL`（此后删除**自己**回收空间）。
    pub switched_to_full: bool,
    pub elapsed_ms: u128,
}

impl VacuumReport {
    /// 一行摘要（CLI 与日志共用同一份措辞）。
    pub fn summary_line(&self) -> String {
        format!(
            "db={}MB→{}MB freelist={}→{} 页 auto_vacuum={}→{} switched_to_full={} elapsed={}ms",
            self.db_bytes_before / (1024 * 1024),
            self.db_bytes_after / (1024 * 1024),
            self.freelist_pages_before,
            self.freelist_pages_after,
            self.auto_vacuum_before,
            self.auto_vacuum_after,
            self.switched_to_full,
            self.elapsed_ms
        )
    }
}

/// 一次性回收库体：把 `auto_vacuum` 切到 `FULL` 并回收既有空洞。
///
/// ## 语义
///
/// - **破坏性维护**：必须持有 `<db>.lock`（[`acquire_maintenance_lock`]），抢不到即拒绝；
/// - 目标模式是 **`auto_vacuum=FULL`**（不是 INCREMENTAL，实测见下）：`PRAGMA
///   auto_vacuum=FULL` + `VACUUM`（重写整库，需要约 2 倍临时空间）。**只 VACUUM 不改模式
///   会再涨回来**，所以两件事必须一起做；
/// - 收尾 `wal_checkpoint(TRUNCATE)`，把 WAL 归零（否则体积统计会被 `-wal` 掩住）。
///
/// ## 为什么是 FULL 而不是 INCREMENTAL（实测，别改回去）
///
/// 同一台机器、同一份 SQLite（3.45.1），200MB 库删一行 4MB trace：
///
/// | `auto_vacuum` | 删除耗时 | 删除后文件 | freelist |
/// |---|---|---|---|
/// | `0` OFF | 46ms | 200MB | 978 页 |
/// | `2` INCREMENTAL | 36ms | 201MB（**没有缩**） | 978 页（`incremental_vacuum` 只收 1 页） |
/// | `1` FULL | 50ms | **197MB** | **0** |
///
/// `INCREMENTAL` 需要额外手动跑 `PRAGMA incremental_vacuum`，而它在真实形态下**基本收不动**
/// （整表 DELETE 后 1956 页空闲，一次 `incremental_vacuum` 只减 1 页）；FULL 在提交时自己
/// 就截断了文件，代价是 200MB 库上 +4ms。所以「删会话不再留洞」这条承诺只能由 FULL 兑现，
/// 也就不需要在 `remove_session` 里追加任何回收调用。
///
/// ## 失败面
///
/// 任何一步失败都**不改数据**（SQLite 保证 VACUUM 原子）：返回 `Err` 并由调用方给非零退出码。
pub fn reclaim_session_store_space(db_path: &Path) -> Result<VacuumReport, String> {
    if !db_path.exists() {
        return Err(format!("库文件不存在：{}", db_path.display()));
    }
    let started = std::time::Instant::now();
    let _guard = acquire_maintenance_lock(db_path).map_err(|busy| busy.to_string())?;
    let conn = Connection::open(db_path).map_err(|error| format!("open db: {error}"))?;
    conn.execute_batch("PRAGMA busy_timeout=5000;")
        .map_err(|error| format!("pragma busy_timeout: {error}"))?;

    let page_size = pragma_i64(&conn, "page_size")?.max(0) as u64;
    let auto_vacuum_before = pragma_i64(&conn, "auto_vacuum")?;
    let freelist_pages_before = pragma_i64(&conn, "freelist_count")?.max(0) as u64;
    let db_bytes_before = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);

    let switched_to_full = auto_vacuum_before != 1;
    // 幂等：已是 FULL 也照跑 VACUUM（把既有空洞一次性收掉）。
    conn.execute_batch("PRAGMA auto_vacuum=FULL;")
        .map_err(|error| format!("pragma auto_vacuum: {error}"))?;
    // 模式切换只在 VACUUM 之后落到库文件头；同时回收既有空洞。
    conn.execute_batch("VACUUM;")
        .map_err(|error| format!("vacuum: {error}"))?;
    // WAL 不归零的话，体积统计会被 `-wal` 掩住（真实库曾积 431MB）。
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");

    let auto_vacuum_after = pragma_i64(&conn, "auto_vacuum")?;
    let freelist_pages_after = pragma_i64(&conn, "freelist_count")?.max(0) as u64;
    let db_bytes_after = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    let report = VacuumReport {
        db_path: db_path.to_path_buf(),
        page_size_bytes: page_size,
        auto_vacuum_before,
        auto_vacuum_after,
        freelist_pages_before,
        freelist_pages_after,
        db_bytes_before,
        db_bytes_after,
        switched_to_full,
        elapsed_ms: started.elapsed().as_millis(),
    };
    eprintln!(
        "[pony-agent][session] session_store_reclaimed: {}",
        report.summary_line()
    );
    Ok(report)
}

/// 读一个返回单值的整数 PRAGMA（`PRAGMA <name>` 的第二列）。
fn pragma_i64(conn: &Connection, name: &str) -> Result<i64, String> {
    conn.query_row(&format!("PRAGMA {name};"), [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("read pragma {name}: {error}"))
}

// ---------------------------------------------------------------------------
// ADR 0099 阶段 1-B：日志文件真源（写入先行 / 删除后置）
// ---------------------------------------------------------------------------

/// 真源优先：把本批命令追加进会话日志文件（含 `fsync`）。
///
/// 返回 `(written, restored, post_append_len)`：`written` = **实际写入了字节**（R5）；
/// `restored` = 每会话追加前的已提交长度；`post_append_len` = 每会话追加后长度
/// （P0.5：回滚前用它校验偏移未过期，绝不在「文件已被他人追加」时截断）。
///
/// 放在 commit 边界旁边而不是 `sqlite_session.rs` 里：后者已计入 file-size 棘轮。
///
/// - 成功 ⇒ 调用方才允许写 SQLite 索引；
/// - 失败 ⇒ 调用方整批判 `Failed`，**SQLite 一行都不改**（真源不允许「部分成功」）；
/// - 零写入批（`RemoveSession` / 无会话归属的 `PublishMetadata`）⇒ `Ok(false)`：日志层没
///   写任何字节，调用方**不得**把随之而来的索引失败判成 `LogPersisted`（R5）；
/// - **维护句柄**（`new_read_only`：`--dry-run` / 诊断 / 测试夹具，E8 零写入）⇒ `Ok(false)`
///   且连日志都不写（R4）；
/// - ADR 0107 §5 P1：**不存在**「锁降级实例」这一状态（日常实例恒可写），故这里只保留
///   长命 writer 缓存（[`SessionLogSink::append_commands`]）；「按批释放 flock」的
///   `append_commands_releasing` 退化为仅测试用的原语。
pub(crate) fn append_session_log_to_file(
    backend: &super::SqliteSessionBackend,
    commands: &[crate::agent::session::PersistCommand],
) -> Result<(bool, Vec<(String, u64)>, Vec<(String, u64)>), String> {
    // R4 / E8：维护句柄零写入（连日志也不写——否则 `--dry-run` 会产出 session.jsonl）。
    if backend.storage_read_only {
        return Ok((false, Vec::new(), Vec::new()));
    }
    let receipt = backend.session_log_sink.append_commands(commands)?;
    Ok((receipt.written, receipt.restored, receipt.post_append_len))
}

/// ADR 0107 P0.3：事务拒 stale-epoch 时**回滚本批已写日志**（预检与事务之间的竞态窗口兜底）。
///
/// 只回滚 `StaleEpoch`：`Failed` 时日志本来就没写成；`LogPersisted`（索引滞后）必须保留日志。
pub(crate) fn rollback_stale_batch_log(
    backend: &super::SqliteSessionBackend,
    written: bool,
    restored: &[(String, u64)],
    post_append_len: &[(String, u64)],
    outcome: &crate::agent::session::PersistCommandOutcome,
) {
    if !written || !matches!(outcome, crate::agent::session::PersistCommandOutcome::StaleEpoch) {
        return;
    }
    eprintln!(
        "[pony-agent][session] stale_epoch_log_rollback: 事务拒收 stale epoch ⇒ 回滚本批日志（{} 个会话）",
        restored.len()
    );
    backend.session_log_sink.rollback(restored, post_append_len);
}

/// 写日志前 epoch 预检的**拒绝原因**（ADR 0107 P0.3：fail-closed）。
pub(crate) enum EpochRejection {
    /// 明确过期：`(本批 epoch, 库内 barrier)`。
    Stale { epoch: u64, current: u64 },
    /// 预检自身不可用（连接 / 查询 / parse 失败）⇒ **不写日志**、整批 `Failed`（fail-loud）。
    Unavailable(String),
}

/// ADR 0107 P0.3（BLOCKING）：**写日志之前**的 epoch 预检——stale-epoch 命令不得进真源日志。
///
/// 为什么需要：日志先行是「先写日志、后校验索引」。stale-epoch 命令若先落进 JSONL，而
/// `fold_session` 不做 epoch 过滤，重启后会把**已被拒绝的消息**当作事实复活（strace 实证：
/// 先写 seq=2，后报 `stale epoch: 1 < 2`）。
///
/// 语义（fail-closed）：`Ok(())` = 预检通过；`Err(Stale)` = 明确过期；`Err(Unavailable)` =
/// 预检读不到 epoch（连接 / 查询 / 解析失败）⇒ 调用方**不得写日志**、整批判 `Failed`——
/// 宁可不写，也不把一批未校验的命令写进唯一真源。库内**没有** barrier 行 = barrier 0
/// （新库 / 未迁移），不是错误。
///
/// 与事务内校验的关系：两者是**双保险**——预检挡掉绝大多数过期批次；事务内校验在竞态窗口
/// （另一进程在预检与写入之间推进 barrier）里仍可能拒收，此时批次日志由
/// [`rollback_stale_batch_log`] 回滚，真源不留被拒命令。
pub(crate) fn precheck_stale_epoch(
    backend: &super::SqliteSessionBackend,
    commands: &[crate::agent::session::PersistCommand],
) -> Result<(), EpochRejection> {
    use rusqlite::OptionalExtension;
    /// 与写路径同口径的本地锁等待预算（慢盘 / 备份进程占锁时不要误判为「预检不可用」）。
    const PRECHECK_BUSY_TIMEOUT_MS: u64 = 5000;
    let current: u64 = {
        let slot = backend
            .connection()
            .map_err(|error| EpochRejection::Unavailable(format!("open connection: {error}")))?;
        let conn = slot
            .as_ref()
            .ok_or_else(|| EpochRejection::Unavailable("connection not initialized".to_string()))?;
        let _ = conn.busy_timeout(std::time::Duration::from_millis(PRECHECK_BUSY_TIMEOUT_MS));
        let raw = conn
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'storage.normalized.v1.epoch'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| EpochRejection::Unavailable(format!("read epoch: {error}")))?;
        match raw {
            // 无 barrier 行 = barrier 0（新库 / 未迁移）。
            None => 0,
            Some(raw) => raw.trim().parse::<u64>().map_err(|error| {
                EpochRejection::Unavailable(format!("parse epoch `{raw}`: {error}"))
            })?,
        }
    };
    match commands
        .iter()
        .map(|command| command.epoch())
        .find(|epoch| *epoch < current)
    {
        Some(epoch) => Err(EpochRejection::Stale { epoch, current }),
        None => Ok(()),
    }
}

/// 预检拒绝的收口：打结构化日志并给出批次 outcome。
pub(crate) fn refuse_epoch_precheck(
    rejection: EpochRejection,
) -> crate::agent::session::PersistCommandOutcome {
    use crate::agent::session::PersistCommandOutcome;
    match rejection {
        EpochRejection::Stale { epoch, current } => {
            eprintln!(
                "[pony-agent][session] SQLite batch command stale epoch（预检拒绝，未写日志）：{epoch} < {current}"
            );
            PersistCommandOutcome::StaleEpoch
        }
        EpochRejection::Unavailable(error) => {
            eprintln!(
                "[pony-agent][session] epoch_precheck_unavailable（fail-closed：不写日志、整批 Failed）：{error}"
            );
            PersistCommandOutcome::Failed
        }
    }
}

/// 日志（真源）写入失败：统一打印并整批判 `Failed`（SQLite 一行都不改）。
pub(crate) fn log_write_failure(error: String) -> crate::agent::session::PersistCommandOutcome {
    use crate::agent::session::PersistCommandOutcome;
    // ADR 0107 §5 P1 / D4：会话日志 flock 冲突 ≠ 数据损坏 ≠ 索引故障。
    // 这是「同一会话正被另一个进程写」的正常拒绝，必须可被上层识别并提示用户。
    if is_session_writer_busy(&error) {
        eprintln!(
            "[pony-agent][session] session_writer_busy: 同一会话正被另一个进程写，本次写入拒绝（不影响其它会话）：{error}"
        );
        return PersistCommandOutcome::WriterBusy;
    }
    eprintln!("[pony-agent][session] 会话日志（真源）写入失败，SQLite 索引未改动：{error}");
    PersistCommandOutcome::Failed
}

/// 会话日志写者冲突判定：`LogStoreError::Locked` 的文本由 `Display` 产生，含
/// `locked by another writer` / `Locked` 两种历史形态，两者都算「会话被占用」。
pub(crate) fn is_session_writer_busy(error: &str) -> bool {
    // 只认 `LogStoreError::Locked` 的精确 Display 文本（`log_store.rs`）：
    // 过宽的 "locked" 会把 SQLite 的 `database is locked` 也吞进来，那是 BUSY 重试域，
    // 不是会话写者冲突。
    error.contains("session log is locked by another writer")
}

/// ADR 0107 §5 P0（失败域隔离）：把**日志已落盘之后**的索引失败归一为 `LogPersisted`。
///
/// `Succeeded` / `Unsupported` / `StaleEpoch` 原样透传（语义不变，R1：命令被拒 ≠ 索引故障）；
/// 只有 `Failed` **且日志确实写入了字节**（`log_written`，R5）时才等价于「索引滞后」。
pub(crate) fn index_failure_after_log(
    outcome: crate::agent::session::PersistCommandOutcome,
    log_written: bool,
) -> crate::agent::session::PersistCommandOutcome {
    use crate::agent::session::PersistCommandOutcome;
    match outcome {
        PersistCommandOutcome::Failed if log_written => PersistCommandOutcome::LogPersisted,
        other => other,
    }
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

/// 应用单条命令失败的**语义分类**（ADR 0107 P0.1 / R1）。
///
/// 必须区分：`StaleEpoch`（materialize barrier 拒收旧 epoch——**不是**索引故障，
/// 不可当「索引滞后」）与 `Other`（约束/注入/IO 等真实应用错误，日志已落盘时即
/// `LogPersisted`）。把两者混成 `String` 会让 stale-epoch 被吞成 `LogPersisted`
/// ⇒ 调用方误认为「日志已落盘、可前进内存」。
pub(crate) enum ApplyFailure {
    /// 旧 epoch 命令被拒绝（materialize barrier 之后）。
    StaleEpoch(String),
    /// 其它应用错误（约束失败 / 注入失败 / IO）。
    Other(String),
}

impl ApplyFailure {
    /// 旧 epoch 拒收（materialize barrier 语义的唯一构造点）。
    pub(crate) fn stale_epoch(epoch: u64, current: i64) -> Self {
        Self::StaleEpoch(format!("stale epoch: {epoch} < {current}"))
    }

    /// 失败原文（诊断用，不改判定语义）。
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::StaleEpoch(message) | Self::Other(message) => message,
        }
    }
}

/// 既有 `map_err(|e| format!(…))` / `?` 站点统一落 `Other`（应用错误）。
impl From<String> for ApplyFailure {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

/// 重建重放等以 `String` 为错误类型的既有调用方保持可用（失败原文透传）。
impl From<ApplyFailure> for String {
    fn from(failure: ApplyFailure) -> Self {
        failure.message().to_string()
    }
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
            tracing::error!(target: "pony_core", error = %error, "[pony-agent][session] SQLite batch command begin tx error");
            return BatchAttempt::Done(PersistCommandOutcome::Failed);
        }
    };
    for command in commands {
        if let Err(failure) = backend.apply_persist_command_tx(&tx, command) {
            eprintln!(
                "[pony-agent][session] SQLite batch command error: {}",
                failure.message()
            );
            // R1：stale-epoch 必须类型化透传（调用方据此判 Err 且不入内存）。
            return BatchAttempt::Done(match failure {
                ApplyFailure::StaleEpoch(_) => PersistCommandOutcome::StaleEpoch,
                ApplyFailure::Other(_) => PersistCommandOutcome::Failed,
            });
        }
    }
    match tx.commit() {
        Ok(()) => BatchAttempt::Done(PersistCommandOutcome::Succeeded),
        Err(error) => {
            if is_busy_error(&error) {
                return BatchAttempt::Busy(format!("commit: {error}"));
            }
            tracing::error!(target: "pony_core", error = %error, "[pony-agent][session] SQLite batch command commit error");
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
            authoritative_traces: HashMap::new(),
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
        let existing_session_ids = load_existing_session_ids(&tx).map_err(CommitFailure::other)?;
        let existing_log_encoded_ids = self.session_log_sink.existing_log_encoded_ids();
        for (session_id, session) in &store.sessions {
            if store.versions.contains_key(session_id)
                && !existing_session_ids.contains(session_id)
            {
                tombstoned.insert(session_id.clone());
                eprintln!(
                    "[pony-agent][session] commit_boundary_tombstone_skip: session={session_id}（装载时存在、库内行已删除）"
                );
                continue;
            }
            let prepared = prepare_session_for_row(session_id, session);
            let has_log = crate::agent::session::encode_session_id_segment(session_id)
                .map(|enc| existing_log_encoded_ids.contains(&enc))
                .unwrap_or(false);
            self.upsert_session_row_bump_with_log_check(&tx, session_id, &prepared, has_log)
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
        let existing_log_encoded_ids = self.session_log_sink.existing_log_encoded_ids();
        for (session_id, session) in &store.sessions {
            if skip.contains(session_id) {
                continue;
            }
            let prepared = prepare_session_for_row(session_id, session);
            let has_log = crate::agent::session::encode_session_id_segment(session_id)
                .map(|enc| existing_log_encoded_ids.contains(&enc))
                .unwrap_or(false);
            self.upsert_session_row_bump_with_log_check(tx, session_id, &prepared, has_log)?;
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
            // 始终写 trace 表，保持 blob 与 trace 表同步（PA-088：authoritative 会话统一
            // 在本事务内从 store.authoritative_traces 写入，杜绝 171 次独立小事务与 fsync）。
            if session.trace_migration_state == TraceMigrationState::TraceTableAuthoritative {
                if let Some(traces) = store.authoritative_traces.get(session_id) {
                    self.replace_session_traces_tx(tx, session_id, traces)?;
                }
            } else {
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
#[allow(dead_code)]
fn session_row_exists(conn: &Connection, session_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE conversation_id = ?1)",
        params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(|error| format!("check session row: {error}"))
}

/// 批量加载 `sessions` 现存会话 ID 集合（单次 SQL 查询替代 O(N) 次 `session_row_exists` 往返）。
fn load_existing_session_ids(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT conversation_id FROM sessions")
        .map_err(|error| format!("prepare query existing session ids: {error}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("query existing session ids: {error}"))?;
    let mut ids = HashSet::new();
    for row in rows {
        ids.insert(row.map_err(|error| format!("read session id row: {error}"))?);
    }
    Ok(ids)
}

/// 已耗时（毫秒），用于把 busy 重试收敛到 NFR 总预算。
fn elapsed_ms(started: &std::time::Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// legacy trace 表里**出现过**的会话 id（只走 `session_id` 覆盖索引，不碰 `trace_data` 大字段）。
///
/// 供 [`SqliteSessionBackend::read_traces_normalized`] 的兜底步骤使用：**先**读规范化侧，
/// **再**只对规范化侧给不出 trace 的会话回读 legacy 行。
///
/// 早先的批量版本（`read_all_session_traces`）在同一次查询里扫完整个 `session_turn_traces`
/// （真实库 397 行 / 222MB / 645ms），调用方却按 `entry().or_insert(..)` 逐会话丢弃——
/// 绝大多数字节读完就扔。真实库实测「有 legacy 行却无规范化 trace 的会话」= 0（B047）。
pub(crate) fn legacy_trace_session_ids(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT session_id FROM session_turn_traces ORDER BY session_id")
        .map_err(|e| format!("prepare legacy trace session scan: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query legacy trace sessions: {e}"))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row.map_err(|e| format!("read legacy trace session id: {e}"))?);
    }
    Ok(ids)
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
        filter: Option<&str>,
    ) -> Result<HashMap<String, Vec<TurnTraceRecord>>, String> {
        let mut traces = self.read_all_normalized_traces(conn, filter)?;
        // E7 兜底口子收窄（B047）：只在**规范化侧给不出该会话语义**时回读 legacy 行。
        // B148：filter=Some 时只回读该会话，避免整表 legacy 扫描；语义与全量装载逐会话等价。
        let legacy_ids = match filter {
            Some(session_id) => vec![session_id.to_string()],
            None => legacy_trace_session_ids(conn)?,
        };
        for session_id in legacy_ids {
            if traces.contains_key(&session_id) {
                continue;
            }
            match self.read_session_traces(conn, &session_id) {
                Ok(rows) if !rows.is_empty() => {
                    traces.insert(session_id, rows);
                }
                Ok(_) => {}
                Err(error) => eprintln!(
                    "[pony-agent][session] legacy trace fallback failed for session {session_id}: {error}"
                ),
            }
        }
        Ok(traces)
    }

    fn read_all_normalized_traces(
        &self,
        conn: &Connection,
        filter: Option<&str>,
    ) -> Result<HashMap<String, Vec<TurnTraceRecord>>, String> {
        let filter_suffix = if filter.is_some() { " WHERE session_id = ?1" } else { "" };
        let mut statement = conn
            .prepare(&format!(
                "SELECT session_id, raw_json FROM normalized_turn_traces{filter_suffix}
                 ORDER BY session_id, trace_order ASC, updated_at_ms ASC, turn_id ASC",
            ))
            .map_err(|e| format!("prepare normalized trace load: {e}"))?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(filter.into_iter()), |row| {
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
        // ADR 0107 §5 P1：全库回填属于破坏性维护，非 dry-run 必须独占 `<db>.lock`。
        let _maintenance_guard = if dry_run {
            None
        } else {
            Some(
                acquire_maintenance_lock(self.db_path()).map_err(|busy| busy.to_string())?,
            )
        };
        let slot = self.connection()?;
        let conn = slot
            .as_ref()
            .ok_or_else(|| "connection not initialized".to_string())?;
        let sessions = self
            .load_store_normalized(conn, None)
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

#[cfg(test)]
mod startup_self_heal_budget_tests {
    //! P1 启动提速：自愈预算的**行为**断言（不是实现细节）。
    //!
    //! 口径：预算耗尽只降级「本轮视图」，一行都不写回；持久化语义不受影响。

    use super::*;
    use crate::agent::session::SeparateTraceTableMode;

    fn backend(tag: &str) -> SqliteSessionBackend {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let db_path = std::env::temp_dir().join(format!(
            "pony-heal-budget-{tag}-{}-{unique}.db",
            std::process::id()
        ));
        SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::DualWrite)
    }

    fn empty_history_session(id: &str) -> SessionState {
        serde_json::from_value(serde_json::json!({
            "conversationId": id,
            "title": "标题",
            "summary": "",
            "history": [],
            "turnCount": 3,
            "lastReferencedFile": null,
            "updatedAtMs": 10,
        }))
        .expect("minimal session state")
    }

    fn expired() -> Option<std::time::Instant> {
        // 用一个必然已经到期的 deadline 驱动「预算耗尽」路径。
        Some(std::time::Instant::now())
    }

    // 注：**没有**「预算耗尽跳过索引回填」的用例——索引回填是「fold 降级仍给出完整视图」
    // 这条契约的落点，不受预算约束。它的守卫是集成测试
    // `session_read_from_log::fifty_logged_sessions_load_within_budget_and_degrade_keeps_the_index_complete`。

    #[test]
    fn zero_budget_skips_history_rebuild_without_writing() {
        let backend = backend("rebuild");
        let mut sessions: HashMap<String, SessionState> = HashMap::new();
        sessions.insert("s1".to_string(), empty_history_session("s1"));

        let skipped = backend
            .rebuild_history_from_turn_events_when_projection_empty_with_deadline(
                &mut sessions,
                expired(),
            );

        assert_eq!(skipped, 1, "预算耗尽必须报告 1 个未重建会话");
        assert!(
            sessions["s1"].history.is_empty(),
            "预算耗尽不得凭空造出消息"
        );
    }

    #[test]
    fn zero_budget_orchestrator_leaves_every_session_untouched() {
        let backend = backend("orchestrator");
        let mut sessions: HashMap<String, SessionState> = HashMap::new();
        sessions.insert("s1".to_string(), empty_history_session("s1"));
        sessions.insert("s2".to_string(), empty_history_session("s2"));
        let covered = HashSet::new();

        backend.run_budgeted_startup_self_heal(&mut sessions, &covered, expired());

        assert_eq!(sessions.len(), 2);
        for id in ["s1", "s2"] {
            assert!(
                sessions[id].history.is_empty(),
                "自愈编排超预算时不得改动 {id} 的内容"
            );
        }
    }

    #[test]
    fn unlimited_deadline_never_reports_budget_exhaustion() {
        assert!(!SqliteSessionBackend::self_heal_budget_exhausted(None));
        let past = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_millis(1))
            .expect("instant minus 1ms");
        assert!(
            SqliteSessionBackend::self_heal_budget_exhausted(Some(past)),
            "已过去的截止时刻必须判定为耗尽"
        );
    }

    #[test]
    fn default_budget_is_bounded_and_env_override_stays_parseable() {
        assert_eq!(
            crate::agent::session::startup_repair::DEFAULT_STARTUP_SELF_HEAL_BUDGET_MS,
            1200
        );
        // ADR 0107 修复后语义：预算**每次装载**重起算（`reset_startup_budget` 在
        // `load_store` 入口调用），同一次装载内的多个自愈入口仍共用一条 deadline。
        // 因此这里不再断言「进程内跨调用恒等」——那正是导致长驻进程后续装载永久跳过
        // 历史重建的旧缺陷；改为断言「reset 后重新起算、且不回退到更早的 deadline」。
        let first = SqliteSessionBackend::startup_self_heal_deadline();
        crate::agent::session::startup_repair::reset_startup_budget();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let second = SqliteSessionBackend::startup_self_heal_deadline();
        assert!(
            second >= first,
            "reset 后必须按当前时刻重新起算（旧 deadline={first:?}，新 deadline={second:?}）"
        );
        // 0ms ⇒ 调用瞬间即到期（应急/测试用的「全跳过」口径）。
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_millis(0))
            .expect("deadline");
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(SqliteSessionBackend::self_heal_budget_exhausted(Some(deadline)));
    }
}

#[cfg(test)]
mod store_reclaim_tests {
    //! B046：库体回收的**行为**断言（不是实现细节）。
    //!
    //! 反证也钉在这里：本条目是磁盘占用项，不是性能项（空洞不影响扫表字节数）。
    use super::*;

    fn temp_db(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "pony-vacuum-{tag}-{}-{unique}.db",
            std::process::id()
        ))
    }

    fn create_table(db_path: &Path) {
        let conn = Connection::open(db_path).expect("open db");
        conn.execute_batch("PRAGMA page_size=4096; CREATE TABLE filler (x TEXT);")
            .expect("ddl");
    }

    /// 造洞：插入 N 行 ~4MB 文本再整表删除 ⇒ 页进 freelist、`page_count` 不变。
    fn seed_freelist(db_path: &Path, rows: usize) {
        let conn = Connection::open(db_path).expect("open db");
        for _ in 0..rows {
            conn.execute("INSERT INTO filler (x) VALUES (hex(randomblob(2000000)))", [])
                .expect("insert filler");
        }
        conn.execute("DELETE FROM filler", []).expect("delete filler");
    }

    #[test]
    fn vacuum_switches_to_full_and_reclaims_holes() {
        let db_path = temp_db("migrate");
        create_table(&db_path);
        seed_freelist(&db_path, 2);

        let before = inspect_session_store(&db_path).expect("inspect before");
        assert_eq!(before.auto_vacuum, 0, "前置：新库默认 auto_vacuum=OFF");
        assert!(
            before.freelist_pages > 0,
            "前置：整表 DELETE 必须留下空闲页（实际 {}）",
            before.freelist_pages
        );

        let report = reclaim_session_store_space(&db_path).expect("reclaim");

        assert_eq!(
            report.auto_vacuum_after, 1,
            "必须切到 auto_vacuum=FULL（INCREMENTAL 实测收不动，见函数文档的对照表）"
        );
        assert!(report.switched_to_full, "首次迁移必须报告模式切换");
        assert_eq!(report.freelist_pages_after, 0, "既有空洞必须清零");
        assert!(
            report.db_bytes_after < report.db_bytes_before,
            "文件必须真的变小（{} → {}）",
            report.db_bytes_before,
            report.db_bytes_after
        );
        let after = inspect_session_store(&db_path).expect("inspect after");
        assert_eq!(after.auto_vacuum, 1, "模式切换必须落到库文件头（持久）");
        assert_eq!(after.freelist_pages, 0);
        let _ = std::fs::remove_file(&db_path);
    }

    /// B046 的核心承诺：迁移之后**不再需要任何手动回收**——删除自己就把文件截回去。
    #[test]
    fn full_mode_makes_deletes_reclaim_space_without_manual_vacuum() {
        let db_path = temp_db("self-reclaim");
        create_table(&db_path);
        reclaim_session_store_space(&db_path).expect("migrate to FULL");

        // 造一块大内容再删掉：FULL 模式应在提交时自己把尾部页截掉。
        {
            let conn = Connection::open(&db_path).expect("open db");
            for _ in 0..2 {
                conn.execute("INSERT INTO filler (x) VALUES (hex(randomblob(2000000)))", [])
                    .expect("insert filler");
            }
        }
        let grown = inspect_session_store(&db_path).expect("inspect grown");
        {
            let conn = Connection::open(&db_path).expect("open db");
            conn.execute("DELETE FROM filler", []).expect("delete filler");
        }
        let after = inspect_session_store(&db_path).expect("inspect after delete");

        assert!(
            grown.db_bytes > after.db_bytes + 4 * 1024 * 1024,
            "FULL 模式删除必须自动缩文件（{} → {}）",
            grown.db_bytes,
            after.db_bytes
        );
        assert_eq!(after.freelist_pages, 0, "删除后不得留下空闲页（无人工回收）");
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn reclaim_is_idempotent_on_an_already_full_store() {
        let db_path = temp_db("idempotent");
        create_table(&db_path);
        reclaim_session_store_space(&db_path).expect("first reclaim");

        let report = reclaim_session_store_space(&db_path).expect("second reclaim");

        assert!(!report.switched_to_full, "已是 FULL ⇒ 不再报告模式切换");
        assert_eq!(report.auto_vacuum_after, 1, "模式必须仍是 FULL");
        assert_eq!(report.freelist_pages_after, 0, "二次执行不得留下空闲页");
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn wal_is_truncated_once_it_crosses_the_threshold() {
        let db_path = temp_db("wal");
        create_table(&db_path);
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .expect("wal pragmas");
        for _ in 0..2 {
            conn.execute("INSERT INTO filler (x) VALUES (hex(randomblob(1000000)))", [])
                .expect("insert filler");
        }
        let before = wal_file_bytes(&db_path);
        assert!(before > 1024 * 1024, "前置：WAL 必须真的长大（实际 {before} 字节）");

        // 阈值 0 ⇒ 一定走 TRUNCATE 分支（真实阈值是 64MB，不为测试造 64MB 的 WAL）。
        checkpoint_after_write_with_threshold(&conn, &db_path, 0);

        assert_eq!(
            wal_file_bytes(&db_path),
            0,
            "过阈值的 WAL 必须被 TRUNCATE 归零（真实库曾积 431MB 未回收）"
        );
        drop(conn);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(PathBuf::from(format!("{}-wal", db_path.display())));
    }

    #[test]
    fn wal_truncate_falls_back_to_passive_when_a_reader_pins_the_wal() {
        let db_path = temp_db("wal-busy");
        create_table(&db_path);
        let writer = Connection::open(&db_path).expect("open writer");
        writer
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .expect("wal pragmas");
        for _ in 0..2 {
            writer
                .execute("INSERT INTO filler (x) VALUES (hex(randomblob(1000000)))", [])
                .expect("insert filler");
        }
        // 第二个连接开一个读事务：TRUNCATE 需要「没有其他读事务」，此刻必然 busy。
        let reader = Connection::open(&db_path).expect("open reader");
        reader.execute_batch("BEGIN;").expect("begin read tx");
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM filler", [], |row| row.get(0))
            .expect("read");
        let before = wal_file_bytes(&db_path);
        assert!(before > 0, "前置：WAL 非空");

        // 不 panic、不报错、不阻塞写路径：抢不到就退回 PASSIVE。
        checkpoint_after_write_with_threshold(&writer, &db_path, 0);

        let _ = reader.execute_batch("ROLLBACK;");
        drop(reader);
        drop(writer);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(PathBuf::from(format!("{}-wal", db_path.display())));
    }

    #[test]
    fn reclaim_is_fail_closed_when_the_maintenance_lock_is_held() {
        let db_path = temp_db("lock");
        create_table(&db_path);
        let _guard = acquire_maintenance_lock(&db_path).expect("取到维护锁");

        let error = reclaim_session_store_space(&db_path).expect_err("抢不到锁必须拒绝");

        assert!(
            error.starts_with("maintenance_lock_busy"),
            "必须是显式的维护锁拒绝（实际：{error}）"
        );
        let _ = std::fs::remove_file(&db_path);
    }
}

/// 规范化投影只携带「内容」，索引行才是**元数据真值**（标题 / 时间 / 轮次 / 归属）。
///
/// 存量库里 `normalized_sessions` 的这几列可能是 0 / 空（写入方只落消息与 trace）：
/// 若直接用投影替换索引行，`updated_at_ms` 会被抹成 0，启动修复再用 `now` 兜底 ⇒
/// 会话在侧边栏「凭空跳到最新」，列表顺序每次启动都漂移（真实库 227 行里有 7 行如此）。
/// 规则：**投影为空才继承索引**，非空值一律以内容侧为准。
pub(crate) fn inherit_index_metadata(indexed: &SessionState, state: &mut SessionState) {
    if state.updated_at_ms == 0 {
        state.updated_at_ms = indexed.updated_at_ms;
    }
    if state.turn_count == 0 {
        state.turn_count = indexed.turn_count;
    }
    if state.title.trim().is_empty() && !indexed.title.trim().is_empty() {
        state.title = indexed.title.clone();
    }
    if state.summary.trim().is_empty() {
        state.summary = indexed.summary.clone();
    }
    if state.last_referenced_file.is_none() {
        state.last_referenced_file = indexed.last_referenced_file.clone();
    }
    if state.workspace_id.is_none() {
        state.workspace_id = indexed.workspace_id.clone();
    }
    if state.title_override.is_none() {
        state.title_override = indexed.title_override.clone();
    }
    if indexed.archived {
        state.archived = true;
    }
}
