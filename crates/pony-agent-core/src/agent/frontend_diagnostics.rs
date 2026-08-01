use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendTraceEvent {
    pub seq: u64,
    pub ts_wall_ms: i64,
    pub ts_perf_ms: f64,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub scope: String,
    pub category: String,
    pub name: String,
    pub kind: String,
    pub duration_ms: Option<f64>,
    pub stall_level: Option<String>,
    pub trigger_kind: Option<String>,
    pub data: Option<Value>,
    pub truncated: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendStallSnapshot {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub ts_wall_ms: i64,
    pub ts_perf_ms: f64,
    pub stall_level: String,
    pub trigger_kind: String,
    pub stall_gap_ms: i64,
    pub snapshot: Value,
    pub truncated: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendTraceQuery {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub from_wall_ms: Option<i64>,
    pub to_wall_ms: Option<i64>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendTraceQueryResult {
    pub events: Vec<FrontendTraceEvent>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendTraceExportPayload {
    pub format: String,
    pub file_name: String,
    pub content: String,
    pub truncated: bool,
    pub event_count: usize,
    pub snapshot_count: usize,
    pub from_wall_ms: Option<i64>,
    pub to_wall_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendTraceAppendCommand {
    pub events: Vec<FrontendTraceEvent>,
    #[serde(default)]
    pub stall_snapshots: Vec<FrontendStallSnapshot>,
}

pub struct FrontendDiagnosticsStore {
    db_path: PathBuf,
    connection: Mutex<Option<Connection>>,
}

impl FrontendDiagnosticsStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            connection: Mutex::new(None),
        }
    }

    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Option<Connection>>, String> {
        let mut slot = self.connection.lock().map_err(|e| format!("lock: {e}"))?;
        if slot.is_none() {
            let conn = Connection::open(&self.db_path).map_err(|e| format!("open db: {e}"))?;
            conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=5000;
                 PRAGMA wal_autocheckpoint=1000;",
            )
            .map_err(|e| format!("pragma: {e}"))?;
            self.ensure_schema(&conn)?;
            *slot = Some(conn);
        }
        Ok(slot)
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS frontend_trace_event (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NULL,
                turn_id TEXT NULL,
                ts_wall_ms INTEGER NOT NULL,
                ts_perf_ms REAL NOT NULL,
                seq INTEGER NOT NULL,
                scope TEXT NOT NULL,
                category TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                duration_ms REAL NULL,
                stall_level TEXT NULL,
                trigger_kind TEXT NULL,
                data_json TEXT NULL,
                truncated INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_frontend_trace_session_ts
              ON frontend_trace_event(session_id, ts_wall_ms, seq);
            CREATE INDEX IF NOT EXISTS idx_frontend_trace_turn_ts
              ON frontend_trace_event(turn_id, ts_wall_ms, seq);

            CREATE TABLE IF NOT EXISTS frontend_stall_snapshot (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NULL,
                turn_id TEXT NULL,
                ts_wall_ms INTEGER NOT NULL,
                ts_perf_ms REAL NOT NULL,
                stall_level TEXT NOT NULL,
                trigger_kind TEXT NOT NULL,
                stall_gap_ms INTEGER NOT NULL,
                snapshot_json TEXT NOT NULL,
                truncated INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_frontend_stall_session_ts
              ON frontend_stall_snapshot(session_id, ts_wall_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_frontend_stall_turn_ts
              ON frontend_stall_snapshot(turn_id, ts_wall_ms DESC);",
        )
        .map_err(|e| format!("schema: {e}"))?;
        Ok(())
    }

    pub fn append(&self, command: FrontendTraceAppendCommand) -> Result<(), String> {
        let mut slot = self.connection()?;
        let conn = slot.as_mut().expect("connection initialized");
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO frontend_trace_event
                    (session_id, turn_id, ts_wall_ms, ts_perf_ms, seq, scope, category, name, kind, duration_ms, stall_level, trigger_kind, data_json, truncated)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                )
                .map_err(|e| format!("prepare trace insert: {e}"))?;
            for event in command.events {
                let data_json = event.data.map(|value| {
                    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
                });
                stmt.execute(params![
                    event.session_id,
                    event.turn_id,
                    event.ts_wall_ms,
                    event.ts_perf_ms,
                    event.seq as i64,
                    event.scope,
                    event.category,
                    event.name,
                    event.kind,
                    event.duration_ms,
                    event.stall_level,
                    event.trigger_kind,
                    data_json,
                    if event.truncated.unwrap_or(false) {
                        1
                    } else {
                        0
                    }
                ])
                .map_err(|e| format!("insert trace event: {e}"))?;
            }
        }
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO frontend_stall_snapshot
                    (session_id, turn_id, ts_wall_ms, ts_perf_ms, stall_level, trigger_kind, stall_gap_ms, snapshot_json, truncated)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .map_err(|e| format!("prepare snapshot insert: {e}"))?;
            for snapshot in command.stall_snapshots {
                stmt.execute(params![
                    snapshot.session_id,
                    snapshot.turn_id,
                    snapshot.ts_wall_ms,
                    snapshot.ts_perf_ms,
                    snapshot.stall_level,
                    snapshot.trigger_kind,
                    snapshot.stall_gap_ms,
                    serde_json::to_string(&snapshot.snapshot).unwrap_or_else(|_| "{}".to_string()),
                    if snapshot.truncated.unwrap_or(false) {
                        1
                    } else {
                        0
                    }
                ])
                .map_err(|e| format!("insert stall snapshot: {e}"))?;
            }
        }
        tx.commit().map_err(|e| format!("commit: {e}"))?;
        drop(slot);
        self.clear_before(now_ms() - retention_window_ms())?;
        Ok(())
    }

    pub fn query_window(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<FrontendTraceQueryResult, String> {
        let limit = query.limit.unwrap_or(500).clamp(1, 5000);
        let offset = query
            .cursor
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let mut slot = self.connection()?;
        let conn = slot.as_mut().expect("connection initialized");
        let sql = String::from(
            "SELECT session_id, turn_id, ts_wall_ms, ts_perf_ms, seq, scope, category, name, kind, duration_ms, stall_level, trigger_kind, data_json, truncated
             FROM frontend_trace_event
             WHERE (?1 IS NULL OR session_id = ?1)
               AND (?2 IS NULL OR turn_id = ?2)
               AND (?3 IS NULL OR ts_wall_ms >= ?3)
               AND (?4 IS NULL OR ts_wall_ms <= ?4)
             ORDER BY ts_wall_ms ASC, seq ASC
             LIMIT ?5 OFFSET ?6",
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare query: {e}"))?;
        let rows = stmt
            .query_map(
                params![
                    query.session_id,
                    query.turn_id,
                    query.from_wall_ms,
                    query.to_wall_ms,
                    limit,
                    offset
                ],
                |row| {
                    let data_json: Option<String> = row.get(12)?;
                    Ok(FrontendTraceEvent {
                        seq: row.get::<_, i64>(4)? as u64,
                        ts_wall_ms: row.get(2)?,
                        ts_perf_ms: row.get(3)?,
                        session_id: row.get(0)?,
                        turn_id: row.get(1)?,
                        scope: row.get(5)?,
                        category: row.get(6)?,
                        name: row.get(7)?,
                        kind: row.get(8)?,
                        duration_ms: row.get(9)?,
                        stall_level: row.get(10)?,
                        trigger_kind: row.get(11)?,
                        data: data_json
                            .and_then(|value| serde_json::from_str::<Value>(&value).ok()),
                        truncated: Some(row.get::<_, i64>(13)? != 0),
                    })
                },
            )
            .map_err(|e| format!("query rows: {e}"))?;
        let events: Vec<FrontendTraceEvent> = rows.filter_map(Result::ok).collect();
        let has_more = events.len() as u32 == limit;
        let next_cursor = if has_more {
            Some((offset + limit).to_string())
        } else {
            None
        };
        Ok(FrontendTraceQueryResult {
            events,
            next_cursor,
            has_more,
            limit,
        })
    }

    pub fn query_stall_snapshots(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<Vec<FrontendStallSnapshot>, String> {
        let limit = query.limit.unwrap_or(100).clamp(1, 1000);
        let mut slot = self.connection()?;
        let conn = slot.as_mut().expect("connection initialized");
        let mut stmt = conn
            .prepare(
                "SELECT session_id, turn_id, ts_wall_ms, ts_perf_ms, stall_level, trigger_kind, stall_gap_ms, snapshot_json, truncated
                 FROM frontend_stall_snapshot
                 WHERE (?1 IS NULL OR session_id = ?1)
                   AND (?2 IS NULL OR turn_id = ?2)
                   AND (?3 IS NULL OR ts_wall_ms >= ?3)
                   AND (?4 IS NULL OR ts_wall_ms <= ?4)
                 ORDER BY ts_wall_ms DESC
                 LIMIT ?5",
            )
            .map_err(|e| format!("prepare stall query: {e}"))?;
        let rows = stmt
            .query_map(
                params![
                    query.session_id,
                    query.turn_id,
                    query.from_wall_ms,
                    query.to_wall_ms,
                    limit
                ],
                |row| {
                    let snapshot_json: String = row.get(7)?;
                    Ok(FrontendStallSnapshot {
                        session_id: row.get(0)?,
                        turn_id: row.get(1)?,
                        ts_wall_ms: row.get(2)?,
                        ts_perf_ms: row.get(3)?,
                        stall_level: row.get(4)?,
                        trigger_kind: row.get(5)?,
                        stall_gap_ms: row.get(6)?,
                        snapshot: serde_json::from_str(&snapshot_json)
                            .unwrap_or_else(|_| json!({})),
                        truncated: Some(row.get::<_, i64>(8)? != 0),
                    })
                },
            )
            .map_err(|e| format!("query stall rows: {e}"))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn export_json(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<FrontendTraceExportPayload, String> {
        let events = self.query_window(FrontendTraceQuery {
            limit: Some(query.limit.unwrap_or(5000)),
            cursor: None,
            ..query.clone()
        })?;
        let snapshots = self.query_stall_snapshots(query.clone())?;
        let content = serde_json::to_string_pretty(&json!({
            "events": events.events,
            "stallSnapshots": snapshots
        }))
        .map_err(|e| format!("serialize json export: {e}"))?;
        Ok(FrontendTraceExportPayload {
            format: "json".to_string(),
            file_name: build_export_file_name("json", &query.session_id),
            content,
            truncated: events.has_more,
            event_count: events.events.len(),
            snapshot_count: snapshots.len(),
            from_wall_ms: query.from_wall_ms,
            to_wall_ms: query.to_wall_ms,
        })
    }

    pub fn export_chrome_trace(
        &self,
        query: FrontendTraceQuery,
    ) -> Result<FrontendTraceExportPayload, String> {
        let events = self.query_window(FrontendTraceQuery {
            limit: Some(query.limit.unwrap_or(5000)),
            cursor: None,
            ..query.clone()
        })?;
        let trace_origin_wall_ms = events
            .events
            .first()
            .map(|event| event.ts_wall_ms)
            .unwrap_or(0);
        let trace_events: Vec<Value> = events
            .events
            .iter()
            .map(|event| {
                let phase = match event.kind.as_str() {
                    "span" => "X",
                    "counter" => "C",
                    _ => "i",
                };
                json!({
                    "name": event.name,
                    "cat": event.category,
                    "ph": phase,
                    "ts": ((event.ts_wall_ms - trace_origin_wall_ms) * 1000) as i64,
                    "dur": event.duration_ms.map(|value| (value * 1000.0).round() as i64),
                    "pid": 1,
                    "tid": event.turn_id.clone().unwrap_or_else(|| "frontend".to_string()),
                    "s": "t",
                    "args": event.data.clone().unwrap_or_else(|| json!({}))
                })
            })
            .collect();
        let content = serde_json::to_string_pretty(&json!({
            "traceEvents": trace_events,
            "displayTimeUnit": "ms",
            "metadata": {
                "traceOriginWallMs": trace_origin_wall_ms
            }
        }))
        .map_err(|e| format!("serialize chrome trace export: {e}"))?;
        Ok(FrontendTraceExportPayload {
            format: "chrome-trace".to_string(),
            file_name: build_export_file_name("chrome-trace", &query.session_id),
            content,
            truncated: events.has_more,
            event_count: events.events.len(),
            snapshot_count: 0,
            from_wall_ms: query.from_wall_ms,
            to_wall_ms: query.to_wall_ms,
        })
    }

    pub fn clear_before(&self, ts_wall_ms: i64) -> Result<(), String> {
        let mut slot = self.connection()?;
        let conn = slot.as_mut().expect("connection initialized");
        conn.execute(
            "DELETE FROM frontend_trace_event WHERE ts_wall_ms < ?1",
            params![ts_wall_ms],
        )
        .map_err(|e| format!("clear trace events: {e}"))?;
        conn.execute(
            "DELETE FROM frontend_stall_snapshot WHERE ts_wall_ms < ?1",
            params![ts_wall_ms],
        )
        .map_err(|e| format!("clear stall snapshots: {e}"))?;
        Ok(())
    }

    pub fn latest_event_ts(&self) -> Result<Option<i64>, String> {
        let mut slot = self.connection()?;
        let conn = slot.as_mut().expect("connection initialized");
        let value = conn
            .query_row(
                "SELECT MAX(ts_wall_ms) FROM frontend_trace_event",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(|e| format!("latest event ts: {e}"))?
            .flatten();
        Ok(value)
    }
}

pub fn default_frontend_diagnostics_sqlite_path() -> PathBuf {
    #[cfg(test)]
    {
        // 测试必须隔离：绝不读写用户生产数据（%LOCALAPPDATA%/PonyAgent/frontend-diagnostics.db）。
        std::env::temp_dir().join(format!(
            "pony-agent-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ))
        .join("frontend-diagnostics.db")
    }

    #[cfg(not(test))]
    {
        dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PonyAgent")
            .join("frontend-diagnostics.db")
    }
}

fn build_export_file_name(format: &str, session_id: &Option<String>) -> String {
    let safe_session_id = session_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("global");
    format!("frontend-trace-{}-{}.json", safe_session_id, format)
}

fn retention_window_ms() -> i64 {
    1000 * 60 * 60 * 24 * 14
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "frontend-diagnostics-test-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn append_and_query_roundtrip() {
        let dir = unique_dir("roundtrip");
        fs::create_dir_all(&dir).unwrap();
        let store = FrontendDiagnosticsStore::new(dir.join("diag.db"));
        let now = now_ms();
        store
            .append(FrontendTraceAppendCommand {
                events: vec![FrontendTraceEvent {
                    seq: 1,
                    ts_wall_ms: now - 1000,
                    ts_perf_ms: 10.0,
                    session_id: Some("session-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    scope: "turn".to_string(),
                    category: "runtime.turn".to_string(),
                    name: "completed".to_string(),
                    kind: "instant".to_string(),
                    duration_ms: None,
                    stall_level: None,
                    trigger_kind: None,
                    data: Some(json!({ "messagesCount": 2 })),
                    truncated: Some(false),
                }],
                stall_snapshots: vec![FrontendStallSnapshot {
                    session_id: Some("session-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    ts_wall_ms: now - 990,
                    ts_perf_ms: 11.0,
                    stall_level: "light".to_string(),
                    trigger_kind: "raf-gap".to_string(),
                    stall_gap_ms: 280,
                    snapshot: json!({ "messagesCount": 2 }),
                    truncated: Some(false),
                }],
            })
            .unwrap();

        let result = store
            .query_window(FrontendTraceQuery {
                session_id: Some("session-1".to_string()),
                limit: Some(10),
                ..FrontendTraceQuery::default()
            })
            .unwrap();
        assert_eq!(result.events.len(), 1);
        let snapshots = store
            .query_stall_snapshots(FrontendTraceQuery {
                session_id: Some("session-1".to_string()),
                limit: Some(10),
                ..FrontendTraceQuery::default()
            })
            .unwrap();
        assert_eq!(snapshots.len(), 1);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn chrome_trace_export_normalizes_timestamps_to_trace_origin() {
        let dir = unique_dir("chrome-trace");
        fs::create_dir_all(&dir).unwrap();
        let store = FrontendDiagnosticsStore::new(dir.join("diag.db"));
        let now = now_ms();
        store
            .append(FrontendTraceAppendCommand {
                events: vec![
                    FrontendTraceEvent {
                        seq: 1,
                        ts_wall_ms: now - 1000,
                        ts_perf_ms: 10.0,
                        session_id: Some("session-1".to_string()),
                        turn_id: Some("turn-1".to_string()),
                        scope: "turn".to_string(),
                        category: "runtime.turn".to_string(),
                        name: "started".to_string(),
                        kind: "instant".to_string(),
                        duration_ms: None,
                        stall_level: None,
                        trigger_kind: None,
                        data: Some(json!({ "phase": "start" })),
                        truncated: Some(false),
                    },
                    FrontendTraceEvent {
                        seq: 2,
                        ts_wall_ms: now - 875,
                        ts_perf_ms: 135.0,
                        session_id: Some("session-1".to_string()),
                        turn_id: Some("turn-1".to_string()),
                        scope: "turn".to_string(),
                        category: "runtime.turn".to_string(),
                        name: "completed".to_string(),
                        kind: "span".to_string(),
                        duration_ms: Some(12.5),
                        stall_level: None,
                        trigger_kind: None,
                        data: Some(json!({ "phase": "done" })),
                        truncated: Some(false),
                    },
                ],
                stall_snapshots: vec![],
            })
            .unwrap();

        let export = store
            .export_chrome_trace(FrontendTraceQuery {
                session_id: Some("session-1".to_string()),
                limit: Some(10),
                ..FrontendTraceQuery::default()
            })
            .unwrap();
        let parsed: Value = serde_json::from_str(&export.content).unwrap();
        let trace_events = parsed["traceEvents"].as_array().unwrap();

        assert_eq!(trace_events[0]["ts"].as_i64(), Some(0));
        assert_eq!(trace_events[1]["ts"].as_i64(), Some(125_000));
        assert_eq!(
            parsed["metadata"]["traceOriginWallMs"].as_i64(),
            Some(now - 1000)
        );

        fs::remove_dir_all(&dir).ok();
    }
}
