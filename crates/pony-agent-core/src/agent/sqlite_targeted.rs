//! B148：SQLite 会话库的**轻量元数据索引**与**单会话定向装载**。
//!
//! 为什么独立成文件：`sqlite_session.rs` 已计入 `scripts/file-size-baseline.json` 的文件
//! 行数棘轮（净值只许减），本模块承载新增实作；`sqlite_session.rs` 只保留 trait 接线。
//!
//! ## 语义锚点（改动前先读）
//!
//! - `load_session_meta_index`：**只发一条投影 SQL**，不反序列化整包 blob、不读 trace、
//!   不 fold 日志。`has_content` 与 `store::session_is_persistable` 同口径（另接受
//!   `turnCount > 0`：轻量索引行内容全在日志/投影里，其轮次仍是「有内容」的证据）。
//! - `load_session_targeted`：结果必须与 `load_store().sessions[id]` **逐字段一致**。
//!   顺序与全量装载对齐：索引视图（legacy blob 或 normalized 投影）→ 日志 fold
//!   （`fold_targeted::read_single_session_truth`，复用 fold 的同一套投影规则）→
//!   索引投影回填（仅轻量行）→ trace 投影补全 → 事件重建（仅空历史）。
//! - `load_store_metadata`：全局元数据（附件 / workspace / 授权 / 版本），不含会话。
//!
//! 失败语义：任一维护步骤失败都**保持索引视图**（与全量装载的 grandfather 一致），
//! 只有「索引与日志都没有该会话」才返回 `NotFound`。

use rusqlite::{params, Connection, OptionalExtension};

use super::commit;
use crate::agent::session::fold_targeted::{read_single_session_truth, SingleSessionTruth};
use crate::agent::session::{
    PersistedStore, SessionBackend, SessionMetaEntry, SessionState, TargetedSessionLoad,
    TurnTraceRecord,
};
use super::SqliteSessionBackend;

/// legacy blob 的元数据投影：字段在 SQLite 侧取值，Rust 只收标量。
///
/// `json_valid` 先挡坏行；`json_extract` 取不到键 → NULL，由 `COALESCE` 兜底。
///
/// `include_persisted_content` 为真时，`has_content` 额外认「内容只落在规范化表」的会话：
/// 全量装载会把这类行的消息 / trace / 历史节点合并进内存态（`session_is_persistable`
/// 为真、列表可见），稀疏列表若只认 blob 就会静默漏会话——真实库 227 行里恰有 7 行
/// 属此类（blob 是内容为空的索引行）。老库没有规范化表 ⇒ 传 `false`，否则 `prepare`
/// 整个失败会把元数据索引降级成空列表。
fn legacy_meta_sql(include_persisted_content: bool) -> String {
    let persisted_content = if include_persisted_content {
        "\n              OR EXISTS (SELECT 1 FROM normalized_messages m WHERE m.session_id = s.conversation_id)\
         \n              OR EXISTS (SELECT 1 FROM normalized_turn_traces t WHERE t.session_id = s.conversation_id)\
         \n              OR EXISTS (SELECT 1 FROM normalized_history_nodes n WHERE n.session_id = s.conversation_id)"
    } else {
        ""
    };
    format!(
        r#"
SELECT conversation_id,
       COALESCE(json_extract(session_data, '$.titleOverride'),
                json_extract(session_data, '$.title'), title, '') AS effective_title,
       json_extract(session_data, '$.titleOverride') AS title_override,
       COALESCE(json_extract(session_data, '$.summary'), '') AS summary,
       COALESCE(json_extract(session_data, '$.turnCount'), 0) AS turn_count,
       COALESCE(updated_at_ms, 0) AS updated_at_ms,
       json_extract(session_data, '$.workspaceId') AS workspace_id,
       COALESCE(json_extract(session_data, '$.archived'), 0) AS archived,
       CASE WHEN instr(session_data, '"__index_only"') > 0 THEN 1 ELSE 0 END AS index_only,
       CASE WHEN COALESCE(json_extract(session_data, '$.turnCount'), 0) > 0
              OR COALESCE(json_array_length(session_data, '$.history'), 0) > 0
              OR COALESCE(json_array_length(session_data, '$.turnTraceHistory'), 0) > 0
              OR COALESCE(json_array_length(session_data, '$.longTermMemoryEntries'), 0) > 0
              OR COALESCE(json_array_length(session_data, '$.memoryWriteEvidence'), 0) > 0
              OR COALESCE(json_array_length(session_data, '$.memoryWriteHookTraceRecords'), 0) > 0
              OR COALESCE(json_array_length(session_data, '$.historyStateEvidence'), 0) > 0
              OR COALESCE(json_array_length(session_data, '$.historyNodes'), 0) > 0{persisted_content}
            THEN 1 ELSE 0 END AS has_content,
       json_extract(session_data, '$.lastReferencedFile') AS last_referenced_file
  FROM sessions s
 WHERE json_valid(session_data)
"#
    )
}

/// 规范化表是否齐备（老库可能没有，或缺其一：此时只能按 blob 判内容）。
fn normalized_tables_available(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
         AND name IN ('normalized_messages', 'normalized_turn_traces', 'normalized_history_nodes')",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count == 3)
    .unwrap_or(false)
}

