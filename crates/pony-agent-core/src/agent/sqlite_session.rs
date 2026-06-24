use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};

use super::session::{
    AttachmentAsset, FileSessionBackend, PersistedStore, SeparateTraceTableMode,
    SessionBackend, SessionBackendMutationResult, SessionBackendTraceLoadResult,
    SessionState, SessionTraceMutation, TraceMigrationState, TurnTraceRecord,
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
                ON session_turn_traces (session_id, trace_order, updated_at_ms);",
        )
        .map_err(|e| format!("schema: {e}"))?;
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
                if !matches!(
                    normalized_session.trace_migration_state,
                    TraceMigrationState::TraceTableAuthoritative
                ) {
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
            let metadata_entries: [(&str, Option<String>); 4] = [
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
            params![session_id, session.title, session.updated_at_ms as i64, data],
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

    fn merge_trace_history(
        &self,
        session: &SessionState,
        table_traces: Vec<TurnTraceRecord>,
    ) -> Vec<TurnTraceRecord> {
        match session.trace_migration_state {
            TraceMigrationState::TraceTableAuthoritative => table_traces,
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
}

impl SessionBackend for SqliteSessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        eprintln!(
            "[pony-agent][session] loading sessions from SQLite {}",
            self.db_path.display()
        );

        // Initialize (and migrate) the pooled connection, then borrow it.
        let mut slot = self.connection().ok()?;
        let conn = slot.as_mut().expect("connection initialized");

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
                session.turn_trace_history = self.merge_trace_history(&session, table_traces);
                Some((id, session))
            })
            .collect();

        let attachment_assets = self.read_metadata(conn, "attachment_assets");
        let session_attachment_index = self.read_metadata(conn, "session_attachment_index");
        let mcp_source_snapshots = self.read_metadata(conn, "mcp_source_snapshots");
        let skill_source_snapshots = self.read_metadata(conn, "skill_source_snapshots");

        Some(PersistedStore {
            sessions,
            attachment_assets,
            session_attachment_index,
            mcp_source_snapshots,
            skill_source_snapshots,
        })
    }

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
            params![session_id, session.title, session.updated_at_ms as i64, data],
        ) {
            eprintln!("[pony-agent][session] SQLite upsert session error: {e}");
            return false;
        }
        if !matches!(
            session.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        ) {
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
                        eprintln!("[pony-agent][session] SQLite metadata delete-upsert error: {error}");
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
            matches!(self.trace_mode, SeparateTraceTableMode::WriteSeparate),
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
            trace.hook_trace_records.extend(hook_trace_records.iter().cloned());
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
            SessionTraceMutation::ReplaceAll { traces } => {
                self.replace_session_traces_tx(&tx, session_id, &traces)
                    .map(|_| SessionBackendMutationResult::Succeeded)
            }
            SessionTraceMutation::UpsertOne { trace, trace_order } => {
                self.upsert_turn_trace_tx(
                    &tx,
                    session_id,
                    &trace,
                    trace_order,
                    matches!(self.trace_mode, SeparateTraceTableMode::WriteSeparate)
                        && matches!(session.trace_migration_state, TraceMigrationState::TraceTableAuthoritative),
                )
                    .map(|_| SessionBackendMutationResult::Succeeded)
            }
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
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PonyAgent")
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

    fn minimal_session(id: &str, title: &str, updated_at_ms: u64) -> SessionState {
        SessionState {
            conversation_id: id.to_string(),
            title: title.to_string(),
            summary: String::new(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
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
        }
    }

    #[test]
    fn removes_sessions_missing_from_latest_store_snapshot() {
        let dir = unique_dir("delete");
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("delete.db");

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::DualWrite,
        );

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

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::DualWrite,
        );

        let mut store = PersistedStore::default();
        store
            .sessions
            .insert("s1".to_string(), minimal_session("s1", "first", 1000));
        store
            .sessions
            .insert("s2".to_string(), minimal_session("s2", "second", 2000));
        backend.save_store(&store);

        let mut updated = minimal_session("s1", "first-updated", 3000);
        updated.turn_trace_history.push(trace("turn-1", "trace title", 42));

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

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::DualWrite,
        );

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
        store.session_attachment_index.insert(
            "s1".to_string(),
            vec!["asset:s1/file.dataurl".to_string()],
        );
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

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::DualWrite,
        );

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

        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::DualWrite,
        );

        let mut session = minimal_session("s1", "first", 1000);
        session.turn_trace_history.push(trace("turn-1", "legacy", 1));

        let mut store = PersistedStore::default();
        store.sessions.insert("s1".to_string(), session.clone());
        backend.save_store(&store);

        let mut updated = session.clone();
        updated.trace_migration_state = TraceMigrationState::DualWrite;
        updated.turn_trace_history[0].title = "table".to_string();
        updated.turn_trace_history.push(trace("turn-2", "second", 2));
        assert!(backend.upsert_session("s1", &updated));

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), 2);
        assert_eq!(loaded.sessions["s1"].turn_trace_history[0].title, "table");
        assert_eq!(loaded.sessions["s1"].turn_trace_history[1].turn_id, "turn-2");

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
            params![
                "s1",
                serde_json::to_string(&authoritative_blob).unwrap()
            ],
        )
        .unwrap();
        drop(slot);

        let loaded = backend.load_store().unwrap();
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), 1);
        assert_eq!(loaded.sessions["s1"].turn_trace_history[0].turn_id, "turn-legacy");

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
        assert_eq!(loaded.sessions["s1"].turn_trace_history[0].title, "first-trace");

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

        store.record_turn_trace(
            Some("s1"),
            trace("turn-1", "first-trace", 11),
        );
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
        assert_eq!(snapshot.turn_trace_history[0].event_id.as_deref(), Some("turn-1:4"));
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
    fn write_separate_prunes_trace_table_to_history_limit() {
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
        assert_eq!(loaded.sessions["s1"].turn_trace_history.len(), SQLITE_TRACE_HISTORY_LIMIT);
        assert_eq!(loaded.sessions["s1"].turn_trace_history[0].turn_id, "turn-16");
        assert_eq!(loaded.sessions["s1"].turn_trace_history[23].turn_id, "turn-39");

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

        let (nodes_before, _, _) = store.load_history_graph(Some("switch-session"));
        let first_node_id = nodes_before[0].node_id.clone();
        let second_node_id = nodes_before[1].node_id.clone();

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
        assert_eq!(main_snapshot.resolved_node_id.as_deref(), Some(second_node_id.as_str()));
        assert_eq!(main_snapshot.turn_trace_history.len(), 2);
        assert_eq!(main_snapshot.turn_trace_history[1].turn_id, "turn-main-2");

        let restored = store
            .restore_branch_head(Some("switch-session"), Some(fork_branch_id.as_str()), None)
            .expect("restore fork branch head should succeed");
        assert_ne!(restored.resolved_node_id.as_deref(), Some(second_node_id.as_str()));

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

        let inspect_backend =
            SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::WriteSeparate);
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

        let inspect_backend =
            SqliteSessionBackend::new_with_trace_mode(db_path.clone(), SeparateTraceTableMode::WriteSeparate);
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
        assert_eq!(snapshot.turn_trace_history[0].event_id.as_deref(), Some("turn-1:4"));

        fs::remove_dir_all(&dir).ok();
    }
}
