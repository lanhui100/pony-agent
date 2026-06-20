use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use super::session::{FileSessionBackend, PersistedStore, SessionBackend, SessionState};

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
    /// Path to the legacy JSON file, used for one-time migration.
    legacy_json_path: Option<PathBuf>,
    /// Lazily opened, then reused for the lifetime of the backend.
    connection: Mutex<Option<Connection>>,
}

impl SqliteSessionBackend {
    pub fn new(db_path: PathBuf) -> Self {
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
            );",
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
                let data = serde_json::to_string(session)
                    .map_err(|e| format!("serialize session: {e}"))?;
                stmt.execute(params![
                    id,
                    session.title,
                    session.updated_at_ms as i64,
                    data
                ])
                .map_err(|e| format!("insert session: {e}"))?;
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
                for id in removed_ids {
                    delete_stmt
                        .execute(params![id])
                        .map_err(|e| format!("delete session: {e}"))?;
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
                let session: SessionState = serde_json::from_str(&data).ok()?;
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
    use super::super::session::HistoryCursor;
    use super::*;
    use std::fs;

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

        let backend = SqliteSessionBackend::new(db_path.clone());
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

        let backend = SqliteSessionBackend::new(db_path);

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
}
