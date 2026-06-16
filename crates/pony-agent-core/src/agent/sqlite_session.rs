use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{params, Connection};

use super::session::{
    FileSessionBackend, PersistedStore, SessionBackend, SessionState,
};

/// SQLite-backed session storage.
///
/// Stores each session as an individual row with a JSON blob, enabling:
/// - Incremental updates (only write changed sessions)
/// - Concurrent access safety via SQLite's WAL mode
/// - Future query capabilities for trace data, history graphs, etc.
///
/// Schema:
/// ```sql
/// sessions(conversation_id PK, title, updated_at_ms, session_data)
/// store_metadata(key PK, value)
/// ```
pub struct SqliteSessionBackend {
    db_path: PathBuf,
    attachment_root: PathBuf,
    /// Path to the legacy JSON file, used for one-time migration.
    legacy_json_path: Option<PathBuf>,
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
        }
    }

    fn open_connection(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.db_path).map_err(|e| format!("open db: {e}"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(|e| format!("pragma: {e}"))?;
        Ok(conn)
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
        // This is safe because we own the connection (opened fresh each time).
        let tx = conn.unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;

        // Upsert sessions — scoped to drop the statement before commit
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO sessions (conversation_id, title, updated_at_ms, session_data)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| format!("prepare session insert: {e}"))?;

            for (id, session) in &store.sessions {
                let data = serde_json::to_string(session).map_err(|e| format!("serialize session: {e}"))?;
                stmt.execute(params![id, session.title, session.updated_at_ms as i64, data])
                    .map_err(|e| format!("insert session: {e}"))?;
            }
        }

        // Remove sessions that are no longer in the store
        if store.sessions.is_empty() {
            tx.execute("DELETE FROM sessions", [])
                .map_err(|e| format!("clear sessions: {e}"))?;
        }

        // Upsert metadata — scoped to drop the statement before commit
        {
            let metadata_entries: [(&str, Option<String>); 4] = [
                ("attachment_assets", serde_json::to_string(&store.attachment_assets).ok()),
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

        let conn = self.open_connection().ok()?;
        self.ensure_schema(&conn).ok()?;
        self.migrate_from_json(&conn).ok()?;

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

        let attachment_assets = self.read_metadata(&conn, "attachment_assets");
        let session_attachment_index = self.read_metadata(&conn, "session_attachment_index");
        let mcp_source_snapshots = self.read_metadata(&conn, "mcp_source_snapshots");
        let skill_source_snapshots = self.read_metadata(&conn, "skill_source_snapshots");

        Some(PersistedStore {
            sessions,
            attachment_assets,
            session_attachment_index,
            mcp_source_snapshots,
            skill_source_snapshots,
        })
    }

    fn save_store(&self, store: &PersistedStore) {
        let Ok(conn) = self.open_connection() else {
            return;
        };
        if self.ensure_schema(&conn).is_err() {
            return;
        }

        eprintln!(
            "[pony-agent][session] saving {} sessions to SQLite {}",
            store.sessions.len(),
            self.db_path.display()
        );

        if let Err(e) = self.write_full_store(&conn, store) {
            eprintln!("[pony-agent][session] SQLite save error: {e}");
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
    use super::*;
    use std::fs;

    #[test]
    fn roundtrip_empty_store() {
        let dir = std::env::temp_dir().join(format!("sqlite-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");

        let backend = SqliteSessionBackend::new(db_path.clone());
        let store = PersistedStore::default();
        backend.save_store(&store);

        let loaded = backend.load_store().unwrap();
        assert!(loaded.sessions.is_empty());

        fs::remove_dir_all(&dir).ok();
    }
}
