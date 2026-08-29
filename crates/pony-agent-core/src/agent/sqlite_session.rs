use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};

use super::session::{
    AttachmentAsset, FileSessionBackend, HistoryBranch, HistoryCheckoutMode, HistoryCheckoutStatus,
    HistoryCursor, HistoryCursorMode, HistoryNode, HistoryNodeKind, PersistCommand,
    PersistCommandOutcome, PersistedStore, SeparateTraceTableMode, SessionBackend,
    SessionBackendMutationResult, SessionBackendTraceLoadResult, SessionState,
    SessionTraceMutation, TraceMigrationState, TurnHistoryMessage, TurnTraceRecord,
};

const SQLITE_TRACE_HISTORY_LIMIT: usize = 24;

/// 测试注入开关：flush_events_tx 在事件行写入后、计数器更新前强制失败，
/// 用于验证"事件 + 快照"同事务的回滚原子性（PA-091）。
/// 按 db 路径后缀匹配（全局标志避免误伤并行测试的其他 backend）。
#[cfg(test)]
pub static TEST_INJECT_FLUSH_FAILURE_DB_SUFFIX: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

/// PA-095 #7：trace 行 JSON 打补丁——buildContextObservation 置空 +
/// buildContextObservationRef 写入（camelCase 与 TurnTraceRecord serde 一致）。
/// 只动这两个键，其余键原样保留；解析失败返回 None（调用方记日志跳过）。
fn patch_trace_json_observation_ref(json: &str, reference: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(json).ok()?;
    let object = value.as_object_mut()?;
    object.insert(
        "buildContextObservation".to_string(),
        serde_json::Value::Null,
    );
    object.insert(
        "buildContextObservationRef".to_string(),
        serde_json::Value::String(reference.to_string()),
    );
    serde_json::to_string(&value).ok()
}

/// PA-091：从 blob 会话状态反推事件（回填用）。
/// - history 按 user 消息切分 turn 边界（无法配对的消息按"单条消息 = 独立 turn"合成）；
/// - assistant 消息携带 `chunk_missing: true`（过程 chunk 不可恢复）；
/// - tool_activities → ToolCall/ToolResult（缺 started_at/duration 的字段为 None）；
/// - provider_call_records → ProviderUsage（usage 缺失时以 turn 级 token 字段反推）。
fn derive_events_from_session(
    session: &crate::agent::session::SessionState,
) -> Vec<crate::agent::turn_event::TurnEvent> {
    use crate::agent::turn_event::{TurnEndReason, TurnEvent};
    let mut events: Vec<TurnEvent> = Vec::new();
    let mut current_turn: Option<String> = None;
    for message in &session.history {
        match message.role.as_str() {
            "user" => {
                if let Some(turn_id) = current_turn.take() {
                    events.push(TurnEvent::TurnEnd {
                        turn_id,
                        reason: TurnEndReason::Completed,
                        turn_duration_ms: None,
                    });
                }
                let turn_id = message
                    .turn_id
                    .clone()
                    .unwrap_or_else(|| format!("backfill-{}", events.len()));
                events.push(TurnEvent::TurnStart {
                    turn_id: turn_id.clone(),
                });
                events.push(TurnEvent::UserMessage {
                    turn_id: turn_id.clone(),
                    text: message.content.clone(),
                    attachments: message.attachments.clone(),
                });
                current_turn = Some(turn_id);
            }
            "assistant" => {
                let turn_id = current_turn
                    .clone()
                    .unwrap_or_else(|| format!("backfill-{}", events.len()));
                events.push(TurnEvent::AssistantMessage {
                    turn_id: turn_id.clone(),
                    step: 0,
                    text: message.content.clone(),
                    reasoning_content: message.reasoning_content.clone(),
                    usage: None,
                    chunk_missing: Some(true),
                });
            }
            _ => {}
        }
    }
    if let Some(turn_id) = current_turn {
        events.push(TurnEvent::TurnEnd {
            turn_id,
            reason: TurnEndReason::Completed,
            turn_duration_ms: None,
        });
    }
    // trace 反推 tool / provider 事件
    for trace in &session.turn_trace_history {
        let turn_id = trace.turn_id.clone();
        // 审核 P1：legacy 内嵌 build_context_observation 派生 ContextObservation
        // 事件（内存带 payload，flush 时经外置逻辑自动落 bco 表）——否则清表
        // 重建后 observation 为 None 且无 ref，数据永久丢失。
        if let Some(observation) = &trace.build_context_observation {
            events.push(TurnEvent::ContextObservation {
                turn_id: turn_id.clone(),
                step: 0,
                observation: Some(observation.clone()),
                observation_ref: None,
            });
        }
        for activity in &trace.tool_activities {
            if activity.status == "running" {
                events.push(TurnEvent::ToolCall {
                    turn_id: turn_id.clone(),
                    step: 0,
                    call_id: activity.id.clone(),
                    name: activity.name.clone(),
                    arguments: activity.arguments_text.clone().unwrap_or_default(),
                    started_at_ms: None,
                });
            } else {
                events.push(TurnEvent::ToolResult {
                    turn_id: turn_id.clone(),
                    step: 0,
                    call_id: activity.id.clone(),
                    result: activity.result_text.clone(),
                    error: activity.error.as_ref().map(|e| e.to_string()),
                    status: Some(activity.status.clone()),
                    duration_ms: activity.duration_seconds.map(|s| (s * 1000.0) as u64),
                    artifacts: activity.artifacts.clone(),
                    capability_invocation: activity.capability_invocation.clone(),
                });
            }
        }
        for (index, record) in trace.provider_call_records.iter().enumerate() {
            events.push(TurnEvent::ProviderUsage {
                turn_id: turn_id.clone(),
                // 审核 P1：step 按记录序号递增（addReplacing 键 (turn_id, step)，
                // 全 0 会让多条记录坍缩为一条，重建 provider_call_records 数据缺失）。
                step: index as u32,
                request_kind: record.request_kind.clone(),
                usage: crate::agent::provider::TokenUsage {
                    input_tokens: record.input_tokens,
                    cache_hit_input_tokens: record.cache_hit_input_tokens,
                    cache_hit_source: None,
                    reasoning_tokens: record.reasoning_tokens,
                    output_tokens: record.output_tokens,
                    total_tokens: record.total_tokens,
                },
                cache_hit_input_tokens: record.cache_hit_input_tokens,
                cache_miss_input_tokens: record.cache_miss_input_tokens,
                prefix_mutation_reasons: Vec::new(),
                first_token_latency_ms: record.first_token_latency_ms,
                turn_duration_ms: record.turn_duration_ms,
                latency_kind: record.latency_kind.clone(),
                provider: trace.provider_name.clone().unwrap_or_default(),
                model: trace.provider_model.clone().unwrap_or_default(),
            });
        }
    }
    events
}

/// SQLite-backed session storage.
///
/// Stores each session as an individual row with a JSON blob, enabling:
/// - Incremental upserts (write only the sessions that exist in the store)
/// - Concurrent access safety via SQLite's WAL mode
/// - Future query capabilities for trace data, history graphs, etc.
///
/// Schema:
/// ```sql
/// sessions(conversation_id PK, title, updated_at_ms, session_data)
/// store_metadata(key PK, value)
/// ```
///
/// The backend keeps a single pooled `Connection` behind a `Mutex`. This is
/// critical for the turn hot path: `save_store` is invoked many times per turn
/// (append_turn, record_turn_trace, annotate terminal events, hook traces, …),
/// so opening a fresh connection — re-running `PRAGMA journal_mode=WAL`,
/// `CREATE TABLE IF NOT EXISTS`, fsync-ing the WAL header — on every save made
/// each save cost tens of milliseconds and churned the WAL. Reusing one
/// connection turns those repeated saves into cheap in-process transactions.
pub struct SqliteSessionBackend {
    db_path: PathBuf,
    attachment_root: PathBuf,
    trace_mode: SeparateTraceTableMode,
    /// Path to the legacy JSON file, used for one-time migration.
    legacy_json_path: Option<PathBuf>,
    /// Lazily opened, then reused for the lifetime of the backend.
    connection: Mutex<Option<Connection>>,
}

impl SqliteSessionBackend {
    pub fn new(db_path: PathBuf) -> Self {
        Self::new_with_trace_mode(db_path, SeparateTraceTableMode::Off)
    }

    pub fn new_with_trace_mode(db_path: PathBuf, trace_mode: SeparateTraceTableMode) -> Self {
        let attachment_root = db_path
            .parent()
            .map(|parent| parent.join("attachments"))
            .unwrap_or_else(|| PathBuf::from("attachments"));

        // Detect legacy JSON file for migration
        let legacy_json_path = db_path
            .parent()
            .map(|dir| dir.join("sessions.json"))
            .filter(|path| path.exists());

        Self {
            db_path,
            attachment_root,
            trace_mode,
            legacy_json_path,
            connection: Mutex::new(None),
        }
    }