/// 规范化库的元数据投影（`normalized_sessions` 本身就是元数据表，不再读 blob）。
const NORMALIZED_META_SQL: &str = r#"
SELECT session_id,
       COALESCE(title_override, title, '') AS effective_title,
       title_override,
       COALESCE(summary, '') AS summary,
       COALESCE(turn_count, 0) AS turn_count,
       COALESCE(updated_at_ms, 0) AS updated_at_ms,
       workspace_id,
       COALESCE(archived, 0) AS archived,
       CASE WHEN COALESCE(turn_count, 0) > 0
              OR EXISTS(SELECT 1 FROM normalized_messages m WHERE m.session_id = s.session_id)
              OR EXISTS(SELECT 1 FROM normalized_turn_traces t WHERE t.session_id = s.session_id)
              OR EXISTS(SELECT 1 FROM normalized_history_nodes n WHERE n.session_id = s.session_id)
            THEN 1 ELSE 0 END AS has_content,
       last_referenced_file
  FROM normalized_sessions s
"#;

/// 迁移相位（与 `load_index_store` 同口径；缺失/不可读按 legacy）。
fn normalized_phase(conn: &Connection) -> bool {
    let phase: String = conn
        .query_row(
            "SELECT value FROM store_metadata WHERE key = 'storage.normalized.v1.phase'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "legacy".to_string());
    phase == "observing" || phase == "retired"
}

impl SqliteSessionBackend {
    /// B148：单会话 legacy 索引视图（blob + trace 表 materialize）。
    ///
    /// 与 `load_index_store` 的逐行逻辑同源：轻量行**不读** legacy trace 表（真源是日志/
    /// 规范化投影），非轻量行按表恢复并 materialize 节点 refs。返回 `(state, index_only)`。
    fn load_targeted_index_view(
        &self,
        conn: &Connection,
        session_id: &str,
    ) -> Option<(SessionState, bool)> {
        let row: Option<String> = conn
            .query_row(
                "SELECT session_data FROM sessions WHERE conversation_id = ?1",
                params![session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()?;
        let data = row?;
        let mut session: SessionState = serde_json::from_str(&data).ok()?;
        if session.conversation_id != session_id {
            session.conversation_id = session_id.to_string();
        }
        let index_only = commit::is_index_only_row(&data);
        if index_only {
            return Some((session, true));
        }
        let table_traces = self.read_session_traces(conn, session_id).ok()?;
        session.turn_trace_history = self.merge_trace_history(&session, table_traces.clone());
        materialize_node_traces(&mut session, &table_traces);
        Some((session, false))
    }

    /// B148：定向装载实作（trait 薄壳在下方）。
    pub(crate) fn load_session_targeted_impl(&self, session_id: &str) -> TargetedSessionLoad {
        // 连接锁不可重入：SQL 读取全部收在本作用域内，出作用域后再做日志 fold 与
        // trace / 历史补全（它们各自要重新取连接锁，嵌套加锁即死锁）。
        let (mut indexed, index_only) = {
            let slot = match self.connection() {
                Ok(slot) => slot,
                Err(error) => {
                    eprintln!(
                        "[pony-agent][session] targeted_load_failed: open error ({error}) session={session_id}"
                    );
                    return TargetedSessionLoad::Unsupported;
                }
            };
            let Some(conn) = slot.as_ref() else {
                return TargetedSessionLoad::Unsupported;
            };
            if normalized_phase(conn) {
                // normalized 相位：单会话定向投影（与 `load_normalized_session` 同源，
                // 区别是复用已持有的连接，不再二次加锁）。
                let view = self
                    .load_store_normalized(conn, Some(session_id))
                    .and_then(|store| {
                        store
                            .sessions
                            .into_iter()
                            .next()
                            .map(|(_, state)| state)
                    });
                (view, false)
            } else {
                match self.load_targeted_index_view(conn, session_id) {
                    Some((state, index_only)) => (Some(state), index_only),
                    None => (None, false),
                }
            }
        };

        // 1) 日志真源合并（与全量装载同一套 apply_index_projection 规则）。
        let mut state = match read_single_session_truth(
            &self.session_log_root(),
            session_id,
            indexed.clone(),
        ) {
            SingleSessionTruth::Merged(state) => state,
            SingleSessionTruth::Deleted => return TargetedSessionLoad::NotFound,
            SingleSessionTruth::Absent => return TargetedSessionLoad::NotFound,
            SingleSessionTruth::Failed(reason) => {
                eprintln!(
                    "[pony-agent][session] targeted_load_fold_failed: session={session_id} reason={reason} — 回落索引视图"
                );
                match indexed.take() {
                    Some(state) => state,
                    // 与全量装载一致：索引没有这一行且日志 fold 失败 ⇒ 该会话不可用。
                    None => return TargetedSessionLoad::NotFound,
                }
            }
        };

        // 2) 轻量索引行：索引侧投影回填（不读日志；与 fill_index_only_rows_from_index_projection 同义）。
        if index_only && state.history.is_empty() && state.history_nodes.is_empty() {
            if let Some(projection) = self.load_normalized_session(session_id) {
                if !projection.history.is_empty()
                    || !projection.history_nodes.is_empty()
                    || !projection.turn_trace_history.is_empty()
                {
                    let mut merged = projection;
                    // 投影只带内容：标题/时间/轮次/归属仍以索引行为真值（见 helper 文档）。
                    if let Some(indexed_row) = indexed.as_ref() {
                        commit::inherit_index_metadata(indexed_row, &mut merged);
                    }
                    state = merged;
                }
            }
        }
        if state.conversation_id != session_id {
            state.conversation_id = session_id.to_string();
        }

        // 3) 维护项：trace 投影补全 + 空历史事件重建（显式恢复 ⇒ 不设预算上限）。
        let mut sessions = std::collections::HashMap::new();
        sessions.insert(session_id.to_string(), state);
        self.attach_index_trace_projections_with_deadline(
            &mut sessions,
            &std::collections::HashSet::from([session_id.to_string()]),
            None,
        );
        self.rebuild_history_from_turn_events_when_projection_empty_with_deadline(
            &mut sessions,
            None,
        );
        match sessions.remove(session_id) {
            Some(state) => TargetedSessionLoad::Loaded(Box::new(state)),
            None => TargetedSessionLoad::NotFound,
        }
    }
}

/// 节点 trace materialize（与 `load_index_store` 的逐行逻辑逐字同义）。
///
/// `by_id` 必须从**全量表**构建：fork 分支/超 24 轮节点的 refs 不在顶层投影里，
/// 用顶层过滤后的列表查会把这些 trace 永久判成 missing。
fn materialize_node_traces(session: &mut SessionState, table_traces: &[TurnTraceRecord]) {
    let full_table_by_id: std::collections::HashMap<&str, &TurnTraceRecord> = table_traces
        .iter()
        .map(|trace| (trace.turn_id.as_str(), trace))
        .collect();
    for node in &mut session.history_nodes {
        let Some(refs) = &node.turn_trace_refs else {
            continue;
        };
        if refs.is_empty() {
            node.turn_trace_history.clear();
            continue;
        }
        let mut missing = 0usize;
        node.turn_trace_history = refs
            .iter()
            .filter_map(|reference| match full_table_by_id.get(reference.turn_id.as_str()) {
                Some(trace) => Some((*trace).clone()),
                None => {
                    missing += 1;
                    None
                }
            })
            .collect();
        if missing > 0 {
            eprintln!(
                "[pony-agent][session] materialize node {}: {} ref(s) missing from trace table",
                node.node_id, missing
            );
        }
    }
}

impl SqliteSessionBackend {
    pub(crate) fn load_store_metadata_impl(&self) -> Option<PersistedStore> {
        let slot = self.connection().ok()?;
        let conn = slot.as_ref()?;
        Some(self.persisted_store_tail(conn))
    }

    pub(crate) fn load_session_meta_index_impl(&self) -> Option<Vec<SessionMetaEntry>> {
        let slot = self.connection().ok()?;
        let conn = slot.as_ref()?;
        let versions = commit::load_session_versions(conn);
        let normalized = normalized_phase(conn);
        let mut entries = Vec::new();
        if normalized {
            let mut statement = conn.prepare(NORMALIZED_META_SQL).ok()?;
            let rows = statement
                .query_map([], |row| {
                    Ok(SessionMetaEntry {
                        session_id: row.get(0)?,
                        title: row.get(1)?,
                        title_override: row.get(2)?,
                        summary: row.get(3)?,
                        turn_count: row.get::<_, i64>(4)?.max(0) as usize,
                        updated_at_ms: row.get::<_, i64>(5)?.max(0) as u64,
                        workspace_id: row.get(6)?,
                        archived: row.get::<_, i64>(7)? != 0,
                        has_content: row.get::<_, i64>(8)? != 0,
                        last_referenced_file: row.get(9)?,
                        index_only: false,
                        version: None,
                    })
                })
                .ok()?;
            for row in rows {
                entries.push(row.ok()?);
            }
        } else {
            let sql = legacy_meta_sql(normalized_tables_available(conn));
            let mut statement = conn.prepare(&sql).ok()?;
            let rows = statement
                .query_map([], |row| {
                    Ok(SessionMetaEntry {
                        session_id: row.get(0)?,
                        title: row.get(1)?,
                        title_override: row.get(2)?,
                        summary: row.get(3)?,
                        turn_count: row.get::<_, i64>(4)?.max(0) as usize,
                        updated_at_ms: row.get::<_, i64>(5)?.max(0) as u64,
                        workspace_id: row.get(6)?,
                        archived: row.get::<_, i64>(7)? != 0,
                        index_only: row.get::<_, i64>(8)? != 0,
                        has_content: row.get::<_, i64>(9)? != 0,
                        last_referenced_file: row.get(10)?,
                        version: None,
                    })
                })
                .ok()?;
            for row in rows {
                entries.push(row.ok()?);
            }
        }
        for entry in &mut entries {
            entry.version = versions.get(&entry.session_id).map(|version| version.0);
        }
        Some(entries)
    }
}
