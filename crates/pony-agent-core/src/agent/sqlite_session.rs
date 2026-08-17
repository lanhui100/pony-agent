use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};

use super::session::{
    AttachmentAsset, FileSessionBackend, HistoryBranch, HistoryCheckoutMode, HistoryCheckoutStatus,
    HistoryCursor, HistoryCursorMode, HistoryNode, HistoryNodeKind, PersistCommand,
    PersistCommandOutcome, PersistedStore, SeparateTraceTableMode, SessionBackend,
    SessionBackendMutationResult, SessionBackendTraceLoadResult, SessionState, SessionTraceMutation,
    TraceMigrationState, TurnHistoryMessage, TurnTraceRecord,
};

const SQLITE_TRACE_HISTORY_LIMIT: usize = 24;

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
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| format!("schema: {e}"))?;
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
                UNIQUE (session_id, ordinal),
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
                UNIQUE (session_id, ordinal),
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
             );",
        )
        .map_err(|e| format!("normalized schema: {e}"))?;
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
                    self.replace_session_traces_tx(&tx, id, &normalized_session.turn_trace_history)?;
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
                (
                    "workspaces",
                    serde_json::to_string(&store.workspaces).ok(),
                ),
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
                Ok(trace) => traces.push(trace),
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
    fn read_all_session_traces(&self, conn: &Connection) -> Result<HashMap<String, Vec<TurnTraceRecord>>, String> {
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
                    by_session.entry(session_id).or_default().push(trace);
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
                | PersistCommand::UpdateSessionMeta { session_id, .. }
                | PersistCommand::RemoveSession { session_id, .. } => session_id,
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
                let message_id = message.stable_id();
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
                        message.status.as_ref().map(|s| serde_json::to_string(s).unwrap_or_default()),
                        message.model_name,
                        message.token_count,
                        serde_json::to_string(&message.attachments).unwrap_or_else(|_| "[]".to_string()),
                    ],
                )
                .map_err(|e| format!("append message: {e}"))?;
            }
            PersistCommand::UpsertTurn {
                session_id,
                turn,
                ..
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
                let raw = serde_json::to_string(trace).map_err(|e| format!("serialize trace: {e}"))?;
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_turn_traces
                     (session_id, turn_id, trace_order, phase, provider_name, provider_model, provider_mode,
                      session_summary, fallback_reason, error, input_tokens, output_tokens, total_tokens,
                      first_token_latency_ms, turn_duration_ms, updated_at_ms, extension_json, raw_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, NULL, ?17)",
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
                     SET phase = COALESCE(?3, phase), updated_at_ms = ?4, extension_json = ?5
                     WHERE session_id = ?1 AND turn_id = ?2",
                    params![
                        session_id,
                        turn_id,
                        terminal_patch.phase,
                        terminal_patch.updated_at,
                        serde_json::to_string(&ext).unwrap_or_else(|_| "{}".to_string()),
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
                session_id,
                node,
                ..
            } => {
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_history_nodes
                     (session_id, node_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id,
                      turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms, snapshot_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL)",
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
                    ],
                )
                .map_err(|e| format!("update history node: {e}"))?;
            }
            PersistCommand::UpdateCursor {
                session_id,
                cursor,
                ..
            } => {
                tx.execute(
                    "INSERT OR REPLACE INTO normalized_history_cursor
                     (session_id, visible_node_id, active_branch_id, branch_head_node_id, workspace_node_id,
                      cursor_version, mode, checkout_mode, checkout_status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
                    ],
                )
                .map_err(|e| format!("update cursor: {e}"))?;
            }
            PersistCommand::UpdateSessionMeta {
                session_id,
                meta_patch,
                ..
            } => {
                tx.execute(
                    "UPDATE normalized_sessions
                     SET title = COALESCE(?2, title), summary = COALESCE(?3, summary),
                         turn_count = COALESCE(?4, turn_count),
                         last_referenced_file = COALESCE(?5, last_referenced_file),
                         updated_at_ms = COALESCE(?6, updated_at_ms),
                         state_version = state_version + 1
                     WHERE session_id = ?1",
                    params![
                        session_id,
                        meta_patch.title,
                        meta_patch.summary,
                        meta_patch.turn_count.map(|v| v as i64),
                        meta_patch.last_referenced_file,
                        meta_patch.updated_at_ms.map(|v| v as i64),
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
    fn sync_blob_trace_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        session_id: &str,
        trace: &TurnTraceRecord,
    ) -> Result<(), String> {
        let raw: Option<String> = tx
            .query_row(
                "SELECT session_data FROM sessions WHERE conversation_id = ?1",
                params![session_id],
                |row| row.get::<_, String>(0),
            )
            .ok();
        let Some(raw) = raw else {
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
            };
            let initial = serde_json::to_string(&session).map_err(|e| format!("serialize blob: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
                 VALUES (?1, '', ?2, ?3)",
                params![session_id, trace.updated_at as i64, initial],
            )
            .map_err(|e| format!("insert blob: {e}"))?;
            return Ok(());
        };
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
            .prepare("SELECT session_id, title, summary, turn_count, last_referenced_file, created_at_ms, updated_at_ms, state_version, trace_migration_state, turn_trace_refs_json, provider_native_transcript_json, history_state_evidence_json, memory_json, workspace_id FROM normalized_sessions")
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
            .prepare("SELECT session_id, node_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id, turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms, snapshot_json FROM normalized_history_nodes ORDER BY session_id")
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
            .prepare("SELECT session_id, visible_node_id, active_branch_id, branch_head_node_id, workspace_node_id, cursor_version, mode, checkout_mode, checkout_status FROM normalized_history_cursor ORDER BY session_id")
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
                ))
            })
            .ok()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();

        let mut sessions: HashMap<String, SessionState> = HashMap::new();
        for (session_id, (title, summary, turn_count, last_referenced_file, _created_at, updated_at_ms, state_version, trace_migration_state, turn_trace_refs_json, provider_native_transcript_json, history_state_evidence_json, memory_json, workspace_id)) in session_rows {
            let mut session = SessionState {
                conversation_id: session_id.clone(),
                title,
                summary,
                history: Vec::new(),
                provider_native_transcript: provider_native_transcript_json
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default(),
                turn_trace_history: Vec::new(),
                trace_migration_state: serde_json::from_str(&trace_migration_state).unwrap_or(TraceMigrationState::LegacyBlob),
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
                workspace_id,
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

            // messages → history（批量预取）
            let messages = all_messages
                .iter()
                .filter(|(sid, _, _, _, _, _, _, _)| sid == &session_id)
                .map(|(_, turn_id, content, reasoning_content, status, model_name, token_count, attachments_json)| {
                    (turn_id.clone(), content.clone(), reasoning_content.clone(), status.clone(), model_name.clone(), token_count.clone(), attachments_json.clone())
                })
                .collect::<Vec<_>>();
            for (turn_id, content, reasoning_content, status, model_name, token_count, attachments_json) in messages {
                session.history.push(TurnHistoryMessage {
                    role: "assistant".to_string(),
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
            // role 修正（上面简化用 assistant，这里重新按表读）
            let role_rows = conn
                .prepare("SELECT role, ordinal FROM normalized_messages WHERE session_id = ?1 ORDER BY ordinal")
                .ok()?
                .query_map(params![session_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
                .ok()?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            for (role, ordinal) in role_rows {
                if let Some(message) = session.history.get_mut(ordinal as usize) {
                    message.role = role;
                }
            }

            // traces → turn_trace_history（批量预取）
            let table_traces = all_traces.get(&session_id).cloned().unwrap_or_default();
            session.turn_trace_history = table_traces.clone();

            // history_nodes（批量预取）
            let node_rows = all_nodes
                .iter()
                .filter(|(sid, _, _, _, _, _, _, _, _, _, _, _, _, _)| sid == &session_id)
                .map(|(_, node_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id, turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms, snapshot_json)| {
                    (node_id.clone(), parent_node_id.clone(), branch_id.clone(), forked_from_node_id.clone(), kind.clone(), turn_id.clone(), turn_trace_refs_json.clone(), run_id.clone(), workspace_ref_json.clone(), summary.clone(), title.clone(), *created_at_ms, snapshot_json.clone())
                })
                .collect::<Vec<_>>();
            let full_table_by_id: HashMap<&str, &TurnTraceRecord> = table_traces
                .iter()
                .map(|trace| (trace.turn_id.as_str(), trace))
                .collect();
            for (node_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id, turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms, snapshot_json) in node_rows {
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
                };
                // snapshot_json → 节点快照
                if let Some(raw) = snapshot_json {
                    if let Ok(snap) = serde_json::from_str::<serde_json::Value>(&raw) {
                        node.history = snap.get("history").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                        node.provider_native_transcript = snap.get("providerNativeTranscript").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                        node.long_term_memory_entries = snap.get("longTermMemoryEntries").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                        node.memory_write_evidence = snap.get("memoryWriteEvidence").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                        node.memory_write_hook_trace_records = snap.get("memoryWriteHookTraceRecords").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                        node.turn_count = snap.get("turnCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        node.last_referenced_file = snap.get("lastReferencedFile").and_then(|v| v.as_str()).map(str::to_string);
                    }
                }
                // 节点 trace materialize（按 refs 从全量表）
                if let Some(refs) = &node.turn_trace_refs {
                    if refs.is_empty() {
                        node.turn_trace_history.clear();
                    } else {
                        node.turn_trace_history = refs
                            .iter()
                            .filter_map(|reference| full_table_by_id.get(reference.turn_id.as_str()).map(|t| (*t).clone()))
                            .collect();
                    }
                }
                session.history_nodes.push(node);
            }

            // history_branches（批量预取）
            session.history_branches = all_branches
                .iter()
                .filter(|(sid, _, _, _, _, _, _, _, _)| sid == &session_id)
                .map(|(_, branch_id, base_node_id, head_node_id, forked_from_branch_id, forked_from_node_id, label, created_at_ms, updated_at_ms)| HistoryBranch {
                    branch_id: branch_id.clone(),
                    session_id: session_id.clone(),
                    base_node_id: base_node_id.clone(),
                    head_node_id: head_node_id.clone(),
                    forked_from_branch_id: forked_from_branch_id.clone(),
                    forked_from_node_id: forked_from_node_id.clone(),
                    label: label.clone(),
                    created_at_ms: *created_at_ms as u64,
                    updated_at_ms: *updated_at_ms as u64,
                })
                .collect::<Vec<_>>();

            // history_cursor（批量预取）
            if let Some((_, visible_node_id, active_branch_id, branch_head_node_id, workspace_node_id, cursor_version, mode, checkout_mode, checkout_status)) = all_cursors
                .iter()
                .find(|(sid, _, _, _, _, _, _, _, _)| sid == &session_id)
            {
                session.history_cursor = HistoryCursor {
                    session_id: session_id.clone(),
                    visible_node_id: visible_node_id.clone(),
                    active_branch_id: active_branch_id.clone(),
                    branch_head_node_id: branch_head_node_id.clone(),
                    workspace_node_id: workspace_node_id.clone(),
                    cursor_version: *cursor_version as u64,
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
            let mut trace: serde_json::Value =
                serde_json::from_str(&normalized_raw).map_err(|e| format!("parse normalized trace: {e}"))?;
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
            let updated = serde_json::to_string(&trace).map_err(|e| format!("serialize trace: {e}"))?;
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
        let updated = serde_json::to_string(&trace).map_err(|e| format!("serialize legacy trace: {e}"))?;
        tx.execute(
            "UPDATE session_turn_traces SET trace_data = ?3, updated_at_ms = ?4 WHERE session_id = ?1 AND turn_id = ?2",
            params![session_id, turn_id, updated, updated_at as i64],
        )
        .map_err(|e| format!("update legacy trace: {e}"))?;
        Ok(())
    }
}

impl SessionBackend for SqliteSessionBackend {
    /// PA-089 阶段 3：规范化双写命令——统一事务写 blob（旧 sessions 表）+ normalized_* 表。
    /// 迁移 barrier：epoch 检查（旧 epoch 命令拒绝）。
    fn persist_command(&self, command: PersistCommand) -> PersistCommandOutcome {
        let slot = match self.connection() {
            Ok(slot) => slot,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite open error: {error}");
                return PersistCommandOutcome::Failed;
            }
        };
        let conn = slot.as_ref().expect("connection initialized");
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite command begin tx error: {error}");
                return PersistCommandOutcome::Failed;
            }
        };

        let result = self.apply_persist_command_tx(&tx, &command);
        match result {
            Ok(()) => {
                if let Err(error) = tx.commit() {
                    eprintln!("[pony-agent][session] SQLite command commit error: {error}");
                    return PersistCommandOutcome::Failed;
                }
                PersistCommandOutcome::Succeeded
            }
            Err(error) => {
                eprintln!("[pony-agent][session] SQLite command error: {error}");
                PersistCommandOutcome::Failed
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
            if let Err(e) = self.replace_session_traces_tx(&tx, session_id, &session.turn_trace_history)
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

    fn replace_session_traces(
        &self,
        session_id: &str,
        traces: &[TurnTraceRecord],
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
        assert_eq!(workspaces[0].id, crate::agent::workspace::DEFAULT_WORKSPACE_ID);

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
        store.workspaces.push(crate::agent::workspace::WorkspaceRecord {
            id: crate::agent::workspace::DEFAULT_WORKSPACE_ID.to_string(),
            name: "默认工作区".to_string(),
            root_path: root.display().to_string(),
        });
        backend.save_store(&store);

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(loaded.workspaces[0].id, crate::agent::workspace::DEFAULT_WORKSPACE_ID);

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
                ["turn-0".to_string(), "turn-4".to_string()].into_iter().collect();
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
        assert_eq!(outcome, PersistCommandOutcome::Failed, "old epoch should fail");

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
        assert_eq!(session.history_cursor.visible_node_id.as_deref(), Some("node-1"));

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
            guard.execute("ALTER TABLE sessions RENAME TO session_blobs", []).expect("rename");
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
        assert!(backend.upsert_session("s1", &session), "upsert via view should succeed");

        // remove_session 删 sessions 视图 → 应转发
        assert!(backend.remove_session("s1", &HashMap::new(), &HashMap::new(), &HashMap::new(), &HashMap::new()), "remove via view should succeed");

        // 验证 session_blobs 已被删除
        let conn = backend.connection().expect("connection");
        let guard = conn.as_ref().expect("initialized");
        let count: i64 = guard
            .query_row("SELECT COUNT(*) FROM session_blobs WHERE conversation_id = 's1'", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 0, "remove 应转发到 session_blobs");

        fs::remove_dir_all(&dir).ok();
    }
}