    /// Returns the pooled connection, opening and initializing it on first use.
    ///
    /// `journal_mode=WAL` is a persistent database property, so it is set once
    /// when the connection is first created rather than on every write.
    /// `wal_autocheckpoint` keeps the `-wal` file bounded so it does not grow
    /// unbounded across the many saves issued during a turn.
    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Option<Connection>>, String> {
        let mut slot = self.connection.lock().map_err(|e| format!("lock: {e}"))?;
        if slot.is_none() {
            let conn = Connection::open(&self.db_path).map_err(|e| format!("open db: {e}"))?;
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=5000;
                 PRAGMA wal_autocheckpoint=256;",
            )
            .map_err(|e| format!("pragma: {e}"))?;
            self.ensure_schema(&conn)?;
            self.migrate_from_json(&conn)?;
            *slot = Some(conn);
        }
        Ok(slot)
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                session_data TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS store_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS session_turn_traces (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                trace_order INTEGER NOT NULL DEFAULT 0,
                trace_data TEXT NOT NULL,
                PRIMARY KEY (session_id, turn_id)
             );
             CREATE INDEX IF NOT EXISTS idx_session_turn_traces_session_updated
                ON session_turn_traces (session_id, trace_order, updated_at_ms);
             CREATE TABLE IF NOT EXISTS turn_events (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                branch_id TEXT NOT NULL DEFAULT 'main',
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                PRIMARY KEY (session_id, seq)
             );
             CREATE INDEX IF NOT EXISTS idx_turn_events_turn
                ON turn_events (session_id, turn_id);
             CREATE TABLE IF NOT EXISTS turn_events_archive (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                archived_at_ms INTEGER NOT NULL,
                PRIMARY KEY (session_id, seq)
             );
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| format!("schema: {e}"))?;
        // PA-095：事件 schema 版本契约——缺失视为 v1 并回填（存量库保持可读）；
        // INSERT OR IGNORE 保留已有值（版本升级由迁移分支显式改写）。
        conn.execute(
            "INSERT OR IGNORE INTO store_metadata (key, value) VALUES ('events.schema_version', ?1)",
            params![crate::agent::turn_event::EVENT_SCHEMA_VERSION.to_string()],
        )
        .map_err(|e| format!("schema version seed: {e}"))?;
        self.ensure_normalized_schema(conn)?;
        Ok(())
    }

    /// PA-089 阶段 1：规范化并行表（方案 A——旧 sessions 保留为 blob 表，
    /// 新增 normalized_* 表，阶段 6b 再改名/删除旧表）。
    /// v6：复合主键 + raw_json 逃生舱 + history_state_evidence 独立列（与 spec 3.6 定稿一致）。
    /// 本阶段只建表，不改变任何旧读写路径；回填（阶段 2）前这些表为空。
    fn ensure_normalized_schema(&self, conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS normalized_sessions (
                session_id TEXT PRIMARY KEY,
                workspace_id TEXT,
                title TEXT NOT NULL DEFAULT '',
                title_override TEXT,
                archived INTEGER NOT NULL DEFAULT 0,
                summary TEXT NOT NULL DEFAULT '',
                turn_count INTEGER NOT NULL DEFAULT 0,
                last_referenced_file TEXT,
                created_at_ms INTEGER,
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                state_version INTEGER NOT NULL DEFAULT 0,
                trace_migration_state TEXT NOT NULL DEFAULT 'legacy_blob',
                turn_trace_refs_json TEXT,
                provider_native_transcript_json TEXT,
                history_state_evidence_json TEXT,
                memory_json TEXT
             );
             CREATE TABLE IF NOT EXISTS normalized_turns (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                phase TEXT,
                status TEXT,
                user_message_id TEXT,
                assistant_message_id TEXT,
                started_at_ms INTEGER,
                completed_at_ms INTEGER,
                created_at_ms INTEGER,
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (session_id, turn_id),
                FOREIGN KEY (session_id) REFERENCES normalized_sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS normalized_messages (
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                turn_id TEXT,
                ordinal INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                reasoning_content TEXT,
                status TEXT,
                model_name TEXT,
                token_count INTEGER,
                attachments_json TEXT,
                created_at_ms INTEGER,
                PRIMARY KEY (session_id, message_id),
                FOREIGN KEY (session_id) REFERENCES normalized_sessions(session_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_normalized_messages_session_turn_role
                ON normalized_messages (session_id, turn_id, role);
             CREATE TABLE IF NOT EXISTS normalized_turn_traces (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                trace_order INTEGER NOT NULL DEFAULT 0,
                phase TEXT,
                provider_name TEXT,
                provider_model TEXT,
                provider_mode TEXT,
                session_summary TEXT,
                fallback_reason TEXT,
                error TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                total_tokens INTEGER,
                first_token_latency_ms INTEGER,
                turn_duration_ms INTEGER,
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                extension_json TEXT,
                raw_json TEXT,
                PRIMARY KEY (session_id, turn_id),
                FOREIGN KEY (session_id) REFERENCES normalized_sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS normalized_trace_steps (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                kind TEXT,
                state TEXT,
                label TEXT,
                text TEXT,
                error TEXT,
                duration_ms INTEGER,
                extension_json TEXT,
                raw_json TEXT,
                PRIMARY KEY (session_id, turn_id, ordinal),
                FOREIGN KEY (session_id, turn_id) REFERENCES normalized_turn_traces(session_id, turn_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS normalized_trace_timeline (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                entry_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                kind TEXT,
                label TEXT,
                state TEXT,
                text TEXT,
                reasoning_content TEXT,
                duration_ms INTEGER,
                extension_json TEXT,
                raw_json TEXT,
                PRIMARY KEY (session_id, turn_id, entry_id),
                FOREIGN KEY (session_id, turn_id) REFERENCES normalized_turn_traces(session_id, turn_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_normalized_timeline_seq
                ON normalized_trace_timeline (session_id, turn_id, sequence);
             CREATE TABLE IF NOT EXISTS normalized_tool_activities (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                activity_id TEXT NOT NULL,
                timeline_entry_id TEXT,
                parent_activity_id TEXT,
                name TEXT NOT NULL,
                canonical_tool_name TEXT,
                status TEXT NOT NULL,
                description TEXT,
                arguments_preview TEXT,
                result_preview TEXT,
                result_bytes INTEGER,
                result_truncated INTEGER NOT NULL DEFAULT 0,
                error_json TEXT,
                duration_seconds REAL,
                created_at_ms INTEGER,
                extension_json TEXT,
                raw_json TEXT,
                timeline_variants_json TEXT,
                PRIMARY KEY (session_id, turn_id, activity_id),
                FOREIGN KEY (session_id, turn_id) REFERENCES normalized_turn_traces(session_id, turn_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_normalized_tool_activities_turn
                ON normalized_tool_activities (session_id, turn_id, created_at_ms);
             CREATE TABLE IF NOT EXISTS normalized_history_branches (
                session_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                base_node_id TEXT,
                head_node_id TEXT,
                forked_from_branch_id TEXT,
                forked_from_node_id TEXT,
                label TEXT NOT NULL DEFAULT '',
                created_at_ms INTEGER NOT NULL DEFAULT 0,
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (session_id, branch_id),
                FOREIGN KEY (session_id) REFERENCES normalized_sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS normalized_history_nodes (
                session_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                parent_node_id TEXT,
                branch_id TEXT NOT NULL,
                forked_from_node_id TEXT,
                kind TEXT NOT NULL DEFAULT 'checkpoint',
                turn_id TEXT,
                turn_trace_refs_json TEXT,
                run_id TEXT,
                workspace_ref_json TEXT,
                summary TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL DEFAULT '',
                created_at_ms INTEGER NOT NULL DEFAULT 0,
                snapshot_json TEXT,
                event_seq_range_json TEXT,
                PRIMARY KEY (session_id, node_id),
                FOREIGN KEY (session_id) REFERENCES normalized_sessions(session_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_normalized_history_nodes_branch
                ON normalized_history_nodes (session_id, branch_id, parent_node_id);
             CREATE TABLE IF NOT EXISTS normalized_history_cursor (
                session_id TEXT PRIMARY KEY,
                visible_node_id TEXT,
                active_branch_id TEXT,
                branch_head_node_id TEXT,
                workspace_node_id TEXT,
                cursor_version INTEGER NOT NULL DEFAULT 0,
                mode TEXT NOT NULL DEFAULT 'live',
                checkout_mode TEXT,
                checkout_status TEXT,
                FOREIGN KEY (session_id) REFERENCES normalized_sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS build_context_observations (
                session_id TEXT NOT NULL,
                observation_ref TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (session_id, observation_ref),
                FOREIGN KEY (session_id) REFERENCES normalized_sessions(session_id) ON DELETE CASCADE
             );",
        )
        .map_err(|e| format!("normalized schema: {e}"))?;
        // PA-093：旧库迁移——CREATE TABLE IF NOT EXISTS 不补列，显式 ALTER
        // （列已存在时忽略错误，幂等）。
        let _ = conn.execute(
            "ALTER TABLE normalized_history_cursor
             ADD COLUMN event_watermark INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE normalized_history_nodes
             ADD COLUMN event_seq_range_json TEXT",
            [],
        );
        // PA-094：trace 表降级为投影缓存——行带 seq 水位（折叠水位，清表可重建）。
        let _ = conn.execute(
            "ALTER TABLE normalized_turn_traces
             ADD COLUMN event_watermark INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE normalized_sessions
             ADD COLUMN title_override TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE normalized_sessions
             ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // PA-095 #6（实施后审核 P2）：cursor_version 退役的一次性升级校正——
        // 存量行的 cursor_version 是旧 bump-count 语义，与 event_watermark 不等；
        // 不校正则客户端以旧值作 expected_cursor_version 会陷入永久冲突循环。
        // 幂等（相等行为 no-op）；水位为 0 的全新/纯 legacy 会话不动。
        let _ = conn.execute(
            "UPDATE normalized_history_cursor
             SET cursor_version = event_watermark
             WHERE cursor_version != event_watermark AND event_watermark > 0",
            [],
        );
        Ok(())
    }

    /// One-time migration from the legacy JSON file to SQLite.
    /// Runs automatically when the SQLite DB is empty and a JSON file exists.
    fn migrate_from_json(&self, conn: &Connection) -> Result<(), String> {
        let Some(json_path) = &self.legacy_json_path else {
            return Ok(());
        };

        // Check if DB already has data
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap_or(0);
        if session_count > 0 {
            return Ok(());
        }

        eprintln!(
            "[pony-agent][session] migrating from {} to SQLite",
            json_path.display()
        );

        let legacy = FileSessionBackend::new(json_path.clone());
        let Some(store) = legacy.load_store() else {
            return Ok(());
        };

        self.write_full_store(conn, &store)?;

        eprintln!(
            "[pony-agent][session] migration complete: {} sessions",
            store.sessions.len()
        );
        Ok(())
    }

    fn write_full_store(&self, conn: &Connection, store: &PersistedStore) -> Result<(), String> {
        // Use unchecked_transaction since save_store takes &self (not &mut).
        // This is safe because the pooled connection is only mutated through
        // the pooled lock guard held by the caller.
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;

        let existing_session_ids: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT conversation_id FROM sessions")
                .map_err(|e| format!("prepare session scan: {e}"))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("query session scan: {e}"))?;
            rows.filter_map(Result::ok).collect()
        };

        // Upsert current sessions — scoped to drop the statement before commit
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| format!("prepare session insert: {e}"))?;

            for (id, session) in &store.sessions {
                let mut normalized_session = session.clone();
                if normalized_session.conversation_id != *id {
                    eprintln!(
                        "[pony-agent][session] SQLite save_store normalizing conversation_id: key={} payload={}",
                        id, normalized_session.conversation_id
                    );
                    normalized_session.conversation_id = id.clone();
                }
                let data = serde_json::to_string(&normalized_session)
                    .map_err(|e| format!("serialize session: {e}"))?;
                stmt.execute(params![
                    id,
                    normalized_session.title,
                    normalized_session.updated_at_ms as i64,
                    data
                ])
                .map_err(|e| format!("insert session: {e}"))?;
                // Always write traces to keep session blob and trace table in sync,
                // regardless of trace_migration_state. This prevents stale traces
                // from lingering in the trace table after checkout/rollback
                // when the fallback persist path is taken.
                // PA-088：authoritative 会话跳过——其 blob 已被 session_state_for_backend
                // 剥离 trace（只剩 refs），此处若用剥离后的空 trace 替换表会清空数据；
                // 表已由 save_to_backend 的 replace_session_traces 在剥离前写入。
                if normalized_session.trace_migration_state
                    != TraceMigrationState::TraceTableAuthoritative
                {
                    self.replace_session_traces_tx(
                        &tx,
                        id,
                        &normalized_session.turn_trace_history,
                    )?;
                }
            }
        }

        // Remove sessions that are no longer present in the store.
        {
            let removed_ids = existing_session_ids
                .into_iter()
                .filter(|id| !store.sessions.contains_key(id))
                .collect::<Vec<_>>();
            if !removed_ids.is_empty() {
                let mut delete_stmt = tx
                    .prepare("DELETE FROM sessions WHERE conversation_id = ?1")
                    .map_err(|e| format!("prepare session delete: {e}"))?;
                let mut delete_trace_stmt = tx
                    .prepare("DELETE FROM session_turn_traces WHERE session_id = ?1")
                    .map_err(|e| format!("prepare trace delete: {e}"))?;
                for id in removed_ids {
                    delete_stmt
                        .execute(params![id])
                        .map_err(|e| format!("delete session: {e}"))?;
                    delete_trace_stmt
                        .execute(params![id])
                        .map_err(|e| format!("delete session traces: {e}"))?;
                }
            }
        }

        // Upsert metadata — scoped to drop the statement before commit
        {
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

            let mut meta_stmt = tx
                .prepare("INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?1, ?2)")
                .map_err(|e| format!("prepare meta insert: {e}"))?;

            for (key, value) in &metadata_entries {
                if let Some(val) = value {
                    meta_stmt
                        .execute(params![key, val])
                        .map_err(|e| format!("insert metadata: {e}"))?;
                }
            }
        }

        tx.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    }

    fn replace_session_traces_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        traces: &[TurnTraceRecord],
    ) -> Result<(), String> {
        tx.execute(
            "DELETE FROM session_turn_traces WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|e| format!("delete trace rows: {e}"))?;

        if traces.is_empty() {
            return Ok(());
        }

        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO session_turn_traces (
                    session_id, turn_id, updated_at_ms, trace_order, trace_data
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|e| format!("prepare trace insert: {e}"))?;

        for (index, trace) in traces.iter().enumerate() {
            let raw = serde_json::to_string(trace).map_err(|e| format!("serialize trace: {e}"))?;
            stmt.execute(params![
                session_id,
                trace.turn_id,
                trace.updated_at as i64,
                index as i64,
                raw,
            ])
            .map_err(|e| format!("insert trace row: {e}"))?;
        }

        Ok(())
    }

    fn upsert_turn_trace_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        trace: &TurnTraceRecord,
        trace_order: usize,
        should_prune: bool,
    ) -> Result<(), String> {
        let raw = serde_json::to_string(trace).map_err(|e| format!("serialize trace: {e}"))?;
        tx.execute(
            "INSERT OR REPLACE INTO session_turn_traces (
                session_id, turn_id, updated_at_ms, trace_order, trace_data
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                trace.turn_id,
                trace.updated_at as i64,
                trace_order as i64,
                raw,
            ],
        )
        .map_err(|e| format!("upsert trace row: {e}"))?;
        if should_prune {
            self.prune_session_traces_tx(tx, session_id, SQLITE_TRACE_HISTORY_LIMIT)?;
        }
        Ok(())
    }

    fn prune_session_traces_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        limit: usize,
    ) -> Result<(), String> {
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM session_turn_traces WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("count trace rows: {e}"))?;

        if count <= limit as i64 {
            return Ok(());
        }

        tx.execute(
            "DELETE FROM session_turn_traces
             WHERE session_id = ?1 AND turn_id IN (
                SELECT turn_id FROM session_turn_traces
                WHERE session_id = ?1
                ORDER BY trace_order ASC, updated_at_ms ASC, turn_id ASC
                LIMIT ?2
             )",
            params![session_id, count - limit as i64],
        )
        .map_err(|e| format!("prune trace rows: {e}"))?;

        let mut stmt = tx
            .prepare(
                "SELECT turn_id FROM session_turn_traces
                 WHERE session_id = ?1
                 ORDER BY trace_order ASC, updated_at_ms ASC, turn_id ASC",
            )
            .map_err(|e| format!("prepare trace reorder scan: {e}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query trace reorder scan: {e}"))?;
        let turn_ids = rows.filter_map(Result::ok).collect::<Vec<_>>();
        let mut reorder_stmt = tx
            .prepare(
                "UPDATE session_turn_traces
                 SET trace_order = ?3
                 WHERE session_id = ?1 AND turn_id = ?2",
            )
            .map_err(|e| format!("prepare trace reorder update: {e}"))?;
        for (index, turn_id) in turn_ids.iter().enumerate() {
            reorder_stmt
                .execute(params![session_id, turn_id, index as i64])
                .map_err(|e| format!("update trace order: {e}"))?;
        }

        Ok(())
    }

    /// PA-090：refs 保护 prune——只删**无任何引用**且超过软上限的最旧记录。
    /// `protected_turn_ids` = 顶层 refs ∪ 全部节点 refs 指向的 turn_id。
    /// fail closed：protected 集合为空但表非空时（refs 缺失/损坏）零删除。
    /// 预留：PA-089 阶段 6b（规范化权威后）由运行时按 refs 保护调用；当前未接线。
    #[allow(dead_code)]
    fn prune_session_traces_protected_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        limit: usize,
        protected_turn_ids: &std::collections::HashSet<String>,
    ) -> Result<(), String> {
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM session_turn_traces WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("count trace rows: {e}"))?;

        if count <= limit as i64 {
            return Ok(());
        }

        // PA-090 fail-closed：protected 集合为空但表非空（refs 缺失/损坏）→ 零删除。
        if protected_turn_ids.is_empty() {
            return Ok(());
        }

        // 收集候选删除（最旧未引用）
        let mut stmt = tx
            .prepare(
                "SELECT turn_id FROM session_turn_traces
                 WHERE session_id = ?1
                 ORDER BY trace_order ASC, updated_at_ms ASC, turn_id ASC",
            )
            .map_err(|e| format!("prepare prune scan: {e}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query prune scan: {e}"))?;
        let mut candidates: Vec<String> = Vec::new();
        for turn_id in rows.filter_map(Result::ok) {
            if !protected_turn_ids.contains(&turn_id) {
                candidates.push(turn_id);
            }
        }

        let excess = (count - limit as i64) as usize;
        let to_delete = candidates.into_iter().take(excess).collect::<Vec<_>>();
        if to_delete.is_empty() {
            return Ok(());
        }

        let mut del_stmt = tx
            .prepare("DELETE FROM session_turn_traces WHERE session_id = ?1 AND turn_id = ?2")
            .map_err(|e| format!("prepare protected prune delete: {e}"))?;
        for turn_id in &to_delete {
            del_stmt
                .execute(params![session_id, turn_id])
                .map_err(|e| format!("protected prune delete: {e}"))?;
        }

        // 重排 trace_order
        let mut reorder_stmt = tx
            .prepare(
                "SELECT turn_id FROM session_turn_traces
                 WHERE session_id = ?1
                 ORDER BY trace_order ASC, updated_at_ms ASC, turn_id ASC",
            )
            .map_err(|e| format!("prepare protected reorder scan: {e}"))?;
        let remaining = reorder_stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query protected reorder scan: {e}"))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let mut update_stmt = tx
            .prepare(
                "UPDATE session_turn_traces
                 SET trace_order = ?3
                 WHERE session_id = ?1 AND turn_id = ?2",
            )
            .map_err(|e| format!("prepare protected reorder update: {e}"))?;
        for (index, turn_id) in remaining.iter().enumerate() {
            update_stmt
                .execute(params![session_id, turn_id, index as i64])
                .map_err(|e| format!("update protected trace order: {e}"))?;
        }

        Ok(())
    }

    fn update_turn_trace_row_tx<F>(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        turn_id: &str,
        mutate: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(&mut TurnTraceRecord),
    {
        let mut stmt = tx
            .prepare(
                "SELECT trace_order, trace_data FROM session_turn_traces
                 WHERE session_id = ?1 AND turn_id = ?2",
            )
            .map_err(|e| format!("prepare trace fetch: {e}"))?;
        let Some((trace_order, raw)) = stmt
            .query_row(params![session_id, turn_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .ok()
        else {
            return Ok(false);
        };

        let mut trace = serde_json::from_str::<TurnTraceRecord>(&raw)
            .map_err(|e| format!("deserialize trace row: {e}"))?;
        mutate(&mut trace);
        self.upsert_turn_trace_tx(tx, session_id, &trace, trace_order as usize, false)?;
        Ok(true)
    }

    fn upsert_session_row_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        session: &SessionState,
    ) -> Result<(), String> {
        let data = serde_json::to_string(session).map_err(|e| format!("serialize session: {e}"))?;
        tx.execute(
            "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session_id,
                session.title,
                session.updated_at_ms as i64,
                data
            ],
        )
        .map_err(|e| format!("upsert session row: {e}"))?;
        Ok(())
    }

    /// PA-095 #7（实施后审核 P1）：读侧水合——ref-only 行按引用还原 observation
    /// （"读路径优先 ref、缺失回退内嵌"承诺的落实）；已有内嵌 payload 的 legacy 行
    /// 原样返回；ref 缺失/未命中 contained（日志 + 保持 None）。
    fn hydrate_trace_observation(
        &self,
        conn: &Connection,
        session_id: &str,
        mut trace: TurnTraceRecord,
    ) -> TurnTraceRecord {
        if trace.build_context_observation.is_some() {
            return trace;
        }
        let Some(reference) = trace.build_context_observation_ref.clone() else {
            return trace;
        };
        match conn.query_row(
            "SELECT payload_json FROM build_context_observations
             WHERE session_id = ?1 AND observation_ref = ?2",
            params![session_id, reference],
            |row| row.get::<_, String>(0),
        ) {
            Ok(payload) => {
                match serde_json::from_str::<crate::agent::provider::BuildContextObservation>(
                    &payload,
                ) {
                    Ok(observation) => trace.build_context_observation = Some(observation),
                    Err(error) => eprintln!(
                        "[pony-agent][session] observation payload decode failed for {reference}: {error}"
                    ),
                }
            }
            Err(error) => eprintln!(
                "[pony-agent][session] observation ref {reference} not resolvable for session {session_id}: {error}"
            ),
        }
        trace
    }

    fn read_session_traces(
        &self,
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<TurnTraceRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT trace_data FROM session_turn_traces
                 WHERE session_id = ?1
                 ORDER BY trace_order ASC, updated_at_ms ASC, turn_id ASC",
            )
            .map_err(|e| format!("prepare trace load: {e}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query trace load: {e}"))?;
        let mut traces = Vec::new();
        for row in rows {
            let raw = row.map_err(|e| format!("read trace row: {e}"))?;
            match serde_json::from_str::<TurnTraceRecord>(&raw) {
                Ok(trace) => {
                    traces.push(self.hydrate_trace_observation(conn, session_id, trace))
                }
                Err(error) => {
                    eprintln!(
                        "[pony-agent][session] malformed trace row for session {}: {}",
                        session_id, error
                    );
                }
            }
        }
        Ok(traces)
    }

    /// PA-089 优化：批量读取全部会话的 trace（一次查询替代每会话一次 = N+1）。
    fn read_all_session_traces(
        &self,
        conn: &Connection,
    ) -> Result<HashMap<String, Vec<TurnTraceRecord>>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, trace_data FROM session_turn_traces
                 ORDER BY session_id, trace_order ASC, updated_at_ms ASC, turn_id ASC",
            )
            .map_err(|e| format!("prepare all traces load: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("query all traces load: {e}"))?;
        let mut by_session: HashMap<String, Vec<TurnTraceRecord>> = HashMap::new();
        for row in rows {
            let (session_id, raw) = row.map_err(|e| format!("read all trace row: {e}"))?;
            match serde_json::from_str::<TurnTraceRecord>(&raw) {
                Ok(trace) => {
                    let hydrated = self.hydrate_trace_observation(conn, &session_id, trace);
                    by_session.entry(session_id).or_default().push(hydrated);
                }
                Err(error) => {
                    eprintln!(
                        "[pony-agent][session] malformed trace row for session {}: {}",
                        session_id, error
                    );
                }
            }
        }
        Ok(by_session)
    }

    fn merge_trace_history(
        &self,
        session: &SessionState,
        table_traces: Vec<TurnTraceRecord>,
    ) -> Vec<TurnTraceRecord> {
        match session.trace_migration_state {
            TraceMigrationState::TraceTableAuthoritative => {
                // PA-090：顶层 trace 按顶层 refs 过滤（只显示当前分支），
                // 表是全量 Union（含所有节点引用的 trace，供 materialize）。
                // refs 缺失（旧数据/未迁移）时回退全表。
                match &session.turn_trace_refs {
                    Some(refs) => {
                        let by_id: HashMap<&str, &TurnTraceRecord> = table_traces
                            .iter()
                            .map(|trace| (trace.turn_id.as_str(), trace))
                            .collect();
                        refs.iter()
                            .filter_map(|reference| {
                                by_id
                                    .get(reference.turn_id.as_str())
                                    .map(|trace| (*trace).clone())
                            })
                            .collect()
                    }
                    None => table_traces,
                }
            }
            TraceMigrationState::LegacyBlob | TraceMigrationState::DualWrite => {
                if table_traces.is_empty() {
                    return session.turn_trace_history.clone();
                }

                let mut table_by_id = table_traces
                    .iter()
                    .cloned()
                    .map(|trace| (trace.turn_id.clone(), trace))
                    .collect::<HashMap<_, _>>();
                let mut merged = Vec::new();

                for legacy_trace in &session.turn_trace_history {
                    if let Some(table_trace) = table_by_id.remove(&legacy_trace.turn_id) {
                        merged.push(table_trace);
                    } else {
                        merged.push(legacy_trace.clone());
                    }
                }

                if !table_by_id.is_empty() {
                    for table_trace in table_traces {
                        if table_by_id.remove(&table_trace.turn_id).is_some() {
                            merged.push(table_trace);
                        }
                    }
                }

                merged
            }
        }
    }

    fn read_metadata<T: serde::de::DeserializeOwned + Default>(
        &self,
        conn: &Connection,
        key: &str,
    ) -> T {
        conn.query_row(
            "SELECT value FROM store_metadata WHERE key = ?1",
            params![key],
            |row| {
                let raw: String = row.get(0)?;
                Ok(serde_json::from_str(&raw).unwrap_or_default())
            },
        )
        .unwrap_or_default()
    }
    fn apply_persist_command_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        command: &PersistCommand,
    ) -> Result<(), String> {
        let epoch = command.epoch();
        let current_epoch: i64 = tx
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'storage.normalized.v1.epoch'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map(|raw| raw.parse::<i64>().unwrap_or(0))
            .unwrap_or(0);
        if epoch < current_epoch as u64 {
            return Err(format!("stale epoch: {epoch} < {current_epoch}"));
        }

        // 确保 normalized_sessions 行存在（FK 依赖）：命令可能来自未回填的会话
        // （如缺失行修复/新会话），INSERT OR IGNORE 兜底。
        if !matches!(command, PersistCommand::PublishMetadata { .. }) {
            let session_id = match command {
                PersistCommand::AppendMessage { session_id, .. }
                | PersistCommand::UpsertTurn { session_id, .. }
                | PersistCommand::AppendTrace { session_id, .. }
                | PersistCommand::UpdateTraceTerminal { session_id, .. }
                | PersistCommand::AppendHookRecords { session_id, .. }
                | PersistCommand::UpdateHistoryNode { session_id, .. }
                | PersistCommand::UpdateCursor { session_id, .. }
                | PersistCommand::UpdateBranch { session_id, .. }
                | PersistCommand::UpdateSessionMeta { session_id, .. }
                | PersistCommand::RemoveSession { session_id, .. }
                | PersistCommand::FlushEvents { session_id, .. } => session_id,
                PersistCommand::PublishMetadata { .. } => unreachable!(),
            };
            tx.execute(
                "INSERT OR IGNORE INTO normalized_sessions (session_id, title, updated_at_ms) VALUES (?1, '', 0)",
                params![session_id],
            )
            .map_err(|e| format!("ensure normalized session: {e}"))?;
        }

        match command {
            PersistCommand::AppendMessage {
                session_id,
                message,
                ordinal,
                ..
            } => {
                let message_id = match &message.turn_id {
                    Some(turn_id) if !turn_id.is_empty() => format!("{turn_id}-{}", message.role),
                    _ => format!("msg-{ordinal}-{}", message.role),
                };
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_messages
                     (session_id, message_id, turn_id, ordinal, role, content, reasoning_content,
                      status, model_name, token_count, attachments_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
                    params![
                        session_id,
                        message_id,
                        message.turn_id,
                        ordinal,
                        message.role,
                        message.content,
                        message.reasoning_content,
                        message
                            .status
                            .as_ref()
                            .map(|s| serde_json::to_string(s).unwrap_or_default()),
                        message.model_name,
                        message.token_count,
                        serde_json::to_string(&message.attachments)
                            .unwrap_or_else(|_| "[]".to_string()),
                    ],
                )
                .map_err(|e| format!("append message: {e}"))?;
            }
            PersistCommand::UpsertTurn {
                session_id, turn, ..
            } => {
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_turns
                     (session_id, turn_id, ordinal, phase, status, user_message_id, assistant_message_id,
                      started_at_ms, completed_at_ms, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10)",
                    params![
                        session_id,
                        turn.turn_id,
                        turn.ordinal,
                        turn.phase,
                        turn.status.as_ref().map(|s| serde_json::to_string(s).unwrap_or_default()),
                        turn.user_message_id,
                        turn.assistant_message_id,
                        turn.started_at_ms,
                        turn.completed_at_ms,
                        turn.updated_at_ms,
                    ],
                )
                .map_err(|e| format!("upsert turn: {e}"))?;
            }
            PersistCommand::AppendTrace {
                session_id,
                trace,
                trace_order,
                ..
            } => {
                let raw =
                    serde_json::to_string(trace).map_err(|e| format!("serialize trace: {e}"))?;
                // PA-094：trace 表降级为投影缓存——行带 seq 水位（折叠水位；
                // 写入时通常为 0，终态 annotate 时由 UpdateTraceTerminal 更新）。
                let event_watermark = trace.sequence.unwrap_or(0);
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_turn_traces
                     (session_id, turn_id, trace_order, phase, provider_name, provider_model, provider_mode,
                      session_summary, fallback_reason, error, input_tokens, output_tokens, total_tokens,
                      first_token_latency_ms, turn_duration_ms, updated_at_ms, extension_json, raw_json, event_watermark)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, NULL, ?17, ?18)",
                    params![
                        session_id,
                        trace.turn_id,
                        trace_order,
                        trace.phase,
                        trace.provider_name,
                        trace.provider_model,
                        trace.provider_mode,
                        trace.session_summary,
                        trace.fallback_reason,
                        trace.error,
                        trace.input_tokens,
                        trace.output_tokens,
                        trace.total_tokens,
                        trace.first_token_latency_ms,
                        trace.turn_duration_ms,
                        trace.updated_at,
                        raw,
                        event_watermark,
                    ],
                )
                .map_err(|e| format!("append trace: {e}"))?;
                // 双写：同步旧 session_turn_traces 表（load_store 读它）+ blob
                tx.execute(
                    "INSERT OR REPLACE INTO session_turn_traces
                     (session_id, turn_id, updated_at_ms, trace_order, trace_data)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        session_id,
                        trace.turn_id,
                        trace.updated_at as i64,
                        trace_order,
                        raw,
                    ],
                )
                .map_err(|e| format!("append legacy trace: {e}"))?;
                self.sync_blob_trace_tx(tx, session_id, trace)?;
            }
            PersistCommand::UpdateTraceTerminal {
                session_id,
                turn_id,
                terminal_patch,
                ..
            } => {
                let extension: Option<String> = tx
                    .query_row(
                        "SELECT extension_json FROM normalized_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
                        params![session_id, turn_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .unwrap_or(None);
                let mut ext: serde_json::Value = extension
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_else(|| serde_json::json!({ "v": 1 }));
                if let Some(event_id) = &terminal_patch.event_id {
                    ext["eventId"] = serde_json::Value::String(event_id.clone());
                }
                if let Some(event_type) = &terminal_patch.event_type {
                    ext["eventType"] = serde_json::Value::String(event_type.clone());
                }
                if let Some(event_version) = &terminal_patch.event_version {
                    ext["eventVersion"] = serde_json::Value::String(event_version.clone());
                }
                if let Some(sequence) = terminal_patch.sequence {
                    ext["sequence"] = serde_json::json!(sequence);
                }
                if let Some(emitted_at_ms) = terminal_patch.emitted_at_ms {
                    ext["emittedAtMs"] = serde_json::json!(emitted_at_ms);
                }
                tx.execute(
                    "UPDATE normalized_turn_traces
                     SET phase = COALESCE(?3, phase), updated_at_ms = ?4, extension_json = ?5,
                         event_watermark = COALESCE(?6, event_watermark)
                     WHERE session_id = ?1 AND turn_id = ?2",
                    params![
                        session_id,
                        turn_id,
                        terminal_patch.phase,
                        terminal_patch.updated_at,
                        serde_json::to_string(&ext).unwrap_or_else(|_| "{}".to_string()),
                        terminal_patch.event_watermark,
                    ],
                )
                .map_err(|e| format!("update trace terminal: {e}"))?;
                // 同步旧 session_turn_traces 表（load_store 读它）
                self.sync_legacy_trace_update_tx(
                    tx,
                    session_id,
                    turn_id,
                    &serde_json::json!({
                        "eventId": terminal_patch.event_id,
                        "eventType": terminal_patch.event_type,
                        "eventVersion": terminal_patch.event_version,
                        "sequence": terminal_patch.sequence,
                        "emittedAtMs": terminal_patch.emitted_at_ms,
                    }),
                    terminal_patch.updated_at,
                )?;
            }
            PersistCommand::AppendHookRecords {
                session_id,
                turn_id,
                hook_trace_records,
                updated_at,
                ..
            } => {
                let extension: Option<String> = tx
                    .query_row(
                        "SELECT extension_json FROM normalized_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
                        params![session_id, turn_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .unwrap_or(None);
                let mut ext: serde_json::Value = extension
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_else(|| serde_json::json!({ "v": 1 }));
                let mut records = ext
                    .get("hookTraceRecords")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                for record in hook_trace_records {
                    records.push(serde_json::to_value(record).unwrap_or(serde_json::Value::Null));
                }
                ext["hookTraceRecords"] = serde_json::Value::Array(records);
                tx.execute(
                    "UPDATE normalized_turn_traces
                     SET updated_at_ms = ?3, extension_json = ?4
                     WHERE session_id = ?1 AND turn_id = ?2",
                    params![
                        session_id,
                        turn_id,
                        updated_at,
                        serde_json::to_string(&ext).unwrap_or_else(|_| "{}".to_string()),
                    ],
                )
                .map_err(|e| format!("append hook records: {e}"))?;
                // 同步旧 session_turn_traces 表
                self.sync_legacy_trace_update_tx(
                    tx,
                    session_id,
                    turn_id,
                    &serde_json::json!({ "hookTraceRecords": hook_trace_records }),
                    *updated_at,
                )?;
            }
            PersistCommand::UpdateHistoryNode {
                session_id, node, ..
            } => {
                let snapshot_json = if !node.history.is_empty() {
                    serde_json::to_string(&serde_json::json!({
                        "history": node.history,
                        "providerNativeTranscript": node.provider_native_transcript,
                        "longTermMemoryEntries": node.long_term_memory_entries,
                        "memoryWriteEvidence": node.memory_write_evidence,
                        "memoryWriteHookTraceRecords": node.memory_write_hook_trace_records,
                        "turnCount": node.turn_count,
                        "lastReferencedFile": node.last_referenced_file,
                    }))
                    .ok()
                } else {
                    None
                };
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_history_nodes
                     (session_id, node_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id,
                      turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms, snapshot_json, event_seq_range_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        session_id,
                        node.node_id,
                        node.parent_node_id,
                        node.branch_id,
                        node.forked_from_node_id,
                        serde_json::to_string(&node.kind).unwrap_or_else(|_| "\"checkpoint\"".to_string()),
                        node.turn_id,
                        serde_json::to_string(&node.turn_trace_refs).unwrap_or_else(|_| "null".to_string()),
                        node.run_id,
                        serde_json::to_string(&node.workspace_ref).unwrap_or_else(|_| "null".to_string()),
                        node.summary,
                        node.title,
                        node.created_at_ms,
                        snapshot_json,
                        serde_json::to_string(&node.event_seq_range).unwrap_or_else(|_| "null".to_string()),
                    ],
                )
                .map_err(|e| format!("update history node: {e}"))?;
            }
            PersistCommand::UpdateCursor {
                session_id, cursor, ..
            } => {
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_history_cursor
                     (session_id, visible_node_id, active_branch_id, branch_head_node_id, workspace_node_id,
                      cursor_version, mode, checkout_mode, checkout_status, event_watermark)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        session_id,
                        cursor.visible_node_id,
                        cursor.active_branch_id,
                        cursor.branch_head_node_id,
                        cursor.workspace_node_id,
                        cursor.cursor_version,
                        serde_json::to_string(&cursor.mode).unwrap_or_else(|_| "\"live\"".to_string()),
                        serde_json::to_string(&cursor.checkout_mode).unwrap_or_else(|_| "null".to_string()),
                        serde_json::to_string(&cursor.checkout_status).unwrap_or_else(|_| "null".to_string()),
                        cursor.event_watermark,
                    ],
                )
                .map_err(|e| format!("update cursor: {e}"))?;
            }
            PersistCommand::UpdateBranch {
                session_id, branch, ..
            } => {
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_history_branches
                     (session_id, branch_id, base_node_id, head_node_id, forked_from_branch_id, forked_from_node_id, label, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        session_id,
                        branch.branch_id,
                        branch.base_node_id,
                        branch.head_node_id,
                        branch.forked_from_branch_id,
                        branch.forked_from_node_id,
                        branch.label,
                        branch.created_at_ms as i64,
                        branch.updated_at_ms as i64,
                    ],
                )
                .map_err(|e| format!("update branch: {e}"))?;
            }
            PersistCommand::UpdateSessionMeta {
                session_id,
                meta_patch,
                ..
            } => {
                let (has_title_override, title_override_val) = match &meta_patch.title_override {
                    Some(Some(val)) => (1, Some(val.clone())),
                    Some(None) => (1, None),
                    None => (0, None),
                };
                tx.execute(
                    "UPDATE normalized_sessions
                     SET title = COALESCE(?2, title), summary = COALESCE(?3, summary),
                         turn_count = COALESCE(?4, turn_count),
                         last_referenced_file = COALESCE(?5, last_referenced_file),
                         updated_at_ms = COALESCE(?6, updated_at_ms),
                         title_override = CASE WHEN ?7 = 1 THEN ?8 ELSE title_override END,
                         archived = COALESCE(?9, archived),
                         state_version = state_version + 1
                     WHERE session_id = ?1",
                    params![
                        session_id,
                        meta_patch.title,
                        meta_patch.summary,
                        meta_patch.turn_count.map(|v| v as i64),
                        meta_patch.last_referenced_file,
                        meta_patch.updated_at_ms.map(|v| v as i64),
                        has_title_override,
                        title_override_val,
                        meta_patch.archived.map(|b| if b { 1i64 } else { 0i64 }),
                    ],
                )
                .map_err(|e| format!("update session meta: {e}"))?;
            }
            PersistCommand::RemoveSession { session_id, .. } => {
                tx.execute(
                    "DELETE FROM normalized_sessions WHERE session_id = ?1",
                    params![session_id],
                )
                .map_err(|e| format!("remove normalized session: {e}"))?;
            }
            PersistCommand::FlushEvents {
                session_id,
                turn_id,
                branch_id,
                events,
                ..
            } => {
                self.flush_events_tx(tx, session_id, turn_id, branch_id, events)?;
            }
            PersistCommand::PublishMetadata { key, value, .. } => {
                tx.execute(
                    "INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?1, ?2)",
                    params![key, value],
                )
                .map_err(|e| format!("publish metadata: {e}"))?;
            }
        }
        Ok(())
    }
    /// turn 终态 flush：为事件批次分配连续 seq（0-based，从计数器起）并写入
    /// `turn_events`，计数器与事件行同事务更新。`#[cfg(test)]` 注入点位于
    /// 事件行写入与计数器更新之间，用于验证事务回滚（两表均无残留）。
    fn flush_events_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        turn_id: &str,
        branch_id: &str,
        events: &[crate::agent::turn_event::TurnEvent],
    ) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        let counter_key = format!("turn_event_seq:{session_id}");
        // PA-095 #7：本批外置的 observation 引用（turn_id → ref），flush 内同事务回填。
        let mut observation_refs: Vec<(String, String)> = Vec::new();
        // 计数器缺失（legacy/损坏）时以 MAX(seq)+1 修复（避免 PK 冲突静默丢事件）。
        let next_seq: i64 = tx
            .query_row(
                "SELECT value FROM store_metadata WHERE key = ?1",
                params![counter_key],
                |row| row.get::<_, String>(0),
            )
            .map(|raw| raw.parse::<i64>().unwrap_or(0))
            .unwrap_or_else(|_| {
                tx.query_row(
                    "SELECT COALESCE(MAX(seq), -1) + 1 FROM turn_events WHERE session_id = ?1",
                    params![session_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
            });
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        for (offset, event) in events.iter().enumerate() {
            let seq = next_seq + offset as i64;
            // PA-094：大字段外置——ContextObservation 事件携带全量 payload 时，
            // 写入 build_context_observations 表并改为只含引用的落盘形态。
            let payload = self
                .serialize_event_with_observation_externalization(tx, session_id, seq, event)
                .map_err(|e| format!("flush event serialize: {e}"))?;
            // PA-095 #7：记录本批外置的 observation 引用（同事务回填 trace 行）。
            if let crate::agent::turn_event::TurnEvent::ContextObservation {
                turn_id: observation_turn_id,
                step: _,
                observation: Some(_),
                observation_ref,
            } = event
            {
                let resolved = observation_ref
                    .clone()
                    .unwrap_or_else(|| format!("bco:{observation_turn_id}:{seq}"));
                observation_refs.push((observation_turn_id.clone(), resolved));
            }
            tx.execute(
                "INSERT INTO turn_events
                 (session_id, turn_id, branch_id, seq, event_type, payload, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session_id,
                    turn_id,
                    branch_id,
                    seq,
                    event.type_name(),
                    payload,
                    now
                ],
            )
            .map_err(|e| format!("flush event insert: {e}"))?;
        }
        #[cfg(test)]
        if let Some(suffix) = TEST_INJECT_FLUSH_FAILURE_DB_SUFFIX
            .lock()
            .expect("inject lock poisoned")
            .as_ref()
        {
            if self.db_path.to_string_lossy().contains(suffix.as_str()) {
                return Err("test-injected flush failure after event rows".to_string());
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?1, ?2)",
            params![counter_key, (next_seq + events.len() as i64).to_string()],
        )
        .map_err(|e| format!("flush event counter: {e}"))?;
        // PA-095 #7：同事务回填——外置 payload 的引用写回 trace 缓存行
        // （session_turn_traces.trace_data 与 normalized_turn_traces.raw_json），
        // trace 行不再内嵌 observation 副本（引用加载路径 PA-094 已有）。
        for (observation_turn_id, reference) in &observation_refs {
            self.backfill_trace_observation_ref_tx(
                tx,
                session_id,
                observation_turn_id,
                reference,
            );
        }
        Ok(())
    }

    /// PA-095 #7：把外置 observation 引用回填到 trace 缓存行 JSON
    /// （buildContextObservation 置空 + buildContextObservationRef 写入）。
    /// 两张表任一命中即算成功；两表均无该 turn 行时记日志跳过（live 行尚未落库）。
    fn backfill_trace_observation_ref_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        turn_id: &str,
        reference: &str,
    ) {
        let mut patched_any = false;
        if let Some((data,)) = tx
            .query_row(
                "SELECT trace_data FROM session_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
                params![session_id, turn_id],
                |row| row.get::<_, String>(0).map(|data| (data,)),
            )
            .ok()
        {
            if let Some(patched) = patch_trace_json_observation_ref(&data, reference) {
                if tx
                    .execute(
                        "UPDATE session_turn_traces SET trace_data = ?3
                         WHERE session_id = ?1 AND turn_id = ?2",
                        params![session_id, turn_id, patched],
                    )
                    .is_ok()
                {
                    patched_any = true;
                }
            }
        }
        if let Some((raw,)) = tx
            .query_row(
                "SELECT raw_json FROM normalized_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
                params![session_id, turn_id],
                |row| row.get::<_, String>(0).map(|raw| (raw,)),
            )
            .ok()
        {
            if let Some(patched) = patch_trace_json_observation_ref(&raw, reference) {
                if tx
                    .execute(
                        "UPDATE normalized_turn_traces SET raw_json = ?3
                         WHERE session_id = ?1 AND turn_id = ?2",
                        params![session_id, turn_id, patched],
                    )
                    .is_ok()
                {
                    patched_any = true;
                }
            }
        }
        if !patched_any {
            eprintln!(
                "[pony-agent][session] observation ref backfill skipped: no trace row session={session_id} turn={turn_id}"
            );
        }
    }

    /// PA-094：大字段外置——`ContextObservation` 事件序列化。
    /// 事件内存形态携带全量 payload（`observation`）时，将 payload 写入
    /// `build_context_observations` 表（ref = `bco:<turn_id>:<seq>`，seq 为事件日志
    /// seq），并返回只含引用的落盘形态 JSON；否则原样序列化。
    /// 与事件行同事务（外置失败 → 事件行回滚，无孤儿引用）。
    fn serialize_event_with_observation_externalization(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        seq: i64,
        event: &crate::agent::turn_event::TurnEvent,
    ) -> Result<String, String> {
        if let crate::agent::turn_event::TurnEvent::ContextObservation {
            turn_id,
            step,
            observation,
            observation_ref,
        } = event
        {
            if let Some(payload) = observation {
                let resolved_ref = observation_ref
                    .clone()
                    .unwrap_or_else(|| format!("bco:{turn_id}:{seq}"));
                let payload_json = serde_json::to_string(payload)
                    .map_err(|e| format!("serialize context observation: {e}"))?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                tx.execute(
                    "INSERT OR REPLACE INTO build_context_observations
                     (session_id, observation_ref, payload_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![session_id, resolved_ref, payload_json, now],
                )
                .map_err(|e| format!("store context observation: {e}"))?;
                let ref_event = crate::agent::turn_event::TurnEvent::ContextObservation {
                    turn_id: turn_id.clone(),
                    step: *step,
                    observation: None,
                    observation_ref: Some(resolved_ref),
                };
                return serde_json::to_string(&ref_event)
                    .map_err(|e| format!("serialize context observation ref: {e}"));
            }
        }
        serde_json::to_string(event).map_err(|e| format!("serialize event: {e}"))
    }

    /// PA-094：按引用加载 build_context_observation 全量 payload（host command 后端）。
    /// 引用格式 `bco:<turn_id>:<seq>`；未命中返回 None（legacy 内嵌数据走 trace 字段）。
    pub fn load_build_context_observation(
        &self,
        session_id: &str,
        observation_ref: &str,
    ) -> Option<crate::agent::provider::BuildContextObservation> {
        let slot = self.connection().ok()?;
        let conn = slot.as_ref()?;
        let payload: String = conn
            .query_row(
                "SELECT payload_json FROM build_context_observations
                 WHERE session_id = ?1 AND observation_ref = ?2",
                params![session_id, observation_ref],
                |row| row.get(0),
            )
            .ok()?;
        serde_json::from_str(&payload).ok()
    }

    /// PA-091：迁移回填——从 blob 反推事件（per-session 幂等）。
    /// 数据损失承认：blob 被 `DEFAULT_HISTORY_LIMIT` 截断，只回填最近 24 turn；
    /// chunk 过程不可恢复 → `chunk_missing` 标记。返回回填的会话数。
    pub fn backfill_turn_events(&self) -> Result<usize, String> {
        let slot = self.connection().map_err(|e| format!("open: {e}"))?;
        let conn = slot.as_ref().expect("connection initialized");
        let session_ids: Vec<String> = conn
            .prepare("SELECT conversation_id FROM sessions")
            .map_err(|e| format!("list sessions: {e}"))?
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("query sessions: {e}"))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("collect sessions: {e}"))?;
        let mut backfilled = 0usize;
        for session_id in session_ids {
            let marker_key = format!("turn_event_backfill:{session_id}");
            let done: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM store_metadata WHERE key = ?1)",
                    params![marker_key],
                    |row| row.get::<_, i64>(0),
                )
                .map(|v| v != 0)
                .unwrap_or(false);
            if done {
                continue;
            }
            let data: String = conn
                .query_row(
                    "SELECT session_data FROM sessions WHERE conversation_id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("read blob {session_id}: {e}"))?;
            let session: crate::agent::session::SessionState =
                serde_json::from_str(&data).map_err(|e| format!("parse blob {session_id}: {e}"))?;
            let events = derive_events_from_session(&session);
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            if !events.is_empty() {
                self.flush_events_tx(&tx, &session_id, "backfill", "main", &events)?;
            }
            tx.execute(
                "INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?1, 'done')",
                params![marker_key],
            )
            .map_err(|e| format!("backfill marker: {e}"))?;
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            backfilled += 1;
        }
        Ok(backfilled)
    }
    fn sync_blob_trace_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        trace: &TurnTraceRecord,
    ) -> Result<(), String> {
        // 剥离语义：blob 不存 trace（WriteSeparate + Authoritative），只维护 updated_at_ms。
        // 仅检查 blob 行是否存在（不需要读取 session_data 内容）。
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE conversation_id = ?1)",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .unwrap_or(false);
        if !exists {
            // blob 行不存在（新会话未全量保存过）→ 创建最小行（剥离语义：blob 不存 trace）
            let session = SessionState {
                conversation_id: session_id.to_string(),
                title: String::new(),
                summary: String::new(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                turn_trace_history: Vec::new(),
                trace_migration_state: TraceMigrationState::TraceTableAuthoritative,
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
                updated_at_ms: trace.updated_at,
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                workspace_id: None,
                title_override: None,
                archived: false,
                event_watermark: 0,
                last_commit_watermark: 0,
            };
            let initial =
                serde_json::to_string(&session).map_err(|e| format!("serialize blob: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
                 VALUES (?1, '', ?2, ?3)",
                params![session_id, trace.updated_at as i64, initial],
            )
            .map_err(|e| format!("insert blob: {e}"))?;
            return Ok(());
        }
        // 剥离语义：blob 不存 trace（WriteSeparate + Authoritative），只更新 updated_at_ms。
        // trace 数据在 session_turn_traces / normalized_turn_traces 表（load_store 表优先）。
        tx.execute(
            "UPDATE sessions SET updated_at_ms = ?2 WHERE conversation_id = ?1",
            params![session_id, trace.updated_at as i64],
        )
        .map_err(|e| format!("update blob ts: {e}"))?;
        Ok(())
    }
    fn load_store_normalized(&self, conn: &Connection) -> Option<PersistedStore> {
        let session_rows = conn
            .prepare("SELECT session_id, title, summary, turn_count, last_referenced_file, created_at_ms, updated_at_ms, state_version, trace_migration_state, turn_trace_refs_json, provider_native_transcript_json, history_state_evidence_json, memory_json, workspace_id, title_override, archived FROM normalized_sessions")
            .ok()?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<i64>>(15)?.unwrap_or(0),
                    ),
                ))
            })
            .ok()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();

        // PA-089 优化：批量预取全部子表（一次查询替代每会话 5 次查询 = N+1），
        // 消除启动时 load_store_normalized 的 1.5s 卡顿。
        let all_messages = conn
            .prepare("SELECT session_id, message_id, turn_id, ordinal, role, content, reasoning_content, status, model_name, token_count, attachments_json FROM normalized_messages ORDER BY session_id, ordinal")
            .ok()?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })
            .ok()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let all_traces = self.read_all_session_traces(conn).unwrap_or_default();
        let all_nodes = conn
            .prepare("SELECT session_id, node_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id, turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms, snapshot_json, event_seq_range_json FROM normalized_history_nodes ORDER BY session_id")
            .ok()?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                ))
            })
            .ok()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let all_branches = conn
            .prepare("SELECT session_id, branch_id, base_node_id, head_node_id, forked_from_branch_id, forked_from_node_id, label, created_at_ms, updated_at_ms FROM normalized_history_branches ORDER BY session_id")
            .ok()?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .ok()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let all_cursors = conn
            .prepare("SELECT session_id, visible_node_id, active_branch_id, branch_head_node_id, workspace_node_id, cursor_version, mode, checkout_mode, checkout_status, event_watermark FROM normalized_history_cursor ORDER BY session_id")
            .ok()?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            })
            .ok()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();

        let mut sessions: HashMap<String, SessionState> = HashMap::new();
        for (
            session_id,
            (
                title,
                summary,
                turn_count,
                last_referenced_file,
                _created_at,
                updated_at_ms,
                state_version,
                trace_migration_state,
                turn_trace_refs_json,
                provider_native_transcript_json,
                history_state_evidence_json,
                memory_json,
                workspace_id,
                title_override,
                archived,
            ),
        ) in session_rows
        {
            let mut session = SessionState {
                conversation_id: session_id.clone(),
                title,
                summary,
                history: Vec::new(),
                provider_native_transcript: provider_native_transcript_json
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default(),
                turn_trace_history: Vec::new(),
                trace_migration_state: serde_json::from_str(&trace_migration_state)
                    .unwrap_or(TraceMigrationState::LegacyBlob),
                turn_trace_refs: turn_trace_refs_json
                    .and_then(|raw| serde_json::from_str(&raw).ok()),
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: history_state_evidence_json
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default(),
                turn_count: turn_count as usize,
                last_referenced_file,
                updated_at_ms: updated_at_ms as u64,
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                event_watermark: 0,
                last_commit_watermark: 0,
                workspace_id,
                title_override,
                archived: archived != 0,
            };
            // 记忆四件套（memory_json）
            if let Some(raw) = memory_json {
                if let Ok(mem) = serde_json::from_str::<serde_json::Value>(&raw) {
                    session.long_term_memory_entries = mem
                        .get("longTermMemoryEntries")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    session.memory_write_evidence = mem
                        .get("memoryWriteEvidence")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    session.memory_write_hook_trace_records = mem
                        .get("memoryWriteHookTraceRecords")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                }
            }
            // state_version 记录（分页 revision 用）
            if state_version > 0 {
                // 目前仅记录日志，revision 逻辑归阶段 5 前端分页
            }

            // messages → history（批量预取，消除 N+1）
            let messages = all_messages
                .iter()
                .filter(|(sid, _, _, _, _, _, _, _, _)| sid == &session_id)
                .map(
                    |(
                        _,
                        turn_id,
                        role,
                        content,
                        reasoning_content,
                        status,
                        model_name,
                        token_count,
                        attachments_json,
                    )| {
                        (
                            turn_id.clone(),
                            role.clone(),
                            content.clone(),
                            reasoning_content.clone(),
                            status.clone(),
                            model_name.clone(),
                            token_count.clone(),
                            attachments_json.clone(),
                        )
                    },
                )
                .collect::<Vec<_>>();
            for (
                turn_id,
                role,
                content,
                reasoning_content,
                status,
                model_name,
                token_count,
                attachments_json,
            ) in messages
            {
                session.history.push(TurnHistoryMessage {
                    role,
                    content,
                    attachments: attachments_json
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or_default(),
                    turn_id,
                    status: status.and_then(|s| serde_json::from_str(&s).ok()),
                    model_name,
                    token_count: token_count.map(|v| v as u64),
                    reasoning_content,
                });
            }

            // traces → turn_trace_history（批量预取）
            let table_traces = all_traces.get(&session_id).cloned().unwrap_or_default();
            session.turn_trace_history = table_traces.clone();

            // history_nodes（批量预取）
            let node_rows = all_nodes
                .iter()
                .filter(|(sid, _, _, _, _, _, _, _, _, _, _, _, _, _, _)| sid == &session_id)
                .map(
                    |(
                        _,
                        node_id,
                        parent_node_id,
                        branch_id,
                        forked_from_node_id,
                        kind,
                        turn_id,
                        turn_trace_refs_json,
                        run_id,
                        workspace_ref_json,
                        summary,
                        title,
                        created_at_ms,
                        snapshot_json,
                        event_seq_range_json,
                    )| {
                        (
                            node_id.clone(),
                            parent_node_id.clone(),
                            branch_id.clone(),
                            forked_from_node_id.clone(),
                            kind.clone(),
                            turn_id.clone(),
                            turn_trace_refs_json.clone(),
                            run_id.clone(),
                            workspace_ref_json.clone(),
                            summary.clone(),
                            title.clone(),
                            *created_at_ms,
                            snapshot_json.clone(),
                            event_seq_range_json.clone(),
                        )
                    },
                )
                .collect::<Vec<_>>();
            let full_table_by_id: HashMap<&str, &TurnTraceRecord> = table_traces
                .iter()
                .map(|trace| (trace.turn_id.as_str(), trace))
                .collect();
            for (
                node_id,
                parent_node_id,
                branch_id,
                forked_from_node_id,
                kind,
                turn_id,
                turn_trace_refs_json,
                run_id,
                workspace_ref_json,
                summary,
                title,
                created_at_ms,
                snapshot_json,
                event_seq_range_json,
            ) in node_rows
            {
                let mut node = HistoryNode {
                    node_id,
                    session_id: session_id.clone(),
                    parent_node_id,
                    branch_id,
                    forked_from_node_id,
                    kind: serde_json::from_str(&kind).unwrap_or(HistoryNodeKind::Checkpoint),
                    run_id,
                    workspace_ref: workspace_ref_json
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or_default(),
                    summary,
                    title,
                    history: Vec::new(),
                    provider_native_transcript: Vec::new(),
                    turn_trace_history: Vec::new(),
                    turn_id,
                    turn_trace_refs: turn_trace_refs_json
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                    long_term_memory_entries: Vec::new(),
                    memory_write_evidence: Vec::new(),
                    memory_write_hook_trace_records: Vec::new(),
                    turn_count: 0,
                    last_referenced_file: None,
                    created_at_ms: created_at_ms as u64,
                    event_seq_range: event_seq_range_json
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                };
                // snapshot_json → 节点快照
                if let Some(raw) = snapshot_json {
                    if let Ok(snap) = serde_json::from_str::<serde_json::Value>(&raw) {
                        node.history = snap
                            .get("history")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        node.provider_native_transcript = snap
                            .get("providerNativeTranscript")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        node.long_term_memory_entries = snap
                            .get("longTermMemoryEntries")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        node.memory_write_evidence = snap
                            .get("memoryWriteEvidence")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        node.memory_write_hook_trace_records = snap
                            .get("memoryWriteHookTraceRecords")
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                            .unwrap_or_default();
                        node.turn_count =
                            snap.get("turnCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        node.last_referenced_file = snap
                            .get("lastReferencedFile")
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                    }
                }
                // 节点 trace materialize（按 refs 从全量表）
                if let Some(refs) = &node.turn_trace_refs {
                    if refs.is_empty() {
                        node.turn_trace_history.clear();
                    } else {
                        node.turn_trace_history = refs
                            .iter()
                            .filter_map(|reference| {
                                full_table_by_id
                                    .get(reference.turn_id.as_str())
                                    .map(|t| (*t).clone())
                            })
                            .collect();
                    }
                }
                session.history_nodes.push(node);
            }

            // history_branches（批量预取）
            session.history_branches = all_branches
                .iter()
                .filter(|(sid, _, _, _, _, _, _, _, _)| sid == &session_id)
                .map(
                    |(
                        _,
                        branch_id,
                        base_node_id,
                        head_node_id,
                        forked_from_branch_id,
                        forked_from_node_id,
                        label,
                        created_at_ms,
                        updated_at_ms,
                    )| HistoryBranch {
                        branch_id: branch_id.clone(),
                        session_id: session_id.clone(),
                        base_node_id: base_node_id.clone(),
                        head_node_id: head_node_id.clone(),
                        forked_from_branch_id: forked_from_branch_id.clone(),
                        forked_from_node_id: forked_from_node_id.clone(),
                        label: label.clone(),
                        created_at_ms: *created_at_ms as u64,
                        updated_at_ms: *updated_at_ms as u64,
                    },
                )
                .collect::<Vec<_>>();

            // history_cursor（批量预取）
            if let Some((
                _,
                visible_node_id,
                active_branch_id,
                branch_head_node_id,
                workspace_node_id,
                cursor_version,
                mode,
                checkout_mode,
                checkout_status,
                event_watermark,
            )) = all_cursors
                .iter()
                .find(|(sid, _, _, _, _, _, _, _, _, _)| sid == &session_id)
            {
                session.history_cursor = HistoryCursor {
                    session_id: session_id.clone(),
                    visible_node_id: visible_node_id.clone(),
                    active_branch_id: active_branch_id.clone(),
                    branch_head_node_id: branch_head_node_id.clone(),
                    workspace_node_id: workspace_node_id.clone(),
                    cursor_version: *cursor_version as u64,
                    event_watermark: *event_watermark as u64,
                    mode: mode
                        .clone()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or(HistoryCursorMode::Live),
                    checkout_mode: checkout_mode
                        .clone()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or(HistoryCheckoutMode::TranscriptOnly),
                    checkout_status: checkout_status
                        .clone()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or(HistoryCheckoutStatus::NotRequested),
                };
                session.event_watermark = session.history_cursor.event_watermark;
                session.last_commit_watermark = session.history_cursor.event_watermark;

                if let Some(visible_id) = &session.history_cursor.visible_node_id {
                    if let Some(node) = session.history_nodes.iter().find(|n| &n.node_id == visible_id) {
                        if !node.history.is_empty() {
                            session.history = node.history.clone();
                            session.long_term_memory_entries = node.long_term_memory_entries.clone();
                            if node.turn_count > 0 {
                                session.turn_count = node.turn_count;
                            }
                            if node.last_referenced_file.is_some() {
                                session.last_referenced_file = node.last_referenced_file.clone();
                            }
                        }
                    }
                }
            }

            sessions.insert(session_id, session);
        }

        let attachment_assets = self.read_metadata(conn, "attachment_assets");
        let session_attachment_index = self.read_metadata(conn, "session_attachment_index");
        let mcp_source_snapshots = self.read_metadata(conn, "mcp_source_snapshots");
        let skill_source_snapshots = self.read_metadata(conn, "skill_source_snapshots");
        let workspaces = self.read_metadata(conn, "workspaces");
        let path_authorizations = self.read_metadata(conn, "path_authorizations.v1");

        Some(PersistedStore {
            sessions,
            attachment_assets,
            session_attachment_index,
            mcp_source_snapshots,
            skill_source_snapshots,
            workspaces,
            path_authorizations,
        })
    }
    fn sync_legacy_trace_update_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        turn_id: &str,
        patch: &serde_json::Value,
        updated_at: u64,
    ) -> Result<(), String> {
        let raw: Option<String> = tx
            .query_row(
                "SELECT trace_data FROM session_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
                params![session_id, turn_id],
                |row| row.get::<_, String>(0),
            )
            .ok();
        let Some(raw) = raw else {
            // 行缺失（被删/未写）→ 从 normalized_turn_traces.raw_json 重建修复
            let normalized_raw: Option<String> = tx
                .query_row(
                    "SELECT raw_json FROM normalized_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
                    params![session_id, turn_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            let Some(normalized_raw) = normalized_raw else {
                return Ok(());
            };
            let mut trace: serde_json::Value = serde_json::from_str(&normalized_raw)
                .map_err(|e| format!("parse normalized trace: {e}"))?;
            if let serde_json::Value::Object(map) = &mut trace {
                if let serde_json::Value::Object(patch_map) = patch {
                    for (key, value) in patch_map {
                        if !value.is_null() {
                            map.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
            trace["updatedAt"] = serde_json::json!(updated_at);
            let updated =
                serde_json::to_string(&trace).map_err(|e| format!("serialize trace: {e}"))?;
            let order: i64 = tx
                .query_row(
                    "SELECT trace_order FROM normalized_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
                    params![session_id, turn_id],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            tx.execute(
                "INSERT OR REPLACE INTO session_turn_traces (session_id, turn_id, updated_at_ms, trace_order, trace_data)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, turn_id, updated_at as i64, order, updated],
            )
            .map_err(|e| format!("repair legacy trace: {e}"))?;
            return Ok(());
        };
        let mut trace: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse legacy trace: {e}"))?;
        if let serde_json::Value::Object(map) = &mut trace {
            if let serde_json::Value::Object(patch_map) = patch {
                for (key, value) in patch_map {
                    if !value.is_null() {
                        map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        trace["updatedAt"] = serde_json::json!(updated_at);
        let updated =
            serde_json::to_string(&trace).map_err(|e| format!("serialize legacy trace: {e}"))?;
        tx.execute(
            "UPDATE session_turn_traces SET trace_data = ?3, updated_at_ms = ?4 WHERE session_id = ?1 AND turn_id = ?2",
            params![session_id, turn_id, updated, updated_at as i64],
        )
        .map_err(|e| format!("update legacy trace: {e}"))?;
        Ok(())
    }

    /// PA-095：事件 schema 版本校验（四分支：匹配 / 缺失 key 视为 v1 并回填 /
    /// 不匹配 Err / 读取异常 Err）。缺失 key 的回填保证存量库首次访问后补齐契约。
    fn validate_event_schema_conn(&self, conn: &rusqlite::Connection) -> Result<u64, String> {
        let recorded = conn
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'events.schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok();
        match recorded {
            Some(raw) => {
                let version: u64 = raw
                    .parse()
                    .map_err(|_| format!("events.schema_version unparsable: {raw}"))?;
                if version == crate::agent::turn_event::EVENT_SCHEMA_VERSION {
                    Ok(version)
                } else {
                    Err(format!(
                        "event schema version mismatch: store={} current={}",
                        version,
                        crate::agent::turn_event::EVENT_SCHEMA_VERSION
                    ))
                }
            }
            None => {
                // 缺失 key（本 change 之前的存量库）：视为 v1 并回填。
                conn.execute(
                    "INSERT OR REPLACE INTO store_metadata (key, value) VALUES ('events.schema_version', ?1)",
                    params![crate::agent::turn_event::EVENT_SCHEMA_VERSION.to_string()],
                )
                .map_err(|e| format!("schema version backfill: {e}"))?;
                Ok(crate::agent::turn_event::EVENT_SCHEMA_VERSION)
            }
        }
    }

    /// PA-095：带契约校验的事件加载——版本不匹配或 payload 解析失败即 fail loud
    /// （Err 上抛，调用方标记会话 degraded），不再静默跳过坏行。
    fn load_turn_events_checked_impl(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Result<Vec<(u64, String, crate::agent::turn_event::TurnEvent)>, String> {
        let slot = self.connection().map_err(|e| format!("sqlite open: {e}"))?;
        let conn = slot.as_ref().expect("connection initialized");
        self.validate_event_schema_conn(conn)?;
        let sql = match up_to_seq {
            Some(upper) => format!(
                "SELECT seq, branch_id, payload FROM turn_events
                 WHERE session_id = ?1 AND seq <= {upper} ORDER BY seq"
            ),
            None => "SELECT seq, branch_id, payload FROM turn_events
                 WHERE session_id = ?1 ORDER BY seq"
                .to_string(),
        };
        let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("query: {e}"))?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, branch_id, payload) = row.map_err(|e| format!("row: {e}"))?;
            let event = match serde_json::from_str::<crate::agent::turn_event::TurnEvent>(&payload)
            {
                Ok(event) => event,
                Err(error) => {
                    // PA-095：ignorable 清单内的未知类型跳过（清单当前为空，
                    // 全部 fail loud）；清单外 fail loud 并标记会话 degraded。
                    let event_type = serde_json::from_str::<serde_json::Value>(&payload)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("type")
                                .and_then(|tag| tag.as_str())
                                .map(str::to_string)
                        });
                    let ignorable = event_type
                        .as_deref()
                        .is_some_and(|tag| {
                            crate::agent::turn_event::IGNORABLE_EVENT_TYPES.contains(&tag)
                        });
                    if ignorable {
                        continue;
                    }
                    return Err(format!(
                        "event stream degraded: session={session_id} seq={seq} parse error: {error}"
                    ));
                }
            };
            events.push((seq, branch_id, event));
        }
        Ok(events)
    }
}

impl SessionBackend for SqliteSessionBackend {
    /// PA-095：事件 schema 版本校验（trait 入口）。
    fn validate_event_schema(&self) -> Result<u64, String> {
        let slot = self.connection().map_err(|e| format!("sqlite open: {e}"))?;
        let conn = slot.as_ref().expect("connection initialized");
        self.validate_event_schema_conn(conn)
    }

    /// PA-095：带契约校验的事件加载（trait 入口，fail loud）。
    fn load_turn_events_checked(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Result<Vec<(u64, String, crate::agent::turn_event::TurnEvent)>, String> {
        self.load_turn_events_checked_impl(session_id, up_to_seq)
    }

    /// PA-095 #2：SQLite 后端具备事件表能力（append_turn 物化不回退）。
    fn supports_turn_events(&self) -> bool {
        true
    }

    /// PA-095 #7：列出会话已外置的 observation 引用（flush 后附加到内存
    /// trace 缓存行，防异步 worker REPLACE 覆盖回填）。查询失败 contained 空。
    fn load_observation_refs(&self, session_id: &str) -> Vec<String> {
        let Some(slot) = self.connection().ok() else {
            return Vec::new();
        };
        let Some(conn) = slot.as_ref() else {
            return Vec::new();
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT observation_ref FROM build_context_observations WHERE session_id = ?1",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))
        else {
            return Vec::new();
        };
        rows.filter_map(|row| row.ok()).collect()
    }

    /// PA-089 阶段 3：规范化双写命令——统一事务写 blob（旧 sessions 表）+ normalized_* 表。
    /// 迁移 barrier：epoch 检查（旧 epoch 命令拒绝）。
    fn persist_command(&self, command: PersistCommand) -> PersistCommandOutcome {
        self.persist_commands_batch(vec![command])
    }

    /// 批量规范化持久化命令（单个 SQLite BEGIN EXCLUSIVE 事务原子提交，附带 busy 退避重试）。
    fn persist_commands_batch(&self, commands: Vec<PersistCommand>) -> PersistCommandOutcome {
        if commands.is_empty() {
            return PersistCommandOutcome::Succeeded;
        }
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return PersistCommandOutcome::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");

        let mut retries = 3;
        loop {
            let tx = match conn.unchecked_transaction() {
                Ok(tx) => tx,
                Err(error) => {
                    if retries > 0 && error.to_string().contains("busy") {
                        retries -= 1;
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        continue;
                    }
                    eprintln!("[pony-agent][session] SQLite batch command begin tx error: {error}");
                    return PersistCommandOutcome::Failed;
                }
            };

            let mut all_ok = true;
            for command in &commands {
                if let Err(error) = self.apply_persist_command_tx(&tx, command) {
                    eprintln!("[pony-agent][session] SQLite batch command error: {error}");
                    all_ok = false;
                    break;
                }
            }

            if !all_ok {
                return PersistCommandOutcome::Failed;
            }

            match tx.commit() {
                Ok(()) => return PersistCommandOutcome::Succeeded,
                Err(error) => {
                    if retries > 0 && error.to_string().contains("busy") {
                        retries -= 1;
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        continue;
                    }
                    eprintln!("[pony-agent][session] SQLite batch command commit error: {error}");
                    return PersistCommandOutcome::Failed;
                }
            }
        }
    }

    /// PA-089 阶段 3：双写——同步旧 session_turn_traces 表（load_store 读它）。
    /// 读 trace_data JSON → 合并 patch → 写回。

    /// PA-089 阶段 3：双写——同步更新旧 sessions 表 blob 的 turn_trace_history。
    /// 读 blob → upsert trace（按 turn_id）→ 写回。与规范化表写入同一事务。
    fn load_store(&self) -> Option<PersistedStore> {
        eprintln!(
            "[pony-agent][session] loading sessions from SQLite {}",
            self.db_path.display()
        );

        // Initialize (and migrate) the pooled connection, then borrow it.
        let mut slot = self.connection().ok()?;
        let conn = slot.as_mut().expect("connection initialized");

        // PA-089 阶段 5：切读——若迁移 phase 为 observing/retired，从规范化表重建。
        let phase: String = conn
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'storage.normalized.v1.phase'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|_| "legacy".to_string());
        if phase == "observing" || phase == "retired" {
            return self.load_store_normalized(conn);
        }

        // Read all sessions
        let mut stmt = conn
            .prepare("SELECT conversation_id, session_data FROM sessions")
            .ok()?;

        let sessions: HashMap<String, SessionState> = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let data: String = row.get(1)?;
                Ok((id, data))
            })
            .ok()?
            .filter_map(|result| {
                let (id, data) = result.ok()?;
                let mut session: SessionState = serde_json::from_str(&data).ok()?;
                if session.conversation_id != id {
                    session.conversation_id = id.clone();
                }
                let table_traces = self.read_session_traces(conn, &id).ok()?;
                session.turn_trace_history = self.merge_trace_history(&session, table_traces.clone());
                // PA-088/PA-090：节点 materialize——按 refs 从**全量表**恢复节点 trace。
                // 注意：by_id 必须从 table_traces（全量 Union）构建，而非顶层过滤后的
                // session.turn_trace_history——否则 fork 分支/超 24 轮节点的 refs 会
                // 解析不到（顶层只显示当前分支），且后续 union 重写会永久删除这些 trace。
                // Some([]) = authoritative 且确实无 trace（清空不兜底）；
                // Some(v) = 按 refs 查表组装（缺失的 turn 降级为空并告警）；
                // None = legacy 旧数据（反序列化已带内嵌 trace，保持）。
                let full_table_by_id: HashMap<&str, &TurnTraceRecord> = table_traces
                    .iter()
                    .map(|trace| (trace.turn_id.as_str(), trace))
                    .collect();
                for node in &mut session.history_nodes {
                    if let Some(refs) = &node.turn_trace_refs {
                        if refs.is_empty() {
                            node.turn_trace_history.clear();
                        } else {
                            let mut missing = 0usize;
                            node.turn_trace_history = refs
                                .iter()
                                .filter_map(|reference| {
                                    match full_table_by_id.get(reference.turn_id.as_str()) {
                                        Some(trace) => Some((*trace).clone()),
                                        None => {
                                            missing += 1;
                                            None
                                        }
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
                }
                Some((id, session))
            })
            .collect();

        let attachment_assets = self.read_metadata(conn, "attachment_assets");
        let session_attachment_index = self.read_metadata(conn, "session_attachment_index");
        let mcp_source_snapshots = self.read_metadata(conn, "mcp_source_snapshots");
        let skill_source_snapshots = self.read_metadata(conn, "skill_source_snapshots");
        let workspaces = self.read_metadata(conn, "workspaces");
        let path_authorizations = self.read_metadata(conn, "path_authorizations.v1");

        Some(PersistedStore {
            sessions,
            attachment_assets,
            session_attachment_index,
            mcp_source_snapshots,
            skill_source_snapshots,
            workspaces,
            path_authorizations,
        })
    }

    /// PA-089 阶段 5：规范化 loader——从 normalized_* 表重建 SessionState（切读）。
    /// 消息 → history；trace 表 → turn_trace_history + 节点 materialize；
    /// history_nodes/branches/cursor 直接读表。

    fn save_store(&self, store: &PersistedStore) {
        // Borrow the pooled connection; never panic the caller on a write error
        // — the turn loop must stay alive even if persistence hiccups.
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(e) => {
                eprintln!("[pony-agent][session] SQLite open error: {e}");
                return;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");

        if let Err(e) = self.write_full_store(conn, store) {
            eprintln!("[pony-agent][session] SQLite save error: {e}");
        }

        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);") {
            eprintln!("[pony-agent][session] SQLite checkpoint error: {e}");
        }
    }

    fn trace_storage_mode(&self) -> SeparateTraceTableMode {
        self.trace_mode
    }

    fn upsert_session(&self, session_id: &str, session: &SessionState) -> bool {
        if session.conversation_id != session_id {
            eprintln!(
                "[pony-agent][session] SQLite upsert session mismatch: key={} payload={}",
                session_id, session.conversation_id
            );
            return false;
        }
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(e) => {
                eprintln!("[pony-agent][session] SQLite open error: {e}");
                return false;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let data = match serde_json::to_string(session) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("[pony-agent][session] SQLite session serialize error: {e}");
                return false;
            }
        };
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("[pony-agent][session] SQLite upsert begin tx error: {e}");
                return false;
            }
        };
        if let Err(e) = tx.execute(
            "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session_id,
                session.title,
                session.updated_at_ms as i64,
                data
            ],
        ) {
            eprintln!("[pony-agent][session] SQLite upsert session error: {e}");
            return false;
        }
        // Always write traces to keep session blob and trace table in sync.
        // PA-088：authoritative 会话跳过——blob 已被剥离 trace，若用空 trace 替换
        // 表会清空数据（表由 save_to_backend 在剥离前写入）。
        if session.trace_migration_state != TraceMigrationState::TraceTableAuthoritative {
            if let Err(e) =
                self.replace_session_traces_tx(&tx, session_id, &session.turn_trace_history)
            {
                eprintln!("[pony-agent][session] SQLite replace traces error: {e}");
                return false;
            }
        }
        if let Err(e) = tx.commit() {
            eprintln!("[pony-agent][session] SQLite upsert commit error: {e}");
            return false;
        }
        true
    }

    fn remove_session(
        &self,
        session_id: &str,
        attachment_assets: &HashMap<String, AttachmentAsset>,
        session_attachment_index: &HashMap<String, Vec<String>>,
        mcp_source_snapshots: &HashMap<String, McpSourceSnapshot>,
        skill_source_snapshots: &HashMap<String, SkillSourceSnapshot>,
    ) -> bool {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return false;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite delete begin tx error: {error}");
                return false;
            }
        };

        if let Err(error) = tx.execute(
            "DELETE FROM sessions WHERE conversation_id = ?1",
            params![session_id],
        ) {
            eprintln!("[pony-agent][session] SQLite delete session error: {error}");
            return false;
        }
        if let Err(error) = tx.execute(
            "DELETE FROM session_turn_traces WHERE session_id = ?1",
            params![session_id],
        ) {
            eprintln!("[pony-agent][session] SQLite delete session traces error: {error}");
            return false;
        }

        {
            let metadata_entries: [(&str, Option<String>); 4] = [
                (
                    "attachment_assets",
                    serde_json::to_string(attachment_assets).ok(),
                ),
                (
                    "session_attachment_index",
                    serde_json::to_string(session_attachment_index).ok(),
                ),
                (
                    "mcp_source_snapshots",
                    serde_json::to_string(mcp_source_snapshots).ok(),
                ),
                (
                    "skill_source_snapshots",
                    serde_json::to_string(skill_source_snapshots).ok(),
                ),
            ];
            let mut meta_stmt = match tx
                .prepare("INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?1, ?2)")
            {
                Ok(stmt) => stmt,
                Err(error) => {
                    eprintln!("[pony-agent][session] SQLite prepare metadata delete-upsert error: {error}");
                    return false;
                }
            };
            for (key, value) in &metadata_entries {
                if let Some(val) = value {
                    if let Err(error) = meta_stmt.execute(params![key, val]) {
                        eprintln!(
                            "[pony-agent][session] SQLite metadata delete-upsert error: {error}"
                        );
                        return false;
                    }
                }
            }
        }

        if let Err(error) = tx.commit() {
            eprintln!("[pony-agent][session] SQLite delete commit error: {error}");
            return false;
        }
        true
    }

    fn load_session_traces(&self, session_id: &str) -> SessionBackendTraceLoadResult {
        let mut slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return SessionBackendTraceLoadResult::Loaded(Vec::new());
            }
        };
        let conn = slot.as_mut().expect("connection initialized");
        match self.read_session_traces(conn, session_id) {
            Ok(traces) => SessionBackendTraceLoadResult::Loaded(traces),
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite load session traces error: {error}");
                SessionBackendTraceLoadResult::Loaded(Vec::new())
            }
        }
    }

    /// PA-093：读取会话事件流 `(seq, branch_id, event)`（seq 升序，可截断）。
    fn load_turn_events(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Vec<(u64, String, crate::agent::turn_event::TurnEvent)> {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return Vec::new();
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let sql = match up_to_seq {
            Some(upper) => format!(
                "SELECT seq, branch_id, payload FROM turn_events
                 WHERE session_id = ?1 AND seq <= {upper} ORDER BY seq"
            ),
            None => "SELECT seq, branch_id, payload FROM turn_events
                 WHERE session_id = ?1 ORDER BY seq"
                .to_string(),
        };
        let mut events = Vec::new();
        let mut stmt = match conn.prepare(&sql) {
            Ok(stmt) => stmt,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite load turn events prepare error: {error}");
                return Vec::new();
            }
        };
        let rows = match stmt.query_map(params![session_id], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            Ok(rows) => rows,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite load turn events query error: {error}");
                return Vec::new();
            }
        };
        for row in rows {
            let Ok((seq, branch_id, payload)) = row else {
                continue;
            };
            match serde_json::from_str::<crate::agent::turn_event::TurnEvent>(&payload) {
                Ok(event) => events.push((seq, branch_id, event)),
                Err(error) => {
                    eprintln!(
                        "[pony-agent][session] load turn event {seq} parse error: {error} (tombstone isolated)"
                    );
                    let preview = if payload.len() > 100 {
                        format!("{}...", &payload[..100])
                    } else {
                        payload.clone()
                    };
                    events.push((
                        seq,
                        branch_id,
                        crate::agent::turn_event::TurnEvent::CorruptedEventTombstone {
                            turn_id: None,
                            corrupted_seq: seq,
                            error_message: format!("{error}"),
                            raw_snippet: preview,
                        },
                    ));
                }
            }
        }
        events
    }

    /// PA-093：当前事件水位（turn_event_seq:{session_id} 计数器；缺失 → MAX(seq)+1）。
    fn load_event_watermark(&self, session_id: &str) -> u64 {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return 0;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let counter_key = format!("turn_event_seq:{session_id}");
        conn.query_row(
            "SELECT value FROM store_metadata WHERE key = ?1",
            params![counter_key],
            |row| row.get::<_, String>(0),
        )
        .map(|raw| raw.parse::<u64>().unwrap_or(0))
        .unwrap_or_else(|_| {
            conn.query_row(
                "SELECT COALESCE(MAX(seq), -1) + 1 FROM turn_events WHERE session_id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|v| v.max(0) as u64)
            .unwrap_or(0)
        })
    }

    fn replace_session_traces(
        &self,
        session_id: &str,
        traces: &[TurnTraceRecord],
    ) -> SessionBackendMutationResult {        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite replace traces begin tx error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        if let Err(error) = self.replace_session_traces_tx(&tx, session_id, traces) {
            eprintln!("[pony-agent][session] SQLite replace traces error: {error}");
            return SessionBackendMutationResult::Failed;
        }
        if let Err(error) = tx.commit() {
            eprintln!("[pony-agent][session] SQLite replace traces commit error: {error}");
            return SessionBackendMutationResult::Failed;
        }
        SessionBackendMutationResult::Succeeded
    }

    fn upsert_turn_trace(
        &self,
        session_id: &str,
        trace: &TurnTraceRecord,
        trace_order: usize,
    ) -> SessionBackendMutationResult {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite trace upsert begin tx error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        if let Err(error) = self.upsert_turn_trace_tx(
            &tx,
            session_id,
            trace,
            trace_order,
            // PA-088：WriteSeparate 下不 prune——表可能含节点 refs 引用的历史 trace
            // （>24 轮），prune 会删掉它们导致重启 materialize 失败。
            // refs 保护 prune 归 PA-090 迁移阶段实现。
            false,
        ) {
            eprintln!("[pony-agent][session] SQLite trace upsert error: {error}");
            return SessionBackendMutationResult::Failed;
        }
        if let Err(error) = tx.commit() {
            eprintln!("[pony-agent][session] SQLite trace upsert commit error: {error}");
            return SessionBackendMutationResult::Failed;
        }
        SessionBackendMutationResult::Succeeded
    }

    fn update_turn_trace_terminal_event(
        &self,
        session_id: &str,
        turn_id: &str,
        event_id: Option<&str>,
        event_type: Option<&str>,
        event_version: Option<&str>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
        updated_at: u64,
    ) -> SessionBackendMutationResult {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite terminal event begin tx error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        let updated = match self.update_turn_trace_row_tx(&tx, session_id, turn_id, |trace| {
            trace.session_id = Some(session_id.to_string());
            trace.event_id = event_id.map(str::to_string);
            trace.event_type = event_type.map(str::to_string);
            trace.event_version = event_version.map(str::to_string);
            trace.sequence = sequence;
            trace.emitted_at_ms = emitted_at_ms;
            trace.updated_at = updated_at;
        }) {
            Ok(updated) => updated,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite terminal event update error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        if !updated {
            return SessionBackendMutationResult::NotFound;
        }
        if let Err(error) = tx.commit() {
            eprintln!("[pony-agent][session] SQLite terminal event commit error: {error}");
            return SessionBackendMutationResult::Failed;
        }
        SessionBackendMutationResult::Succeeded
    }

    fn append_turn_trace_hook_records(
        &self,
        session_id: &str,
        turn_id: &str,
        hook_trace_records: &[crate::agent::hooks::HookTraceRecord],
        updated_at: u64,
    ) -> SessionBackendMutationResult {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite hook append begin tx error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        let updated = match self.update_turn_trace_row_tx(&tx, session_id, turn_id, |trace| {
            trace.session_id = Some(session_id.to_string());
            trace
                .hook_trace_records
                .extend(hook_trace_records.iter().cloned());
            trace.updated_at = updated_at;
        }) {
            Ok(updated) => updated,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite hook append error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        if !updated {
            return SessionBackendMutationResult::NotFound;
        }
        if let Err(error) = tx.commit() {
            eprintln!("[pony-agent][session] SQLite hook append commit error: {error}");
            return SessionBackendMutationResult::Failed;
        }
        SessionBackendMutationResult::Succeeded
    }

    fn persist_session_with_trace_mutation(
        &self,
        session_id: &str,
        session: &SessionState,
        mutation: SessionTraceMutation,
    ) -> SessionBackendMutationResult {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite combined persist begin tx error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };

        if let Err(error) = self.upsert_session_row_tx(&tx, session_id, session) {
            eprintln!("[pony-agent][session] SQLite combined session upsert error: {error}");
            return SessionBackendMutationResult::Failed;
        }

        let mutation_result = match mutation {
            SessionTraceMutation::ReplaceAll { traces } => self
                .replace_session_traces_tx(&tx, session_id, &traces)
                .map(|_| SessionBackendMutationResult::Succeeded),
            SessionTraceMutation::UpsertOne { trace, trace_order } => self
                .upsert_turn_trace_tx(
                    &tx,
                    session_id,
                    &trace,
                    trace_order,
                    // PA-088：Authoritative 会话不 prune——表可能含节点 refs 引用的
                    // 历史 trace（>24 轮），prune 会删掉它们导致重启 materialize 失败。
                    // refs 保护 prune 归 PA-090 迁移阶段实现。
                    false,
                )
                .map(|_| SessionBackendMutationResult::Succeeded),
            SessionTraceMutation::UpdateTerminalEvent {
                turn_id,
                event_id,
                event_type,
                event_version,
                sequence,
                emitted_at_ms,
                updated_at,
                event_watermark: _,
            } => match self.update_turn_trace_row_tx(&tx, session_id, &turn_id, |trace| {
                trace.session_id = Some(session_id.to_string());
                trace.event_id = event_id.clone();
                trace.event_type = event_type.clone();
                trace.event_version = event_version.clone();
                trace.sequence = sequence;
                trace.emitted_at_ms = emitted_at_ms;
                trace.updated_at = updated_at;
            }) {
                Ok(true) => Ok(SessionBackendMutationResult::Succeeded),
                Ok(false) => Ok(SessionBackendMutationResult::NotFound),
                Err(error) => Err(error),
            },
            SessionTraceMutation::AppendHookRecords {
                turn_id,
                hook_trace_records,
                updated_at,
            } => match self.update_turn_trace_row_tx(&tx, session_id, &turn_id, |trace| {
                trace.session_id = Some(session_id.to_string());
                trace.hook_trace_records.extend(hook_trace_records.clone());
                trace.updated_at = updated_at;
            }) {
                Ok(true) => Ok(SessionBackendMutationResult::Succeeded),
                Ok(false) => Ok(SessionBackendMutationResult::NotFound),
                Err(error) => Err(error),
            },
        };

        let mutation_result = match mutation_result {
            Ok(result) => result,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite combined trace mutation error: {error}");
                return SessionBackendMutationResult::Failed;
            }
        };

        if matches!(mutation_result, SessionBackendMutationResult::NotFound) {
            return SessionBackendMutationResult::NotFound;
        }

        if !matches!(mutation_result, SessionBackendMutationResult::Succeeded) {
            return SessionBackendMutationResult::Failed;
        }

        if let Err(error) = tx.commit() {
            eprintln!("[pony-agent][session] SQLite combined persist commit error: {error}");
            return SessionBackendMutationResult::Failed;
        }

        SessionBackendMutationResult::Succeeded
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

/// Returns the default SQLite database path.
pub fn default_sqlite_path() -> PathBuf {
    #[cfg(test)]
    {
        // 测试必须隔离：绝不读写用户生产数据库（%LOCALAPPDATA%/PonyAgent/sessions.db）。
        unique_test_sqlite_path()
    }

    #[cfg(not(test))]
    {
        dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PonyAgent")
            .join("sessions.db")
    }
}

/// 单元测试专用的会话数据库路径：按进程 + 时间戳唯一化，位于系统临时目录。
#[cfg(test)]
fn unique_test_sqlite_path() -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "pony-agent-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ))
        .join("sessions.db")
}

#[cfg(test)]
mod tests {
    use super::super::session::{
        HistoryCursor, SessionStore, TraceMigrationState, TurnTraceRecord,
    };
    use super::*;
    use std::fs;

    fn trace(turn_id: &str, title: &str, updated_at: u64) -> TurnTraceRecord {
        TurnTraceRecord {
            turn_id: turn_id.to_string(),
            session_id: Some("s1".to_string()),
            event_id: None,
            event_type: Some("turn.completed".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(1),
            emitted_at_ms: Some(updated_at),
            title: title.to_string(),
            phase: "completed".to_string(),
            trace_steps: Vec::new(),
            trace_timeline: Vec::new(),
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: Vec::new(),
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: None,
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            updated_at,
        }
    }

    fn unique_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sqlite-test-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn roundtrip_empty_store() {
        let dir = unique_dir("empty");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::DualWrite,
        );
        let store = PersistedStore::default();
        backend.save_store(&store);

        let loaded = backend.load_store().unwrap();
        assert!(loaded.sessions.is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_workspaces_metadata_falls_back_to_default_on_store_load() {
        // PA-079 P1-2：store_metadata 的 workspaces JSON 损坏 → read_metadata 回退空 → SessionStore 重建默认。
        let dir = unique_dir("corrupt-ws");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::DualWrite,
        );

        {
            let mut slot = backend.connection().expect("connection");
            let conn = slot.as_mut().expect("initialized");
            conn.execute(
                "INSERT OR REPLACE INTO store_metadata (key, value) VALUES ('workspaces', '{bad json')",
                [],
            )
            .expect("insert corrupt workspaces metadata");
        }

        let store = SessionStore::with_backend(Box::new(backend));
        let workspaces = store.list_workspaces();
        assert_eq!(workspaces.len(), 1, "损坏回退后应只剩默认 workspace");
        assert_eq!(
            workspaces[0].id,
            crate::agent::workspace::DEFAULT_WORKSPACE_ID
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn workspaces_roundtrip_through_sqlite_store_metadata() {
        // PA-079：注册表随 PersistedStore.workspaces 经 store_metadata key=workspaces 持久化。
        let dir = unique_dir("workspaces");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::DualWrite,
        );
        let root = dir.join("ws-root");
        fs::create_dir_all(&root).unwrap();
        let mut store = PersistedStore::default();
        store
            .workspaces
            .push(crate::agent::workspace::WorkspaceRecord {
                id: crate::agent::workspace::DEFAULT_WORKSPACE_ID.to_string(),
                name: "默认工作区".to_string(),
                root_path: root.display().to_string(),
            });
        backend.save_store(&store);

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(
            loaded.workspaces[0].id,
            crate::agent::workspace::DEFAULT_WORKSPACE_ID
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn title_override_and_archived_roundtrip_through_sqlite_blob() {
        // 侧边栏三级树（ADR 0015）：override/archived 随会话 blob 走既有序列化，
        // LegacyBlob 与 WriteSeparate 两模式同路径——重启后改名/归档不得丢失。
        let dir = unique_dir("title-archived");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");

        for mode in [
            SeparateTraceTableMode::DualWrite,
            SeparateTraceTableMode::WriteSeparate,
        ] {
            let backend = SqliteSessionBackend::new_with_trace_mode(db_path.clone(), mode);
            let mut store = PersistedStore::default();
            let mut session = minimal_session("s1", "derived-title", 100);
            session.title_override = Some("用户命名".to_string());
            session.archived = true;
            store.sessions.insert("s1".to_string(), session);
            backend.save_store(&store);

            let loaded = backend.load_store().unwrap();
            let restored = loaded.sessions.get("s1").expect("session blob");
            assert_eq!(
                restored.title_override.as_deref(),
                Some("用户命名"),
                "mode={mode:?}"
            );
            assert!(restored.archived, "mode={mode:?}");
        }

        fs::remove_dir_all(&dir).ok();
    }

    fn minimal_session(id: &str, title: &str, updated_at_ms: u64) -> SessionState {
        SessionState {
            conversation_id: id.to_string(),
            title: title.to_string(),
            summary: String::new(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 0,
            last_referenced_file: None,
            updated_at_ms,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        }
    }

    #[test]
    fn removes_sessions_missing_from_latest_store_snapshot() {
        let dir = unique_dir("delete");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("delete.db");

        let backend =
            SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::DualWrite);

        let mut store = PersistedStore::default();
        store
            .sessions
            .insert("s1".to_string(), minimal_session("s1", "first", 1000));
        store
            .sessions
            .insert("s2".to_string(), minimal_session("s2", "second", 2000));
        backend.save_store(&store);

        store.sessions.remove("s1");
        backend.save_store(&store);

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert!(!loaded.sessions.contains_key("s1"));
        assert_eq!(loaded.sessions["s2"].title, "second");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_session_updates_one_row_without_rewriting_other_sessions() {
        let dir = unique_dir("upsert-session");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("upsert-session.db");

        let backend =
            SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::DualWrite);

        let mut store = PersistedStore::default();
        store
            .sessions
            .insert("s1".to_string(), minimal_session("s1", "first", 1000));
        store
            .sessions
            .insert("s2".to_string(), minimal_session("s2", "second", 2000));
        backend.save_store(&store);

        let mut updated = minimal_session("s1", "first-updated", 3000);
        updated
            .turn_trace_history
            .push(trace("turn-1", "trace title", 42));

        assert!(backend.upsert_session("s1", &updated));

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(loaded.sessions["s1"].title, "first-updated");
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), 1);
        assert_eq!(loaded.sessions["s2"].title, "second");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remove_session_updates_rows_and_metadata_incrementally() {
        let dir = unique_dir("remove-session-incremental");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("remove-session-incremental.db");

        let backend =
            SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::DualWrite);

        let mut store = PersistedStore::default();
        store
            .sessions
            .insert("s1".to_string(), minimal_session("s1", "first", 1000));
        store
            .sessions
            .insert("s2".to_string(), minimal_session("s2", "second", 2000));
        store.attachment_assets.insert(
            "asset:s1/file.dataurl".to_string(),
            AttachmentAsset {
                id: "asset:s1/file.dataurl".to_string(),
                session_id: "s1".to_string(),
                name: Some("file".to_string()),
                mime_type: "image/png".to_string(),
                relative_path: "s1/file.dataurl".to_string(),
                size_bytes: 4,
                created_at_ms: 1000,
                ..AttachmentAsset::default()
            },
        );
        store
            .session_attachment_index
            .insert("s1".to_string(), vec!["asset:s1/file.dataurl".to_string()]);
        backend.save_store(&store);

        let mut attachment_assets = store.attachment_assets.clone();
        attachment_assets.remove("asset:s1/file.dataurl");
        let mut session_attachment_index = store.session_attachment_index.clone();
        session_attachment_index.remove("s1");

        assert!(backend.remove_session(
            "s1",
            &attachment_assets,
            &session_attachment_index,
            &HashMap::<String, McpSourceSnapshot>::new(),
            &HashMap::<String, SkillSourceSnapshot>::new(),
        ));

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert!(!loaded.sessions.contains_key("s1"));
        assert!(loaded.attachment_assets.is_empty());
        assert!(!loaded.session_attachment_index.contains_key("s1"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_store_normalizes_session_conversation_id_to_row_key() {
        let dir = unique_dir("normalize-conversation-id");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("normalize-conversation-id.db");

        let backend =
            SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::DualWrite);

        let mut store = PersistedStore::default();
        let mut mismatched = minimal_session("payload-id", "mismatch", 1000);
        mismatched.conversation_id = "payload-id".to_string();
        store.sessions.insert("row-id".to_string(), mismatched);
        backend.save_store(&store);

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions["row-id"].conversation_id, "row-id");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_store_prefers_trace_table_rows_over_legacy_blob() {
        let dir = unique_dir("trace-merge");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-merge.db");

        let backend =
            SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::DualWrite);

        let mut session = minimal_session("s1", "first", 1000);
        session
            .turn_trace_history
            .push(trace("turn-1", "legacy", 1));

        let mut store = PersistedStore::default();
        store.sessions.insert("s1".to_string(), session.clone());
        backend.save_store(&store);

        let mut updated = session.clone();
        updated.trace_migration_state = TraceMigrationState::DualWrite;
        updated.turn_trace_history[0].title = "table".to_string();
        updated
            .turn_trace_history
            .push(trace("turn-2", "second", 2));
        assert!(backend.upsert_session("s1", &updated));

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), 2);
        assert_eq!(loaded.sessions["s1"].turn_trace_history[0].title, "table");
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history[1].turn_id,
            "turn-2"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn authoritative_sessions_do_not_fallback_to_blob_traces() {
        let dir = unique_dir("trace-authoritative");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-authoritative.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );

        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));
        assert!(matches!(
            backend.upsert_turn_trace("s1", &trace("turn-legacy", "legacy", 1), 0),
            SessionBackendMutationResult::Succeeded
        ));

        let slot = backend.connection().unwrap();
        let conn = slot.as_ref().expect("connection initialized");
        let mut authoritative_blob = minimal_session("s1", "first", 1000);
        authoritative_blob.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        conn.execute(
            "UPDATE sessions SET session_data = ?2 WHERE conversation_id = ?1",
            params!["s1", serde_json::to_string(&authoritative_blob).unwrap()],
        )
        .unwrap();
        drop(slot);

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), 1);
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history[0].turn_id,
            "turn-legacy"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_turn_trace_updates_trace_table_without_blob_roundtrip_dependency() {
        let dir = unique_dir("trace-upsert");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-upsert.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );

        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        backend.save_store(&PersistedStore {
            sessions: HashMap::from([("s1".to_string(), session)]),
            ..PersistedStore::default()
        });

        assert!(matches!(
            backend.upsert_turn_trace("s1", &trace("turn-1", "first-trace", 11), 0),
            SessionBackendMutationResult::Succeeded
        ));

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), 1);
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history[0].title,
            "first-trace"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_turn_trace_terminal_event_mutates_trace_table_row() {
        let dir = unique_dir("trace-terminal-update");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-terminal-update.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );

        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));
        assert!(matches!(
            backend.upsert_turn_trace("s1", &trace("turn-1", "first-trace", 11), 0),
            SessionBackendMutationResult::Succeeded
        ));

        assert!(matches!(
            backend.update_turn_trace_terminal_event(
                "s1",
                "turn-1",
                Some("turn-1:4"),
                Some("turn.completed"),
                Some("turn-event-v1"),
                Some(4),
                Some(4242),
                99,
            ),
            SessionBackendMutationResult::Succeeded
        ));

        let loaded = backend.load_store().unwrap();
        let trace = &loaded.sessions["s1"].turn_trace_history[0];
        assert_eq!(trace.event_id.as_deref(), Some("turn-1:4"));
        assert_eq!(trace.sequence, Some(4));
        assert_eq!(trace.emitted_at_ms, Some(4242));
        assert_eq!(trace.updated_at, 99);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn append_turn_trace_hook_records_mutates_trace_table_row() {
        let dir = unique_dir("trace-hook-append");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-hook-append.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );

        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));
        assert!(matches!(
            backend.upsert_turn_trace("s1", &trace("turn-1", "first-trace", 11), 0),
            SessionBackendMutationResult::Succeeded
        ));

        let hook = crate::agent::hooks::HookTraceRecord {
            hook_name: "audit.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::ToolCallEnd,
            hook_order: 1,
            result_kind: crate::agent::hooks::HookResultKind::Observe,
            structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                summary: "hook observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 5,
            input_summary: Some("tool".to_string()),
            persistence_evidence_ref: None,
            summary: "hook observed".to_string(),
        };

        assert!(matches!(
            backend.append_turn_trace_hook_records("s1", "turn-1", &[hook.clone()], 101),
            SessionBackendMutationResult::Succeeded
        ));

        let loaded = backend.load_store().unwrap();
        let trace = &loaded.sessions["s1"].turn_trace_history[0];
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert_eq!(trace.hook_trace_records[0].hook_name, "audit.observe");
        assert_eq!(trace.updated_at, 101);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_store_write_separate_hot_path_keeps_trace_rows_across_reload() {
        let dir = unique_dir("session-store-write-separate");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("session-store-write-separate.db");

        let backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut store = SessionStore::with_backend(backend);

        store.record_turn_trace(Some("s1"), trace("turn-1", "first-trace", 11));
        store.annotate_turn_trace_terminal_event(
            Some("s1"),
            "turn-1",
            Some("turn-1:4".to_string()),
            Some("turn.completed".to_string()),
            Some("turn-event-v1".to_string()),
            Some(4),
            Some(4242),
        );
        store.append_turn_trace_hook_records(
            Some("s1"),
            "turn-1",
            vec![crate::agent::hooks::HookTraceRecord {
                hook_name: "audit.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::ToolCallEnd,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Observe,
                structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                    summary: "hook observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 5,
                input_summary: Some("tool".to_string()),
                persistence_evidence_ref: None,
                summary: "hook observed".to_string(),
            }],
        );

        let reloaded_backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut reloaded = SessionStore::with_backend(reloaded_backend);
        let snapshot = reloaded.snapshot(Some("s1"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(snapshot.turn_trace_history[0].turn_id, "turn-1");
        assert_eq!(
            snapshot.turn_trace_history[0].event_id.as_deref(),
            Some("turn-1:4")
        );
        assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 1);
        assert_eq!(
            snapshot.turn_trace_history[0].hook_trace_records[0].hook_name,
            "audit.observe"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn trace_updates_report_not_found_when_row_missing() {
        let dir = unique_dir("trace-not-found");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-not-found.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );

        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        assert!(matches!(
            backend.update_turn_trace_terminal_event(
                "s1",
                "missing-turn",
                Some("turn:1"),
                Some("turn.completed"),
                Some("turn-event-v1"),
                Some(1),
                Some(1),
                1,
            ),
            SessionBackendMutationResult::NotFound
        ));

        let hook = crate::agent::hooks::HookTraceRecord {
            hook_name: "audit.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::ToolCallEnd,
            hook_order: 1,
            result_kind: crate::agent::hooks::HookResultKind::Observe,
            structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                summary: "hook observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 5,
            input_summary: Some("tool".to_string()),
            persistence_evidence_ref: None,
            summary: "hook observed".to_string(),
        };
        assert!(matches!(
            backend.append_turn_trace_hook_records("s1", "missing-turn", &[hook], 1),
            SessionBackendMutationResult::NotFound
        ));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_separate_keeps_all_trace_rows_for_authoritative_session() {
        // PA-088：Authoritative 会话 upsert 不 prune——表可能含节点 refs 引用的
        // 历史 trace（>24 轮），prune 会删掉它们导致重启 materialize 失败。
        // （原 prune 行为测试已按新设计更新。）
        let dir = unique_dir("trace-prune");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-prune.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );

        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        for index in 0..40 {
            assert!(matches!(
                backend.upsert_turn_trace(
                    "s1",
                    &trace(&format!("turn-{index}"), "trace", index as u64),
                    index,
                ),
                SessionBackendMutationResult::Succeeded
            ));
        }

        let loaded = backend.load_store().unwrap();
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history.len(),
            40,
            "Authoritative 会话 upsert 不应 prune 到 24 行"
        );
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history[0].turn_id,
            "turn-0"
        );
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history[39].turn_id,
            "turn-39"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_store_write_separate_preserves_branch_trace_across_switch_and_restore() {
        let dir = unique_dir("write-separate-branch-restore");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("write-separate-branch-restore.db");

        let backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut store = SessionStore::with_backend(backend);
        store.append_turn(Some("switch-session"), "第一问", "第一答", None, Vec::new());
        store.record_turn_trace(Some("switch-session"), trace("turn-main", "main", 1));
        store.append_turn(Some("switch-session"), "第二问", "第二答", None, Vec::new());
        store.record_turn_trace(Some("switch-session"), trace("turn-main-2", "main-2", 2));

        let (nodes_before, branches_before, _) = store.load_history_graph(Some("switch-session"));
        let first_node_id = nodes_before[0].node_id.clone();
        // The graph includes a checkpoint root node before the two turns, so
        // the main branch head is taken from the branch record (the last turn
        // node), not nodes_before[1].
        let second_node_id = branches_before[0]
            .head_node_id
            .clone()
            .expect("main branch head node should exist");

        store
            .fork_from_history_node(Some("switch-session"), first_node_id.as_str(), None)
            .expect("fork should succeed");
        store.append_turn(
            Some("switch-session"),
            "在分叉上继续",
            "分叉回答",
            None,
            Vec::new(),
        );
        store.record_turn_trace(Some("switch-session"), trace("turn-fork", "fork", 3));

        let (_, branches_after_fork, _) = store.load_history_graph(Some("switch-session"));
        let fork_branch_id = branches_after_fork
            .iter()
            .find(|branch| branch.branch_id != "branch-main")
            .map(|branch| branch.branch_id.clone())
            .expect("fork branch should exist");

        store
            .switch_history_branch(Some("switch-session"), "branch-main", None)
            .expect("switch to main branch should succeed");
        let reloaded_main_backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut reloaded_main = SessionStore::with_backend(reloaded_main_backend);
        let main_snapshot = reloaded_main.snapshot(Some("switch-session"), &[]);
        assert_eq!(
            main_snapshot.resolved_node_id.as_deref(),
            Some(second_node_id.as_str())
        );
        assert_eq!(main_snapshot.turn_trace_history.len(), 2);
        assert_eq!(main_snapshot.turn_trace_history[1].turn_id, "turn-main-2");

        let restored = store
            .restore_branch_head(Some("switch-session"), Some(fork_branch_id.as_str()), None)
            .expect("restore fork branch head should succeed");
        assert_ne!(
            restored.resolved_node_id.as_deref(),
            Some(second_node_id.as_str())
        );

        let reloaded_fork_backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut reloaded_fork = SessionStore::with_backend(reloaded_fork_backend);
        let fork_snapshot = reloaded_fork.snapshot(Some("switch-session"), &[]);
        assert_eq!(
            fork_snapshot.history_cursor.active_branch_id.as_deref(),
            Some(fork_branch_id.as_str())
        );
        assert!(fork_snapshot
            .turn_trace_history
            .iter()
            .any(|trace| trace.turn_id == "turn-fork"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_store_write_separate_strips_live_trace_from_session_blob() {
        let dir = unique_dir("write-separate-blob-strip");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("write-separate-blob-strip.db");

        let backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut store = SessionStore::with_backend(backend);
        store.record_turn_trace(Some("s1"), trace("turn-1", "first-trace", 11));

        let inspect_backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let slot = inspect_backend.connection().unwrap();
        let conn = slot.as_ref().expect("connection initialized");
        let raw: String = conn
            .query_row(
                "SELECT session_data FROM sessions WHERE conversation_id = ?1",
                params!["s1"],
                |row| row.get(0),
            )
            .unwrap();
        let persisted: SessionState = serde_json::from_str(&raw).unwrap();
        assert!(matches!(
            persisted.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        ));
        assert!(persisted.turn_trace_history.is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_store_write_separate_repairs_missing_trace_row_on_update() {
        let dir = unique_dir("write-separate-repair-missing-trace");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("write-separate-repair-missing-trace.db");

        let backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut store = SessionStore::with_backend(backend);
        store.record_turn_trace(Some("s1"), trace("turn-1", "first-trace", 11));

        let inspect_backend = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        );
        let slot = inspect_backend.connection().unwrap();
        let conn = slot.as_ref().expect("connection initialized");
        conn.execute(
            "DELETE FROM session_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
            params!["s1", "turn-1"],
        )
        .unwrap();
        drop(slot);

        store.annotate_turn_trace_terminal_event(
            Some("s1"),
            "turn-1",
            Some("turn-1:4".to_string()),
            Some("turn.completed".to_string()),
            Some("turn-event-v1".to_string()),
            Some(4),
            Some(4242),
        );

        let reloaded_backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut reloaded = SessionStore::with_backend(reloaded_backend);
        let snapshot = reloaded.snapshot(Some("s1"), &[]);
        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(
            snapshot.turn_trace_history[0].event_id.as_deref(),
            Some("turn-1:4")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_turn_trace_does_not_prune_authoritative_session_rows() {
        // PA-088 P0-1′ 回归：Authoritative 会话 upsert 不 prune——
        // 表可能含节点 refs 引用的历史 trace（>24 轮），prune 会删掉它们。
        let dir = unique_dir("no-prune-authoritative");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("no-prune-authoritative.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        );
        let session = super::SessionState {
            conversation_id: "s1".to_string(),
            title: "t".to_string(),
            summary: "s".to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: super::TraceMigrationState::TraceTableAuthoritative,
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 0,
            last_referenced_file: None,
            updated_at_ms: 1,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: Default::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        };
        assert!(
            backend.upsert_session("s1", &session),
            "upsert should succeed"
        );

        // 写入 30 条 trace（>24 上限），全部应保留（不 prune）
        for index in 0..30u64 {
            let result = backend.upsert_turn_trace(
                "s1",
                &trace(&format!("turn-{index}"), "t", 100 + index),
                index as usize,
            );
            assert!(matches!(result, SessionBackendMutationResult::Succeeded));
        }

        let reloaded = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let loaded = reloaded.load_store().expect("load should succeed");
        let loaded_session = loaded
            .sessions
            .get("s1")
            .expect("session should exist after reload");
        assert_eq!(
            loaded_session.turn_trace_history.len(),
            30,
            "Authoritative �Ự upsert ��Ӧ prune �� 24 ��"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn protected_prune_keeps_referenced_traces_and_removes_unreferenced() {
        // PA-090：refs 保护 prune——被节点/顶层 refs 引用的 trace 不删，
        // 只删无引用且超限的最旧记录。
        let dir = unique_dir("protected-prune");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("protected-prune.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        // 写入 5 条 trace
        for index in 0..5u64 {
            assert!(matches!(
                backend.upsert_turn_trace(
                    "s1",
                    &trace(&format!("turn-{index}"), "trace", index),
                    index as usize,
                ),
                SessionBackendMutationResult::Succeeded
            ));
        }

        // 保护 turn-0 和 turn-4（模拟节点 refs），limit=3 → 应删 turn-1, turn-2
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            let tx = guard.unchecked_transaction().expect("tx");
            let protected: std::collections::HashSet<String> =
                ["turn-0".to_string(), "turn-4".to_string()]
                    .into_iter()
                    .collect();
            backend
                .prune_session_traces_protected_tx(&tx, "s1", 3, &protected)
                .expect("protected prune");
            tx.commit().expect("commit");
        } // guard 在此释放连接锁

        let loaded = backend.load_store().expect("load");
        let traces = &loaded.sessions["s1"].turn_trace_history;
        let turn_ids: Vec<&str> = traces.iter().map(|t| t.turn_id.as_str()).collect();
        assert_eq!(turn_ids.len(), 3);
        assert!(turn_ids.contains(&"turn-0"), "被引用 trace 保留");
        assert!(turn_ids.contains(&"turn-4"), "被引用 trace 保留");
        assert!(!turn_ids.contains(&"turn-1"), "无引用最旧被删");
        assert!(!turn_ids.contains(&"turn-2"), "无引用被删");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn protected_prune_fails_closed_when_all_referenced() {
        // PA-090：全部被引用时零删除（fail closed 语义）。
        let dir = unique_dir("protected-prune-all-ref");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("protected-prune-all-ref.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));
        for index in 0..5u64 {
            assert!(matches!(
                backend.upsert_turn_trace(
                    "s1",
                    &trace(&format!("turn-{index}"), "trace", index),
                    index as usize,
                ),
                SessionBackendMutationResult::Succeeded
            ));
        }

        // 全部保护，limit=3 → 零删除（保留 5 条）
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            let tx = guard.unchecked_transaction().expect("tx");
            let protected: std::collections::HashSet<String> =
                (0..5u64).map(|i| format!("turn-{i}")).collect();
            backend
                .prune_session_traces_protected_tx(&tx, "s1", 3, &protected)
                .expect("protected prune");
            tx.commit().expect("commit");
        } // guard 在此释放连接锁

        let loaded = backend.load_store().expect("load");
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history.len(),
            5,
            "全部被引用时零删除"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn protected_prune_fails_closed_when_protected_set_empty() {
        // PA-090 P1-3：protected 集合为空但表非空（refs 缺失/损坏）→ 零删除。
        let dir = unique_dir("protected-prune-empty");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("protected-prune-empty.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));
        for index in 0..5u64 {
            assert!(matches!(
                backend.upsert_turn_trace(
                    "s1",
                    &trace(&format!("turn-{index}"), "trace", index),
                    index as usize,
                ),
                SessionBackendMutationResult::Succeeded
            ));
        }

        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            let tx = guard.unchecked_transaction().expect("tx");
            let protected: std::collections::HashSet<String> = std::collections::HashSet::new();
            backend
                .prune_session_traces_protected_tx(&tx, "s1", 3, &protected)
                .expect("protected prune");
            tx.commit().expect("commit");
        }

        let loaded = backend.load_store().expect("load");
        assert_eq!(
            loaded.sessions["s1"].turn_trace_history.len(),
            5,
            "protected 为空时零删除（fail closed）"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn node_materialize_uses_full_table_not_top_level_filtered_subset() {
        // PA-090 P0-1 回归：节点 materialize 必须从全量表解析 refs——
        // 顶层 refs 过滤只作用于顶层显示；fork 分支/超 24 轮节点的 refs
        // 若从过滤后子集解析会缺失，且后续 union 重写会永久删除这些 trace。
        let dir = unique_dir("node-materialize-full-table");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("node-materialize-full-table.db");

        let backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut store = SessionStore::with_backend(backend);
        store.append_turn(Some("s1"), "第一问", "第一答", None, Vec::new());
        store.record_turn_trace(Some("s1"), trace("turn-main", "main", 1));
        store.append_turn(Some("s1"), "第二问", "第二答", None, Vec::new());
        store.record_turn_trace(Some("s1"), trace("turn-main-2", "main-2", 2));

        // fork 分支写入独有 trace
        let (nodes_before, _, _) = store.load_history_graph(Some("s1"));
        let first_node_id = nodes_before[0].node_id.clone();
        store
            .fork_from_history_node(Some("s1"), first_node_id.as_str(), None)
            .expect("fork");
        store.append_turn(Some("s1"), "分叉", "分叉答", None, Vec::new());
        store.record_turn_trace(Some("s1"), trace("turn-fork", "fork", 3));

        // 切回 main 分支并持久化（顶层 refs 只含 main 分支）
        store
            .switch_history_branch(Some("s1"), "branch-main", None)
            .expect("switch to main");

        // 重启：节点 materialize 应从全量表解析（含 fork 分支的 turn-fork）
        let reloaded_backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut reloaded = SessionStore::with_backend(reloaded_backend);
        let snapshot = reloaded.snapshot(Some("s1"), &[]);

        // 顶层只显示 main 分支（2 条）
        assert_eq!(snapshot.turn_trace_history.len(), 2);
        assert_eq!(snapshot.turn_trace_history[1].turn_id, "turn-main-2");

        // 所有节点 materialize 完整（含 fork 分支节点）——通过 backend.load_store
        // 检查完整 SessionState（snapshot 是轻量投影，节点 trace 被清空）。
        let inspect_backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let loaded_store = inspect_backend.load_store().expect("load store");
        let loaded_session = loaded_store
            .sessions
            .get("s1")
            .expect("session in loaded store");
        let mut all_node_trace_ids: Vec<String> = Vec::new();
        for node in loaded_session.history_nodes.iter() {
            for trace in &node.turn_trace_history {
                all_node_trace_ids.push(trace.turn_id.clone());
            }
        }
        assert!(
            all_node_trace_ids.contains(&"turn-fork".to_string()),
            "fork 分支节点 trace 应从全量表 materialize（顶层过滤不影响节点）"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn persist_command_writes_normalized_tables_and_checks_epoch() {
        let dir = unique_dir("persist-command");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("persist-command.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));
        // 建 normalized_sessions 行（FK 依赖）
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute(
                    "INSERT OR REPLACE INTO normalized_sessions (session_id, title, updated_at_ms) VALUES ('s1', 'first', 1000)",
                    [],
                )
                .expect("insert normalized session");
        }

        let outcome = backend.persist_command(PersistCommand::AppendMessage {
            epoch: 1,
            session_id: "s1".to_string(),
            message: crate::agent::session::TurnHistoryMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                attachments: Vec::new(),
                turn_id: Some("turn-1".to_string()),
                status: None,
                model_name: None,
                token_count: None,
                reasoning_content: None,
            },
            ordinal: 0,
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        let outcome = backend.persist_command(PersistCommand::AppendTrace {
            epoch: 1,
            session_id: "s1".to_string(),
            trace: trace("turn-1", "first", 100),
            trace_order: 0,
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute(
                    "INSERT OR REPLACE INTO store_metadata (key, value) VALUES ('storage.normalized.v1.epoch', '2')",
                    [],
                )
                .expect("set epoch");
        }

        let outcome = backend.persist_command(PersistCommand::AppendMessage {
            epoch: 1,
            session_id: "s1".to_string(),
            message: crate::agent::session::TurnHistoryMessage {
                role: "user".to_string(),
                content: "stale".to_string(),
                attachments: Vec::new(),
                turn_id: Some("turn-2".to_string()),
                status: None,
                model_name: None,
                token_count: None,
                reasoning_content: None,
            },
            ordinal: 1,
        });
        assert_eq!(
            outcome,
            PersistCommandOutcome::Failed,
            "old epoch should fail"
        );

        let outcome = backend.persist_command(PersistCommand::AppendMessage {
            epoch: 2,
            session_id: "s1".to_string(),
            message: crate::agent::session::TurnHistoryMessage {
                role: "user".to_string(),
                content: "fresh".to_string(),
                attachments: Vec::new(),
                turn_id: Some("turn-2".to_string()),
                status: None,
                model_name: None,
                token_count: None,
                reasoning_content: None,
            },
            ordinal: 1,
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        let loaded = backend.load_store().expect("load");
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), 1);
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let msg_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM normalized_messages WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .expect("count messages");
        assert_eq!(msg_count, 2, "two messages in normalized_messages");
        let trace_count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM normalized_turn_traces WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .expect("count traces");
        assert_eq!(trace_count, 1, "one trace in normalized_turn_traces");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_store_normalized_rebuilds_session_from_normalized_tables() {
        // PA-089 阶段 5：切读后 load_store 从规范化表重建 SessionState。
        let dir = unique_dir("normalized-loader");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("normalized-loader.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            // 建 normalized_sessions + messages + turns + traces + nodes
            guard
                .execute(
                    "INSERT INTO normalized_sessions (session_id, title, summary, turn_count, updated_at_ms, trace_migration_state) VALUES ('s1', 't', 's', 1, 1000, 'trace_table_authoritative')",
                    [],
                )
                .expect("insert session");
            guard
                .execute(
                    "INSERT INTO normalized_messages (session_id, message_id, turn_id, ordinal, role, content) VALUES ('s1', 'turn-1-user', 'turn-1', 0, 'user', 'hello'), ('s1', 'turn-1-assistant', 'turn-1', 1, 'assistant', 'hi')",
                    [],
                )
                .expect("insert messages");
            guard
                .execute(
                    "INSERT INTO normalized_turn_traces (session_id, turn_id, trace_order, phase, updated_at_ms) VALUES ('s1', 'turn-1', 0, 'completed', 1000)",
                    [],
                )
                .expect("insert trace");
            guard
                .execute(
                    "INSERT INTO session_turn_traces (session_id, turn_id, updated_at_ms, trace_order, trace_data) VALUES ('s1', 'turn-1', 1000, 0, '{\"turnId\":\"turn-1\",\"title\":\"t\",\"phase\":\"completed\",\"updatedAt\":1000}')",
                    [],
                )
                .expect("insert legacy trace");
            guard
                .execute(
                    "INSERT INTO normalized_history_nodes (session_id, node_id, branch_id, kind, summary, title, created_at_ms, turn_trace_refs_json, snapshot_json) VALUES ('s1', 'node-1', 'branch-main', 'turn_committed', 's', 't', 100, '[{\"turnId\":\"turn-1\",\"updatedAtMs\":1000}]', '{\"v\":1,\"history\":[],\"turnTraceHistory\":[]}')",
                    [],
                )
                .expect("insert node");
            guard
                .execute(
                    "INSERT INTO normalized_history_cursor (session_id, visible_node_id, active_branch_id, mode) VALUES ('s1', 'node-1', 'branch-main', 'live')",
                    [],
                )
                .expect("insert cursor");
            // 设置 phase = observing（触发切读）
            guard
                .execute(
                    "INSERT OR REPLACE INTO store_metadata (key, value) VALUES ('storage.normalized.v1.phase', 'observing')",
                    [],
                )
                .expect("set phase");
        }

        let loaded = backend.load_store().expect("load store");
        let session = loaded.sessions.get("s1").expect("session loaded");

        // 消息重建
        assert_eq!(session.history.len(), 2, "两条消息重建");
        assert_eq!(session.history[0].role, "user");
        assert_eq!(session.history[0].content, "hello");
        assert_eq!(session.history[1].role, "assistant");

        // trace 重建（从旧 session_turn_traces）
        assert_eq!(session.turn_trace_history.len(), 1, "一条 trace 重建");
        assert_eq!(session.turn_trace_history[0].turn_id, "turn-1");

        // 节点重建（snapshot_json + refs materialize）
        assert_eq!(session.history_nodes.len(), 1, "一个节点重建");
        assert_eq!(session.history_nodes[0].node_id, "node-1");
        assert_eq!(
            session.history_nodes[0].turn_trace_history.len(),
            1,
            "节点 refs materialize"
        );

        // cursor 重建
        assert_eq!(
            session.history_cursor.visible_node_id.as_deref(),
            Some("node-1")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tombstone_view_forwards_writes_to_session_blobs() {
        // PA-089 6a：sessions 视图 + INSTEAD OF trigger 应转发 INSERT/UPDATE/DELETE
        // 到 session_blobs（修复 "cannot modify sessions because it is a view"）。
        let dir = unique_dir("tombstone-view");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("tombstone-view.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        // 模拟 6a：sessions → session_blobs + 视图 + trigger
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute("ALTER TABLE sessions RENAME TO session_blobs", [])
                .expect("rename");
            guard.execute("CREATE VIEW sessions AS SELECT conversation_id, title, updated_at_ms, session_data FROM session_blobs", []).expect("view");
            guard.execute(
                "CREATE TRIGGER trg_sessions_insert INSTEAD OF INSERT ON sessions BEGIN INSERT OR REPLACE INTO session_blobs (conversation_id, title, updated_at_ms, session_data) VALUES (NEW.conversation_id, NEW.title, NEW.updated_at_ms, NEW.session_data); END;",
                [],
            ).expect("insert trigger");
            guard.execute(
                "CREATE TRIGGER trg_sessions_update INSTEAD OF UPDATE ON sessions BEGIN INSERT OR REPLACE INTO session_blobs (conversation_id, title, updated_at_ms, session_data) VALUES (OLD.conversation_id, NEW.title, NEW.updated_at_ms, NEW.session_data); END;",
                [],
            ).expect("update trigger");
            guard.execute(
                "CREATE TRIGGER trg_sessions_delete INSTEAD OF DELETE ON sessions BEGIN DELETE FROM session_blobs WHERE conversation_id = OLD.conversation_id; END;",
                [],
            ).expect("delete trigger");
        }

        // upsert_session 写 sessions 视图 → 应转发到 session_blobs
        let session = minimal_session("s1", "first", 1000);
        assert!(
            backend.upsert_session("s1", &session),
            "upsert via view should succeed"
        );

        // remove_session 删 sessions 视图 → 应转发
        assert!(
            backend.remove_session(
                "s1",
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new()
            ),
            "remove via view should succeed"
        );

        // 验证 session_blobs 已被删除
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM session_blobs WHERE conversation_id = 's1'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(count, 0, "remove 应转发到 session_blobs");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flush_events_writes_turn_events_with_contiguous_seq() {
        use crate::agent::turn_event::{TurnEndReason, TurnEvent};
        let dir = unique_dir("flush-events");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("flush-events.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        let events = vec![
            TurnEvent::TurnStart {
                turn_id: "turn-1".into(),
            },
            TurnEvent::AssistantChunk {
                turn_id: "turn-1".into(),
                step: 0,
                text: "hi".into(),
            },
            TurnEvent::TurnEnd {
                turn_id: "turn-1".into(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: Some(100),
            },
        ];
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            branch_id: "main".to_string(),
            events: events.clone(),
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let rows: Vec<(i64, String, String)> = guard
            .prepare("SELECT seq, event_type, payload FROM turn_events WHERE session_id = 's1' ORDER BY seq")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(rows.len(), 3, "all events persisted");
        for (index, (seq, event_type, payload)) in rows.iter().enumerate() {
            assert_eq!(*seq, index as i64, "seq must be 0-based contiguous");
            assert_eq!(event_type, &events[index].type_name());
            let decoded: TurnEvent = serde_json::from_str(payload).expect("decode payload");
            // 语义等价：反序列化后重序列化与原始序列化一致。
            assert_eq!(
                serde_json::to_string(&decoded).expect("re-serialize"),
                serde_json::to_string(&events[index]).expect("serialize"),
                "payload round-trip"
            );
        }
        // 计数器已推进
        let counter: String = guard
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'turn_event_seq:s1'",
                [],
                |row| row.get(0),
            )
            .expect("counter");
        assert_eq!(counter, "3");
        // 释放连接锁后再写（Mutex 单连接，持锁调用 persist_command 会死锁）
        // guard 是 as_ref() 的引用句柄，drop 本就是 no-op；真正的锁由下方
        // drop(conn) / 作用域结束释放。
        let _ = guard;
        drop(conn);

        // 第二次 flush seq 衔接（3 起）
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-2".to_string(),
            branch_id: "main".to_string(),
            events: vec![TurnEvent::TurnStart {
                turn_id: "turn-2".into(),
            }],
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let seq: i64 = guard
            .query_row(
                "SELECT seq FROM turn_events WHERE session_id = 's1' AND turn_id = 'turn-2'",
                [],
                |row| row.get(0),
            )
            .expect("seq");
        assert_eq!(seq, 3, "seq continues across batches");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flush_events_rolls_back_on_injected_failure() {
        use crate::agent::turn_event::{TurnEndReason, TurnEvent};
        let dir = unique_dir("flush-events-rollback");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("flush-events-rollback.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        TEST_INJECT_FLUSH_FAILURE_DB_SUFFIX
            .lock()
            .expect("inject lock")
            .replace("flush-events-rollback".to_string());
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            branch_id: "main".to_string(),
            events: vec![
                TurnEvent::TurnStart {
                    turn_id: "turn-1".into(),
                },
                TurnEvent::TurnEnd {
                    turn_id: "turn-1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: None,
                },
            ],
        });
        TEST_INJECT_FLUSH_FAILURE_DB_SUFFIX
            .lock()
            .expect("inject lock")
            .take();
        assert_eq!(outcome, PersistCommandOutcome::Failed, "injected failure");

        // 事务回滚：事件行与计数器均无残留
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM turn_events WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 0, "event rows must roll back");
        let counter_exists: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM store_metadata WHERE key = 'turn_event_seq:s1'",
                [],
                |row| row.get(0),
            )
            .expect("counter count");
        assert_eq!(counter_exists, 0, "counter must roll back");
        // 释放连接锁后再写（Mutex 单连接，持锁调用 persist_command 会死锁）
        // guard 是 as_ref() 的引用句柄，drop 本就是 no-op；真正的锁由下方
        // drop(conn) / 作用域结束释放。
        let _ = guard;
        drop(conn);

        // 注入关闭后重试成功（seq 从 0 起，无空洞）
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            branch_id: "main".to_string(),
            events: vec![TurnEvent::TurnStart {
                turn_id: "turn-1".into(),
            }],
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let seq: i64 = guard
            .query_row(
                "SELECT seq FROM turn_events WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .expect("seq");
        assert_eq!(seq, 0, "retry starts from 0 after rollback");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flush_events_empty_batch_is_noop() {
        let dir = unique_dir("flush-events-empty");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("flush-events-empty.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            branch_id: "main".to_string(),
            events: Vec::new(),
        });
        assert_eq!(
            outcome,
            PersistCommandOutcome::Succeeded,
            "empty batch is noop"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flush_events_concurrent_seq_no_duplicate_no_gap() {
        use crate::agent::turn_event::TurnEvent;
        use std::sync::{Arc, Barrier};
        let dir = unique_dir("flush-events-concurrent");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("flush-events-concurrent.db");
        let backend = Arc::new(SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        ));
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        // 双线程各 flush 100 个事件（连接 Mutex 串行化 seq 分配；spec 数值对齐）
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for thread_id in 0..2u32 {
            let backend = Arc::clone(&backend);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for i in 0..100u32 {
                    let outcome = backend.persist_command(PersistCommand::FlushEvents {
                        epoch: 1,
                        session_id: "s1".to_string(),
                        turn_id: format!("turn-{thread_id}-{i}"),
                        branch_id: "main".to_string(),
                        events: vec![TurnEvent::TurnStart {
                            turn_id: format!("turn-{thread_id}-{i}"),
                        }],
                    });
                    assert_eq!(outcome, PersistCommandOutcome::Succeeded);
                }
            }));
        }
        barrier.wait();
        for handle in handles {
            handle.join().expect("thread join");
        }

        // 100 行 seq 0-99 连续无重复
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let seqs: Vec<i64> = guard
            .prepare("SELECT seq FROM turn_events WHERE session_id = 's1' ORDER BY seq")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(seqs.len(), 200, "all events persisted");
        for (index, seq) in seqs.iter().enumerate() {
            assert_eq!(*seq, index as i64, "seq contiguous, no duplicate/gap");
        }
        let counter: String = guard
            .query_row(
                "SELECT value FROM store_metadata WHERE key = 'turn_event_seq:s1'",
                [],
                |row| row.get(0),
            )
            .expect("counter");
        assert_eq!(counter, "200");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backfill_resumes_partially_backfilled_sessions() {
        let dir = unique_dir("backfill-resume");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("backfill-resume.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        // 两个会话（s1/s2 各一条 user 消息）
        for (sid, content) in [("s1", "hello-1"), ("s2", "hello-2")] {
            let mut session = minimal_session(sid, "title", 1000);
            session.history = vec![crate::agent::session::TurnHistoryMessage {
                role: "user".to_string(),
                content: content.to_string(),
                attachments: Vec::new(),
                turn_id: Some(format!("turn-{sid}")),
                status: None,
                model_name: None,
                token_count: None,
                reasoning_content: None,
            }];
            assert!(backend.upsert_session(sid, &session));
        }
        // 首次干净回填：两个会话都完成
        let count = backend.backfill_turn_events().expect("first backfill");
        assert_eq!(count, 2, "both sessions backfilled");
        let count = backend.backfill_turn_events().expect("second backfill");
        assert_eq!(count, 0, "idempotent no-op");

        // 模拟"s2 回填未完成"（崩溃于 s2 中途）：删 s2 标记与事件，注入使 s2 flush 失败
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute(
                    "DELETE FROM store_metadata WHERE key = 'turn_event_backfill:s2'",
                    [],
                )
                .expect("delete marker");
            guard
                .execute("DELETE FROM turn_events WHERE session_id = 's2'", [])
                .expect("delete s2 events");
            // guard 是 as_ref() 的引用句柄，drop 本就是 no-op；真正的锁由下方
            // drop(conn) / 作用域结束释放。
            let _ = guard;
            drop(conn);
        }
        TEST_INJECT_FLUSH_FAILURE_DB_SUFFIX
            .lock()
            .expect("inject lock")
            .replace("backfill-resume.db".to_string());
        // 注入使 s2 的 flush 失败 → backfill_turn_events 返回 Err（预期）：
        // s2 无事件残留，s1 已完成跳过
        let _ = backend.backfill_turn_events();
        TEST_INJECT_FLUSH_FAILURE_DB_SUFFIX
            .lock()
            .expect("inject lock")
            .take();
        // 注入使 s2 的 flush 回滚：s2 无事件残留；s1 已完成（跳过，不受影响）
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let s1_done: i64 = guard
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM store_metadata WHERE key = 'turn_event_backfill:s1')",
                [],
                |row| row.get(0),
            )
            .expect("s1 marker");
        assert_eq!(s1_done, 1, "s1 completed sessions skip on resume");
        let s2_events: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM turn_events WHERE session_id = 's2'",
                [],
                |row| row.get(0),
            )
            .expect("s2 count");
        assert_eq!(s2_events, 0, "s2 failed flush left no events (rollback)");
        // guard 是 as_ref() 的引用句柄，drop 本就是 no-op；真正的锁由下方
        // drop(conn) / 作用域结束释放。
        let _ = guard;
        drop(conn);
        // 注入关闭后重跑：s2 从断点续跑完成
        let count = backend.backfill_turn_events().expect("final backfill");
        assert_eq!(count, 1, "s2 backfilled after injection cleared");
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let s2_events: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM turn_events WHERE session_id = 's2'",
                [],
                |row| row.get(0),
            )
            .expect("s2 count");
        assert_eq!(s2_events, 3, "s2 derived events (start/user/end)");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backfill_derives_tool_and_provider_events_from_trace() {
        
        let dir = unique_dir("backfill-trace");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("backfill-trace.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.history = vec![crate::agent::session::TurnHistoryMessage {
            role: "user".to_string(),
            content: "run".to_string(),
            attachments: Vec::new(),
            turn_id: Some("turn-1".to_string()),
            status: None,
            model_name: None,
            token_count: None,
            reasoning_content: None,
        }];
        // trace：tool_activities（running + done）+ provider_call_records
        let mut trace = trace("turn-1", "first", 100);
        trace.tool_activities = vec![
            crate::agent::telemetry::TurnToolActivity {
                id: "act-1".to_string(),
                name: "bash".to_string(),
                canonical_tool_name: None,
                display_name_zh: None,
                status: "running".to_string(),
                description: "run".to_string(),
                arguments_text: Some("{}".to_string()),
                result_text: None,
                duration_seconds: None,
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: None,
            },
            crate::agent::telemetry::TurnToolActivity {
                id: "act-1".to_string(),
                name: "bash".to_string(),
                canonical_tool_name: None,
                display_name_zh: None,
                status: "done".to_string(),
                description: "run".to_string(),
                arguments_text: Some("{}".to_string()),
                result_text: Some("ok".to_string()),
                duration_seconds: Some(0.5),
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: None,
            },
        ];
        trace.provider_call_records = vec![crate::agent::telemetry::ProviderCallCacheRecord {
            request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
            provider_source: None,
            provider_mode: None,
            input_tokens: Some(100),
            cache_hit_input_tokens: Some(40),
            cache_hit_source: None,
            cache_miss_input_tokens: Some(60),
            reasoning_tokens: Some(10),
            output_tokens: Some(50),
            total_tokens: Some(160),
            first_token_latency_ms: Some(88),
            turn_duration_ms: Some(500),
            latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
            prefix_mutation_reasons: Vec::new(),
        }];
        session.turn_trace_history = vec![trace];
        assert!(backend.upsert_session("s1", &session));

        backend.backfill_turn_events().expect("backfill");
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let types: Vec<String> = guard
            .prepare("SELECT event_type FROM turn_events WHERE session_id = 's1' ORDER BY seq")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        // turn/start, user/message, tool/call, tool/result, provider/usage
        assert!(
            types.iter().any(|t| t == "tool/call"),
            "tool/call derived: {types:?}"
        );
        assert!(
            types.iter().any(|t| t == "tool/result"),
            "tool/result derived: {types:?}"
        );
        assert!(
            types.iter().any(|t| t == "provider/usage"),
            "provider/usage derived: {types:?}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backfill_derives_events_and_is_per_session_idempotent() {
        use crate::agent::turn_event::TurnEvent;
        let dir = unique_dir("backfill");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("backfill.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        // 构造带 history + trace 的会话
        let mut session = minimal_session("s1", "first", 1000);
        session.history = vec![
            crate::agent::session::TurnHistoryMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                attachments: Vec::new(),
                turn_id: Some("turn-1".to_string()),
                status: None,
                model_name: None,
                token_count: None,
                reasoning_content: None,
            },
            crate::agent::session::TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "hi".to_string(),
                attachments: Vec::new(),
                turn_id: Some("turn-1".to_string()),
                status: None,
                model_name: None,
                token_count: None,
                reasoning_content: None,
            },
        ];
        assert!(backend.upsert_session("s1", &session));

        let count = backend.backfill_turn_events().expect("backfill");
        assert_eq!(count, 1, "one session backfilled");

        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let rows: Vec<(i64, String)> = guard
            .prepare("SELECT seq, event_type FROM turn_events WHERE session_id = 's1' ORDER BY seq")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        // turn/start, user/message, assistant/message, turn/end
        assert_eq!(rows.len(), 4, "derived events");
        let types: Vec<&str> = rows.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(
            types,
            vec![
                "turn/start",
                "user/message",
                "assistant/message",
                "turn/end"
            ]
        );
        // assistant/message 带 chunk_missing
        let payload: String = guard
            .query_row(
                "SELECT payload FROM turn_events WHERE session_id = 's1' AND event_type = 'assistant/message'",
                [],
                |row| row.get(0),
            )
            .expect("payload");
        let decoded: TurnEvent = serde_json::from_str(&payload).expect("decode");
        match decoded {
            TurnEvent::AssistantMessage { chunk_missing, .. } => {
                assert_eq!(chunk_missing, Some(true), "chunk_missing marker");
            }
            _ => panic!("expected AssistantMessage"),
        }
        // guard 是 as_ref() 的引用句柄，drop 本就是 no-op；真正的锁由下方
        // drop(conn) / 作用域结束释放。
        let _ = guard;
        drop(conn);

        // 幂等：第二次回填跳过
        let count = backend.backfill_turn_events().expect("backfill again");
        assert_eq!(count, 0, "second run is no-op");
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let total: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM turn_events WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(total, 4, "no duplicate events");

        fs::remove_dir_all(&dir).ok();
    }

    /// PA-095：事件 schema 版本契约四分支——匹配 / 缺失 key（legacy 库视为 v1 并
    /// 回填）/ 不匹配 Err / 坏 payload fail loud（degraded 上抛，不静默跳过）。
    #[test]
    fn event_schema_version_contract_four_branches() {
        
        let dir = unique_dir("schema-version");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("schema-version.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        );

        // 分支 1：新建库 → ensure_schema 已写入当前版本 → 校验通过。
        assert_eq!(
            backend.validate_event_schema().expect("fresh store valid"),
            crate::agent::turn_event::EVENT_SCHEMA_VERSION,
            "fresh store matches current version"
        );

        // 分支 2：缺失 key（模拟 legacy 库）→ 删除后校验视为 v1 并回填。
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute(
                    "DELETE FROM store_metadata WHERE key = 'events.schema_version'",
                    [],
                )
                .expect("delete version key");
        }
        assert_eq!(
            backend.validate_event_schema().expect("legacy store valid"),
            crate::agent::turn_event::EVENT_SCHEMA_VERSION,
            "missing key treated as v1"
        );
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            let backfilled: String = guard
                .query_row(
                    "SELECT value FROM store_metadata WHERE key = 'events.schema_version'",
                    [],
                    |row| row.get(0),
                )
                .expect("backfilled");
            assert_eq!(
                backfilled,
                crate::agent::turn_event::EVENT_SCHEMA_VERSION.to_string(),
                "missing key backfilled on validate"
            );
        }

        // 分支 3：版本不匹配 → Err（数据不可用，非部分视图）。
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute(
                    "UPDATE store_metadata SET value = '999' WHERE key = 'events.schema_version'",
                    [],
                )
                .expect("bump to future version");
        }
        let mismatch = backend.validate_event_schema();
        assert!(mismatch.is_err(), "version mismatch must fail loud");
        assert!(
            mismatch.unwrap_err().contains("mismatch"),
            "error names the mismatch"
        );
        // 不匹配时 load_turn_events_checked 同样拒绝。
        assert!(backend.load_turn_events_checked("s1", None).is_err());

        // 恢复版本，写入一条坏 payload 事件。
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute(
                    "UPDATE store_metadata SET value = ?1 WHERE key = 'events.schema_version'",
                    params![crate::agent::turn_event::EVENT_SCHEMA_VERSION.to_string()],
                )
                .expect("restore version");
            guard
                .execute(
                    "INSERT INTO turn_events (session_id, turn_id, branch_id, seq, event_type, payload, created_at_ms)
                     VALUES ('s1', 't1', 'main', 0, 'alien/event', '{\"type\":\"alien/event\"}', 0)",
                    [],
                )
                .expect("insert malformed event");
        }

        // 分支 4：坏 payload → checked 加载 fail loud（Err 含定位信息）；
        // 兼容入口 load_turn_events 保持旧行为（跳过坏行）供 legacy 调用方过渡。
        let checked = backend.load_turn_events_checked("s1", None);
        assert!(checked.is_err(), "malformed payload must fail loud");
        assert!(
            checked.unwrap_err().contains("degraded"),
            "error marks stream degraded"
        );
        let unchecked = SessionBackend::load_turn_events(&backend, "s1", None);
        assert_eq!(unchecked.len(), 0, "legacy path skips bad row (transition)");

        // PA-095：SessionStore 层——坏 payload 上抛同时标记会话 degraded；
        // 正常会话不受影响。
        let store = crate::agent::session::SessionStore::with_backend(Box::new(
            SqliteSessionBackend::new_with_trace_mode(
                db_path.clone(),
                SeparateTraceTableMode::WriteSeparate,
            ),
        ));
        assert!(!store.is_event_stream_degraded("s1"), "clean before load");
        assert!(store.load_turn_events_checked("s1", None).is_err());
        assert!(
            store.is_event_stream_degraded("s1"),
            "session marked degraded after failed load"
        );
        assert!(
            !store.is_event_stream_degraded("other"),
            "unrelated session untouched"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// PA-094：大字段外置——flush ContextObservation 事件时，全量 payload 写入
    /// build_context_observations 表，事件落盘形态只含引用；load 返回与原始一致。
    #[test]
    fn flush_events_externalizes_context_observation() {
        use crate::agent::provider::BuildContextObservation;
        use crate::agent::turn_event::{TurnEndReason, TurnEvent};
        let dir = unique_dir("flush-bco");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("flush-bco.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        let observation = BuildContextObservation {
            request_format: "chat".into(),
            message_count: 3,
            image_count: 0,
            tool_count: 2,
            temperature: 0.7,
            max_output_tokens: 4096,
            stable_prefix_text: "stable-prefix".into(),
            semi_stable_context_text: "semi".into(),
            volatile_input_text: "volatile".into(),
            prefix_mutation_reasons: Vec::new(),
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".into(),
            tool_definitions_text: "tools".into(),
        };
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            branch_id: "main".to_string(),
            events: vec![
                TurnEvent::TurnStart {
                    turn_id: "turn-1".into(),
                },
                TurnEvent::ContextObservation {
                    turn_id: "turn-1".into(),
                    step: 0,
                    observation: Some(observation.clone()),
                    observation_ref: None,
                },
                TurnEvent::TurnEnd {
                    turn_id: "turn-1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: None,
                },
            ],
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        // 事件落盘形态只含引用（ref = bco:turn-1:1，seq 1）。
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let payload: String = guard
            .query_row(
                "SELECT payload FROM turn_events
                 WHERE session_id = 's1' AND event_type = 'context/observation'",
                [],
                |row| row.get(0),
            )
            .expect("event payload");
        assert!(
            !payload.contains("requestFormat"),
            "event payload must not embed full observation: {payload}"
        );
        assert!(payload.contains("bco:turn-1:1"), "ref form: {payload}");
        let decoded: TurnEvent = serde_json::from_str(&payload).expect("decode");
        match decoded {
            TurnEvent::ContextObservation {
                observation,
                observation_ref,
                ..
            } => {
                assert!(observation.is_none(), "no payload in ref form");
                assert_eq!(observation_ref.as_deref(), Some("bco:turn-1:1"));
            }
            _ => panic!("expected ContextObservation"),
        }
        // 全量 payload 在独立表。
        let stored: String = guard
            .query_row(
                "SELECT payload_json FROM build_context_observations
                 WHERE session_id = 's1' AND observation_ref = 'bco:turn-1:1'",
                [],
                |row| row.get(0),
            )
            .expect("stored payload");
        let stored_obs: BuildContextObservation =
            serde_json::from_str(&stored).expect("decode stored");
        assert_eq!(
            serde_json::to_string(&stored_obs).expect("serialize stored"),
            serde_json::to_string(&observation).expect("serialize original"),
            "load returns payload identical to original"
        );
        // guard 是 as_ref() 的引用句柄，drop 本就是 no-op；真正的锁由下方
        // drop(conn) / 作用域结束释放。
        let _ = guard;
        drop(conn);

        // host command 后端：按引用加载返回与原始一致。
        let loaded = backend
            .load_build_context_observation("s1", "bco:turn-1:1")
            .expect("load by ref");
        assert_eq!(
            serde_json::to_string(&loaded).expect("serialize loaded"),
            serde_json::to_string(&observation).expect("serialize original"),
            "load_build_context_observation returns original payload"
        );
        // 未命中返回 None。
        assert!(backend
            .load_build_context_observation("s1", "bco:missing:9")
            .is_none());

        fs::remove_dir_all(&dir).ok();
    }

    /// PA-095 #7：trace cache ref-only——live 写入剥离 observation payload；
    /// flush 同事务把外置引用回填 trace 行；legacy 内嵌数据读取兼容不变。
    #[test]
    fn pa095_ref_only_trace_rows_backfill_and_legacy_reads() {
        use crate::agent::provider::BuildContextObservation;
        use crate::agent::session::{
            SessionStore, TraceMigrationState, TurnTraceRecord,
        };
        use crate::agent::turn_event::TurnEvent;
        let dir = unique_dir("pa095-ref-only");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("ref-only.db");

        let observation = BuildContextObservation {
            request_format: "chat".into(),
            message_count: 2,
            image_count: 0,
            tool_count: 1,
            temperature: 0.7,
            max_output_tokens: 4096,
            stable_prefix_text: "stable".into(),
            semi_stable_context_text: String::new(),
            volatile_input_text: "input".into(),
            prefix_mutation_reasons: Vec::new(),
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".into(),
            tool_definitions_text: "tools".into(),
        };

        // Authoritative 会话种子（与 flush_events_externalizes_context_observation 一致）。
        {
            let backend = SqliteSessionBackend::new_with_trace_mode(
                db_path.clone(),
                SeparateTraceTableMode::WriteSeparate,
            );
            let mut session = minimal_session("s9", "first", 1000);
            session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
            assert!(backend.upsert_session("s9", &session));
        }

        // live 写入：trace 携带全量 payload → 缓存行剥离。
        let mut store = SessionStore::with_backend(Box::new(
            SqliteSessionBackend::new_with_trace_mode(
                db_path.clone(),
                SeparateTraceTableMode::WriteSeparate,
            ),
        ));
        store.record_turn_trace(
            Some("s9"),
            TurnTraceRecord {
                turn_id: "t1".to_string(),
                title: "t1".to_string(),
                phase: "completed".to_string(),
                build_context_observation: Some(observation.clone()),
                ..Default::default()
            },
        );

        let raw_conn = rusqlite::Connection::open(&db_path).expect("open raw");
        let trace_data: String = raw_conn
            .query_row(
                "SELECT trace_data FROM session_turn_traces WHERE session_id = 's9' AND turn_id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("session_turn_traces row");
        assert!(
            !trace_data.contains("requestFormat"),
            "live trace_data must not embed observation payload: {trace_data}"
        );
        let raw_json: String = raw_conn
            .query_row(
                "SELECT raw_json FROM normalized_turn_traces WHERE session_id = 's9' AND turn_id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("normalized_turn_traces row (Authoritative dual-write)");
        assert!(
            !raw_json.contains("requestFormat"),
            "live raw_json must not embed observation payload: {raw_json}"
        );

        // flush 外置事件 → 同事务回填 ref 到 trace 行。
        store.persist_events(
            "s9",
            "t1",
            "main",
            vec![TurnEvent::ContextObservation {
                turn_id: "t1".into(),
                step: 0,
                observation: Some(observation.clone()),
                observation_ref: None,
            }],
        );
        let trace_data_after: String = raw_conn
            .query_row(
                "SELECT trace_data FROM session_turn_traces WHERE session_id = 's9' AND turn_id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("trace row after flush");
        assert!(
            trace_data_after.contains("\"buildContextObservationRef\":\"bco:t1:"),
            "flush backfills observation ref into trace row: {trace_data_after}"
        );
        assert!(
            !trace_data_after.contains("requestFormat"),
            "backfilled row stays payload-free: {trace_data_after}"
        );

        // 引用加载与原始一致。
        drop(raw_conn);
        let reader = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        );
        let reference = {
            let conn = reader.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            let reference: String = guard
                .query_row(
                    "SELECT observation_ref FROM build_context_observations WHERE session_id = 's9'",
                    [],
                    |row| row.get(0),
                )
                .expect("observation ref");
            reference
        };
        let loaded = reader
            .load_build_context_observation("s9", &reference)
            .expect("load by ref");
        assert_eq!(
            serde_json::to_string(&loaded).expect("serialize loaded"),
            serde_json::to_string(&observation).expect("serialize original"),
            "ref load matches original payload"
        );

        // 读侧水合（实施后审核 P1）：ref-only 行读回时按 ref 还原 payload——
        // "读路径优先 ref、缺失回退内嵌"承诺的落实验证。
        {
            let mut hydration_store = SessionStore::with_backend(Box::new(
                SqliteSessionBackend::new_with_trace_mode(
                    db_path.clone(),
                    SeparateTraceTableMode::WriteSeparate,
                ),
            ));
            let hydration_snapshot = hydration_store.snapshot(Some("s9"), &[]);
            let hydrated_trace = hydration_snapshot
                .turn_trace_history
                .iter()
                .find(|trace| trace.turn_id == "t1")
                .expect("t1 trace loads");
            let hydrated_observation = hydrated_trace
                .build_context_observation
                .as_ref()
                .expect("ref-only row hydrates observation on read");
            assert_eq!(
                serde_json::to_string(hydrated_observation).expect("serialize hydrated"),
                serde_json::to_string(&observation).expect("serialize original"),
                "hydration returns original payload"
            );
        }

        // legacy 内嵌数据仍可读：直接插入 legacy 形态行，重载后保留内嵌 observation。
        {
            let conn = reader.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            let legacy_json = format!(
                r#"{{"turnId":"t-legacy","title":"legacy","phase":"completed","buildContextObservation":{}}}"#,
                serde_json::to_string(&observation).expect("serialize legacy obs")
            );
            guard
                .execute(
                    "INSERT OR REPLACE INTO session_turn_traces
                     (session_id, turn_id, updated_at_ms, trace_order, trace_data)
                     VALUES ('s9', 't-legacy', 1001, 1, ?1)",
                    [&legacy_json],
                )
                .expect("insert legacy row");
        }
        // legacy 内嵌数据仍可读：legacy 形态行（含 buildContextObservation 全量
        // payload）必须能被读路径反序列化（serde 兼容；水合只对 ref-only 行生效，
        // 已有内嵌 payload 的行原样保留）。
        {
            let conn = reader.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            let legacy_raw: String = guard
                .query_row(
                    "SELECT trace_data FROM session_turn_traces WHERE session_id = 's9' AND turn_id = 't-legacy'",
                    [],
                    |row| row.get(0),
                )
                .expect("legacy row present");
            let legacy_record: TurnTraceRecord =
                serde_json::from_str(&legacy_raw).expect("legacy row deserializes");
            let legacy_observation = legacy_record
                .build_context_observation
                .as_ref()
                .expect("legacy embedded observation preserved");
            assert_eq!(legacy_observation.request_format, "chat");
        }

        fs::remove_dir_all(&dir).ok();
    }

    /// PA-094：trace 表降级为投影缓存——行带 seq 水位；清表后从事件全量重建，
    /// 与事件折叠结果一致（事件权威；时钟/序列字段豁免）。
    #[test]
    fn trace_cache_watermark_and_clear_rebuild() {
        use crate::agent::projection::{fold_all, TraceProjectionState};
        use crate::agent::turn_event::{TurnEndReason, TurnEvent};
        let dir = unique_dir("trace-cache-rebuild");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-cache-rebuild.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        // 事件序列（真实 turn 形态：start → chunk → tool → result → end）。
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "turn-1".into(),
                },
            ),
            (
                1,
                TurnEvent::AssistantChunk {
                    turn_id: "turn-1".into(),
                    step: 0,
                    text: "hi".into(),
                },
            ),
            (
                2,
                TurnEvent::ToolCall {
                    turn_id: "turn-1".into(),
                    step: 1,
                    call_id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                    started_at_ms: None,
                },
            ),
            (
                3,
                TurnEvent::ToolResult {
                    turn_id: "turn-1".into(),
                    step: 1,
                    call_id: "c1".into(),
                    result: Some("ok".into()),
                    error: None,
                    status: Some("done".into()),
                    duration_ms: Some(500),
                    artifacts: None,
                    capability_invocation: None,
                },
            ),
            (
                4,
                TurnEvent::TurnEnd {
                    turn_id: "turn-1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(800),
                },
            ),
        ];
        let flush_events: Vec<TurnEvent> = events.iter().map(|(_, e)| e.clone()).collect();
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            branch_id: "main".to_string(),
            events: flush_events,
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        // 事件权威：折叠事件得到期望 trace（含 timeline 折叠产物）。
        let expected = fold_all::<_, TraceProjectionState>(&events);
        let expected_trace = expected.trace_for_turn("turn-1").expect("expected trace");

        // 写入缓存行（投影缓存 upsert，带 seq 水位 = 终态事件 seq）。
        let mut cache_trace = expected_trace.clone();
        cache_trace.sequence = Some(4);
        let outcome = backend.persist_command(PersistCommand::AppendTrace {
            epoch: 1,
            session_id: "s1".to_string(),
            trace: cache_trace.clone(),
            trace_order: 0,
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        // 行带 seq 水位。
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let watermark: i64 = guard
            .query_row(
                "SELECT event_watermark FROM normalized_turn_traces
                 WHERE session_id = 's1' AND turn_id = 'turn-1'",
                [],
                |row| row.get(0),
            )
            .expect("watermark");
        assert_eq!(watermark, 4, "cache row carries fold watermark");
        // guard 是 as_ref() 的引用句柄，drop 本就是 no-op；真正的锁由下方
        // drop(conn) / 作用域结束释放。
        let _ = guard;
        drop(conn);

        // 清表重建：清空 trace 表 → 从事件全量重建 → 与事件折叠一致（事件权威）。
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute("DELETE FROM normalized_turn_traces", [])
                .expect("clear trace cache");
            guard
                .execute("DELETE FROM session_turn_traces", [])
                .expect("clear legacy trace cache");
        }
        let rebuilt = fold_all::<_, TraceProjectionState>(&events);
        let rebuilt_trace = rebuilt.trace_for_turn("turn-1").expect("rebuilt trace");
        // 事件权威断言：重建 == 事件折叠（timeline/tool_activities/token 指标）。
        assert_eq!(
            serde_json::to_string(&rebuilt_trace.trace_timeline).expect("timeline json"),
            serde_json::to_string(&expected_trace.trace_timeline).expect("expected timeline json"),
            "rebuilt timeline equals event fold"
        );
        assert_eq!(
            serde_json::to_string(&rebuilt_trace.tool_activities).expect("tools json"),
            serde_json::to_string(&expected_trace.tool_activities).expect("expected tools json"),
            "rebuilt tool_activities equals event fold"
        );
        assert_eq!(
            rebuilt_trace.turn_duration_ms,
            expected_trace.turn_duration_ms
        );
        assert_eq!(rebuilt_trace.event_type, expected_trace.event_type);
        assert_eq!(rebuilt_trace.sequence, expected_trace.sequence);
        // 豁免清单：updated_at/event_id/emitted_at_ms 不参与重建断言
        // （时钟/序列语义，design.md §5 显式豁免）。
        assert_eq!(
            rebuilt_trace.updated_at, 0,
            "rebuilt trace has no clock fields"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// PA-094（审核 P0）：backend 真实清表重建——清空 trace 表后经
    /// SessionStore::with_backend 加载，trace 从事件全量折叠重建（事件权威，
    /// spec 1a 的 Cache rebuild 场景走真实加载路径，非内存平凡断言）。
    #[test]
    fn trace_cache_clear_rebuilds_from_events_on_store_load() {
        use crate::agent::projection::{fold_all, TraceProjectionState};
        use crate::agent::session::SessionStore;
        use crate::agent::turn_event::{TurnEndReason, TurnEvent};
        let dir = unique_dir("trace-clear-rebuild-load");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("trace-clear-rebuild-load.db");
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut session = minimal_session("s1", "first", 1000);
        session.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        assert!(backend.upsert_session("s1", &session));

        // 事件序列（真实 turn 形态）。
        let events: Vec<(u64, TurnEvent)> = vec![
            (
                0,
                TurnEvent::TurnStart {
                    turn_id: "turn-1".into(),
                },
            ),
            (
                1,
                TurnEvent::AssistantChunk {
                    turn_id: "turn-1".into(),
                    step: 0,
                    text: "hi".into(),
                },
            ),
            (
                2,
                TurnEvent::ToolCall {
                    turn_id: "turn-1".into(),
                    step: 3,
                    call_id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                    started_at_ms: None,
                },
            ),
            (
                3,
                TurnEvent::ToolResult {
                    turn_id: "turn-1".into(),
                    step: 5,
                    call_id: "c1".into(),
                    result: Some("ok".into()),
                    error: None,
                    status: Some("done".into()),
                    duration_ms: Some(500),
                    artifacts: None,
                    capability_invocation: None,
                },
            ),
            (
                4,
                TurnEvent::TurnEnd {
                    turn_id: "turn-1".into(),
                    reason: TurnEndReason::Completed,
                    turn_duration_ms: Some(800),
                },
            ),
        ];
        let flush_events: Vec<TurnEvent> = events.iter().map(|(_, e)| e.clone()).collect();
        let outcome = backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: "s1".to_string(),
            turn_id: "turn-1".to_string(),
            branch_id: "main".to_string(),
            events: flush_events,
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        // 写入缓存行（模拟正常运行的投影缓存）。
        let expected = fold_all::<_, TraceProjectionState>(&events);
        let expected_trace = expected.trace_for_turn("turn-1").expect("expected trace");
        let mut cache_trace = expected_trace.clone();
        cache_trace.sequence = Some(4);
        let outcome = backend.persist_command(PersistCommand::AppendTrace {
            epoch: 1,
            session_id: "s1".to_string(),
            trace: cache_trace.clone(),
            trace_order: 0,
        });
        assert_eq!(outcome, PersistCommandOutcome::Succeeded);

        // 清空 trace 表（缓存可丢弃——事件权威）。
        {
            let conn = backend.connection().expect("connection");
            let guard = conn.as_ref().expect("initialized");
            guard
                .execute("DELETE FROM normalized_turn_traces", [])
                .expect("clear trace cache");
            guard
                .execute("DELETE FROM session_turn_traces", [])
                .expect("clear legacy trace cache");
        }

        // 真实加载路径：SessionStore::with_backend → 检测 trace 缓存为空 +
        // 事件存在 → 从事件全量折叠重建。
        let mut store = SessionStore::with_backend(Box::new(backend));
        let snapshot = store.snapshot_at(Some("s1"), None, &[]);
        assert_eq!(
            snapshot.turn_trace_history.len(),
            1,
            "trace rebuilt on load"
        );
        let loaded_trace = &snapshot.turn_trace_history[0];
        assert_eq!(loaded_trace.turn_id, "turn-1");
        // 与事件折叠一致（timeline/tool_activities/token 指标）。
        assert_eq!(
            serde_json::to_string(&loaded_trace.trace_timeline).expect("timeline json"),
            serde_json::to_string(&expected_trace.trace_timeline).expect("expected timeline json"),
            "rebuilt timeline equals event fold"
        );
        assert_eq!(
            serde_json::to_string(&loaded_trace.tool_activities).expect("tools json"),
            serde_json::to_string(&expected_trace.tool_activities).expect("expected tools json"),
            "rebuilt tool_activities equals event fold"
        );
        assert_eq!(
            loaded_trace.turn_duration_ms,
            expected_trace.turn_duration_ms
        );
        assert_eq!(loaded_trace.event_type, expected_trace.event_type);
        // 豁免清单：时钟/序列字段不参与重建断言。
        assert_eq!(
            loaded_trace.updated_at, 0,
            "rebuilt trace has no clock fields"
        );

        fs::remove_dir_all(&dir).ok();
    }
}
