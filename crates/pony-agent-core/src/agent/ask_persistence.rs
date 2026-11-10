//! Ask persistence protocol: durable, session-scoped Ask (`builtin:ask_user` / Interaction
//! control request) records plus their lifecycle state machine (ADR 0074, wave T2 freeze).
//!
//! **Protocol freeze only** — this module declares the wire/storage contract and the store seam.
//! It intentionally contains no storage, no I/O, no lifecycle engine and no clock: the session
//! store implementation lands in a later task and must satisfy exactly these signatures.
//!
//! Contract points frozen here:
//!
//! * An unanswered Ask is **persisted state**, not dispatcher process memory. It survives process
//!   restart and is recoverable by `session_id`, so a host panel can be rebuilt and answered.
//! * `expires_at_ms` is **not** part of this record: wall-clock TTL is no longer product
//!   semantics. Any remaining time bound is a storage GC retention (`gc_orphans`), never an
//!   answer verdict.
//! * Compare-and-swap facts (`version`, `nonce`) are part of the persisted record, so the CAS
//!   guarantee holds across process boundaries (replay of a consumed/nonce-stale answer is
//!   rejected by whichever process loads the record).
//! * Lifecycle: `Pending → Answered | Cancelled | Superseded | Orphaned`. The three non-Answered
//!   terminal states are distinct dispositions and must never collapse into one another:
//!   `Cancelled` = the user explicitly ignored/dismissed the Ask, `Superseded` = a newer user
//!   message made the question moot, `Orphaned` = the owning session was archived/deleted and GC
//!   reaped the record. Only `Pending` records are ever listed to a host.
//! * Supersede rule: the appearance of a **new user message** in a session moves every `Pending`
//!   Ask of that session to `Superseded` (`supersede_pending_on_user_message`). This replaces the
//!   former wall-clock expiry as the "this question is stale" signal.
//!
//! Field names are camelCase on the wire (`requestId`, `sessionId`, `createdAtMs`, …) and match
//! the TypeScript mirror in `src/types/ask-plan.ts`; enum values are snake_case.

use crate::agent::tool_runtime::PendingControlRequestKind;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;

/// Result of a persistence mutation/read. Errors are opaque display strings for the host:
/// a failed write, an unknown `request_id`, a CAS mismatch (stale `version`/`from` state) or a
/// corrupt record. Fail closed on any of them — never fall back to in-memory pending state.
pub type AskPersistenceResult<T> = std::result::Result<T, String>;

/// Stable rejection code: the stored record is not in the expected `from` state (includes
/// attempts to move out of a terminal state).
pub const ASK_ERR_STATE_CONFLICT: &str = "ask_state_conflict";
/// Stable rejection code: `expected_version` does not match the persisted `version`.
pub const ASK_ERR_VERSION_MISMATCH: &str = "ask_version_mismatch";
/// Stable rejection code: the presented `nonce` does not match the persisted `nonce`.
pub const ASK_ERR_NONCE_MISMATCH: &str = "ask_nonce_mismatch";
/// Stable rejection code: no persisted record for `request_id`.
pub const ASK_ERR_NOT_FOUND: &str = "ask_not_found";

/// Lifecycle disposition of a persisted Ask request.
///
/// Exactly one terminal transition out of `Pending` is allowed per request; terminal states are
/// final (no resurrection, no terminal-to-terminal moves).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AskLifecycleState {
    /// Awaiting a host answer. The only state a host UI may surface.
    Pending,
    /// Answered by the host; `answered_at_ms` is set and the updated `version` is persisted.
    Answered,
    /// Explicitly cancelled/ignored by the user (deliberate dismissal, not staleness).
    Cancelled,
    /// Made moot by a newer user message in the same session.
    Superseded,
    /// Owning session was archived/deleted and the record was reclaimed by GC.
    Orphaned,
}

/// Durable record for one Ask (`Interaction`) request.
///
/// `session_id` is required (it is the recovery key); `run_id` is optional because a control
/// request may outlive the run/turn that raised it. `nonce` and `version` travel with the record
/// so cross-process CAS can reject replayed or stale answers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAsk {
    /// Stable control-request identity; the CAS key for `transition`.
    pub request_id: String,
    /// Monotonic record version; bumped on every state change. CAS precondition for answers.
    pub version: u64,
    /// Anti-replay nonce bound to this request instance; persisted so restart keeps the binding.
    pub nonce: String,
    /// Owning session; the load/GC partition key.
    pub session_id: String,
    /// Run that raised the Ask, when still known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Turn that raised the Ask.
    pub turn_id: String,
    /// Tool call that raised the Ask.
    pub call_id: String,
    /// Interaction (Ask) or Approval control request.
    pub kind: PendingControlRequestKind,
    /// Question text shown to the user.
    pub prompt: String,
    /// Selectable options offered with the question (production schema: string array);
    /// empty when the Ask carries no options.
    #[serde(default)]
    pub options: Vec<String>,
    /// Ask 是否允许多选（模型 `ask_user` 的 `multiSelect`）。旧记录 / 旧库缺该列 ⇒ `false`（单选），
    /// 迁移只加列不改语义，因此这是一个向后兼容的持久化事实。
    #[serde(default)]
    pub multi_select: bool,
    /// Current lifecycle disposition.
    pub state: AskLifecycleState,
    /// Creation wall-clock timestamp (Unix epoch ms); display/ordering only, never an expiry.
    pub created_at_ms: u64,
    /// When (if ever) the Ask was answered (Unix epoch ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at_ms: Option<u64>,
}

/// Storage seam for persisted Ask records, implemented by the session store.
///
/// Implementations must be safe for concurrent host readers (two processes may hold the same
/// session open) and must never expose non-`Pending` records through [`Self::load_pending`].
/// `Send + Sync` is required so the host can hold one store behind `Arc` in shared managed state
/// (the Tauri control plane is `Send + Sync`); it does not change any method contract.
pub trait AskPersistenceStore: Send + Sync {
    /// Every `Pending` Ask of a session, oldest first (`created_at_ms`, then `request_id`).
    ///
    /// Returns an empty vector for a session with no pending Ask, including after a restart:
    /// recovery is a storage read, not a replay of dispatcher memory.
    fn load_pending(&self, session_id: &str) -> AskPersistenceResult<Vec<PersistedAsk>>;

    /// Insert or replace the record for `ask.request_id`.
    ///
    /// The write must persist `version` and `nonce` verbatim so a second process observes the
    /// same CAS facts. Terminal records are retained (not deleted) for audit/GC purposes.
    fn upsert(&self, ask: &PersistedAsk) -> AskPersistenceResult<()>;

    /// Compare-and-swap state change: move `request_id` from `from` to `to` only if the stored
    /// record is still in state `from` **and** at version `expected_version`.
    ///
    /// Returns the updated record (with a bumped `version`, and `answered_at_ms` set for
    /// `Answered`). Any mismatch — unknown id, wrong state, stale version — is an error, so
    /// concurrent answers from two processes resolve to exactly one winner.
    fn transition(
        &self,
        request_id: &str,
        from: AskLifecycleState,
        to: AskLifecycleState,
        expected_version: u64,
    ) -> AskPersistenceResult<PersistedAsk> {
        // Contract §4/§5: production CAS requires an explicit `(session_id, nonce)` scope. This
        // 4-argument entry survives only for API compatibility and is fail-closed: accepting it
        // would let a host settle another session's record (L3-P `unscoped` counter-example).
        let _ = (from, to, expected_version);
        Err(format!(
            "ask_scope_required: unscoped `transition` for `{request_id}` is not a production CAS entry (use transition_scoped with session_id + nonce)"
        ))
    }

    /// Read one record by its composite identity `(session_id, request_id)`, whatever its state.
    fn load_ask(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> AskPersistenceResult<Option<PersistedAsk>> {
        let _ = (session_id, request_id);
        Err("ask_not_found: this store does not support scoped Ask reads".to_string())
    }

    /// Diagnostic lookup across sessions (never authorises a mutation): used only to produce a
    /// stable fail-closed error for unscoped legacy calls.
    fn find_ask(&self, request_id: &str) -> AskPersistenceResult<Option<PersistedAsk>> {
        let _ = request_id;
        Err("ask_not_found: this store does not support unscoped lookups".to_string())
    }

    fn transition_scoped(
        &self,
        session_id: &str,
        request_id: &str,
        expected_version: u64,
        expected_nonce: &str,
        from: AskLifecycleState,
        to: AskLifecycleState,
    ) -> AskPersistenceResult<PersistedAsk> {
        let _ = (session_id, request_id, expected_version, expected_nonce, from, to);
        Err("ask_scope_required: this store does not support scoped CAS".to_string())
    }

    /// Scoped CAS **with the durable answer payload in the same transaction** (contract §4/§5).
    fn transition_scoped_with_answer(
        &self,
        session_id: &str,
        request_id: &str,
        expected_version: u64,
        expected_nonce: &str,
        from: AskLifecycleState,
        to: AskLifecycleState,
        answer: &serde_json::Value,
    ) -> AskPersistenceResult<PersistedAsk> {
        let _ = (
            session_id,
            request_id,
            expected_version,
            expected_nonce,
            from,
            to,
            answer,
        );
        Err("ask_scope_required: this store does not support answer-payload CAS".to_string())
    }

    /// Unlanded `Answered` records **with their durable answer payload** (oldest first).
    fn load_unlanded_answered_payloads(
        &self,
        session_id: &str,
    ) -> AskPersistenceResult<Vec<(PersistedAsk, Value)>> {
        let _ = session_id;
        Ok(Vec::new())
    }

    /// Whether the durable record already served its answer (landing ledger set).
    fn is_landed(&self, session_id: &str, request_id: &str) -> AskPersistenceResult<bool> {
        let _ = (session_id, request_id);
        Ok(false)
    }

    fn mark_landed(
        &self,
        session_id: &str,
        request_id: &str,
        at_ms: u64,
    ) -> AskPersistenceResult<()> {
        let _ = (session_id, request_id, at_ms);
        Err("ask_not_found: this store does not support landing bookkeeping".to_string())
    }

    /// `Answered` records whose landing never completed (crash / mirror-sync failure), oldest
    /// first. The sweep path re-queues their injection instead of losing the answer.
    fn load_unlanded_answered(
        &self,
        session_id: &str,
    ) -> AskPersistenceResult<Vec<PersistedAsk>> {
        let _ = session_id;
        Ok(Vec::new())
    }

    /// Retention GC for **terminal** records only (contract §5: GC must never reclaim `Pending`).
    /// Deletes terminal rows whose landing/answer/creation timestamp is older than `before_ms`.
    fn gc_terminal_records(&self, before_ms: u64) -> AskPersistenceResult<usize> {
        let _ = before_ms;
        Ok(0)
    }

    /// Supersede every `Pending` Ask of the session because a new user message arrived at
    /// `at_ms`; returns how many records moved to [`AskLifecycleState::Superseded`].
    ///
    /// This is the product-level replacement for wall-clock expiry: a question left behind by a
    /// new user message is stale by conversation order, not by time.
    fn supersede_pending_on_user_message(
        &self,
        session_id: &str,
        at_ms: u64,
    ) -> AskPersistenceResult<usize>;

    /// Reclaim records whose owning session is archived/deleted: every record of a session in
    /// `archived_session_ids` moves to [`AskLifecycleState::Orphaned`]; returns how many moved.
    ///
    /// `now_ms` exists only to stamp the GC pass/retention bookkeeping — it must never be
    /// compared against a per-request deadline, and no Ask may be expired by time alone.
    fn gc_orphans(
        &self,
        archived_session_ids: &[String],
        now_ms: u64,
    ) -> AskPersistenceResult<usize>;
}

/// Open the durable Ask store on the session SQLite database at `db_path`.
///
/// This is the **single construction surface** shared by the host and the red-phase acceptance
/// tests: no caller may hand-roll an `AskPersistenceStore` implementation. Opening must also
/// create/migrate the `pending_asks` table so a legacy database without it stays usable
/// (contract §7), and the returned store must tolerate concurrent readers from another process.
pub fn open_ask_store(
    db_path: &std::path::Path,
) -> AskPersistenceResult<Box<dyn AskPersistenceStore>> {
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "ask_store_open_failed: create directory `{}` failed: {error}",
                    parent.display()
                )
            })?;
        }
    }
    let connection = rusqlite::Connection::open(db_path)
        .map_err(|error| format!("ask_store_open_failed: `{}`: {error}", db_path.display()))?;
    // WAL + busy_timeout so a second process (restart/reconnect) can read and CAS concurrently
    // instead of failing with SQLITE_BUSY. `journal_mode` is a persistent database property.
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=10000;",
        )
        .map_err(|error| format!("ask_store_open_failed: pragma: {error}"))?;
    SqliteAskStore::initialize(&connection)?;
    Ok(Box::new(SqliteAskStore {
        connection: Mutex::new(connection),
    }))
}

/// Open an isolated in-memory Ask store for a single process (host smoke paths and tests that do
/// not need cross-process recovery).
pub fn open_in_memory_ask_store() -> AskPersistenceResult<Box<dyn AskPersistenceStore>> {
    let connection = rusqlite::Connection::open_in_memory()
        .map_err(|error| format!("ask_store_open_failed: in-memory: {error}"))?;
    SqliteAskStore::initialize(&connection)?;
    Ok(Box::new(SqliteAskStore {
        connection: Mutex::new(connection),
    }))
}

/// Forward migration (contract §7): one `CREATE TABLE/INDEX IF NOT EXISTS` batch, so a legacy
/// `sessions.db` without the table opens unchanged and gets the table on first open. There is no
/// backfill: the former in-memory pending requests have nothing to migrate.
const ASK_STORE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pending_asks (
    session_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    nonce TEXT NOT NULL,
    run_id TEXT,
    turn_id TEXT NOT NULL,
    call_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    prompt TEXT NOT NULL DEFAULT '',
    options_json TEXT NOT NULL DEFAULT '[]',
    multi_select INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    answered_at_ms INTEGER,
    landed_at_ms INTEGER,
    answer_json TEXT,
    PRIMARY KEY (session_id, request_id)
);
CREATE INDEX IF NOT EXISTS idx_pending_asks_session_state
    ON pending_asks(session_id, state);
CREATE INDEX IF NOT EXISTS idx_pending_asks_session_created
    ON pending_asks(session_id, created_at_ms);
";

const ASK_INDEXES: &str = "
CREATE INDEX IF NOT EXISTS idx_pending_asks_session_state
    ON pending_asks(session_id, state);
CREATE INDEX IF NOT EXISTS idx_pending_asks_session_created
    ON pending_asks(session_id, created_at_ms);
";

const ASK_COLUMNS: &str = "request_id, session_id, version, nonce, run_id, turn_id, call_id, \
     kind, prompt, options_json, state, created_at_ms, answered_at_ms, landed_at_ms, multi_select";

/// `ASK_COLUMNS` 的列数；`ask_row_to_record` 末列索引与「追加一列」查询（`ASK_COLUMNS, answer_json`）
/// 都以它为基准 —— 加列时只改这里，`debug_assert` 会抓住漏改造成的静默错位
/// （错位会把 `answer_json` 读成 `multi_select`，sweep 直接丢答案）。
const ASK_COLUMN_COUNT: usize = 15;
/// `multi_select` 在 `ASK_COLUMNS` 里的位置（末列）。
const ASK_MULTI_SELECT_INDEX: usize = ASK_COLUMN_COUNT - 1;
/// `SELECT {ASK_COLUMNS}, answer_json` 中追加列的索引。
const ASK_APPENDED_ANSWER_INDEX: usize = ASK_COLUMN_COUNT;

/// SQLite-backed [`AskPersistenceStore`] over the session database (`sessions.db` family).
///
/// All lifecycle mutations are single SQL statements or `BEGIN IMMEDIATE` transactions, so two
/// processes answering the same Ask resolve to exactly one winner (CAS is enforced by the
/// database, not by process memory).
struct SqliteAskStore {
    connection: Mutex<rusqlite::Connection>,
}

impl SqliteAskStore {
    fn initialize(connection: &rusqlite::Connection) -> AskPersistenceResult<()> {
        debug_assert_eq!(
            ASK_COLUMNS.split(',').count(),
            ASK_COLUMN_COUNT,
            "ASK_COLUMNS 与 ASK_COLUMN_COUNT 漂移：列索引会静默错位"
        );
        match pending_asks_primary_key(connection)? {
            // Fresh or already-migrated store.
            None => connection
                .execute_batch(ASK_STORE_SCHEMA)
                .map_err(|error| format!("ask_store_migration_failed: {error}")),
            Some(primary_key) if primary_key == COMPOSITE_PRIMARY_KEY.to_vec() => {
                ensure_ask_columns(connection)?;
                connection
                    .execute_batch(ASK_INDEXES)
                    .map_err(|error| format!("ask_store_migration_failed: {error}"))
            }
            // Legacy single-column (`request_id`) primary key: `(session_id, request_id)` is the
            // frozen isolation key (contract §2), so rebuild the table and copy surviving rows.
            Some(_) => {
                migrate_legacy_pending_asks(connection)?;
                ensure_ask_columns(connection)
            }
        }
    }

    /// Shared scoped CAS. When `answer` is provided it is committed **in the same transaction**
    /// (contract §4/§5), so the payload can never be missing for an `Answered` record.
    fn transition_scoped_inner(
        &self,
        session_id: &str,
        request_id: &str,
        expected_version: u64,
        expected_nonce: &str,
        from: AskLifecycleState,
        to: AskLifecycleState,
        answer: Option<&Value>,
    ) -> AskPersistenceResult<PersistedAsk> {
        if to == AskLifecycleState::Pending {
            return Err(format!(
                "ask_state_conflict: `{request_id}` cannot transition back to `pending` (terminal states are final)"
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("ask_store_write_failed: {error}"))?;
        let stored = transaction
            .query_row(
                &format!(
                    "SELECT {ASK_COLUMNS} FROM pending_asks \
                     WHERE session_id = ?1 AND request_id = ?2"
                ),
                rusqlite::params![session_id, request_id],
                ask_row_to_record,
            )
            .optional()
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        let Some(stored) = stored else {
            return Err(format!(
                "ask_not_found: no persisted Ask `{request_id}` in session `{session_id}`"
            ));
        };
        validate_transition(&stored, from, expected_version, Some(expected_nonce))?;
        let updated = apply_transition(&transaction, &stored, to)?;
        if let Some(answer) = answer {
            let encoded = serde_json::to_string(answer)
                .map_err(|error| format!("ask_store_encode_failed: {error}"))?;
            transaction
                .execute(
                    "UPDATE pending_asks SET answer_json = ?1 \
                     WHERE session_id = ?2 AND request_id = ?3",
                    rusqlite::params![encoded, session_id, request_id],
                )
                .map_err(|error| format!("ask_store_write_failed: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("ask_store_write_failed: {error}"))?;
        Ok(updated)
    }

    fn lock(&self) -> AskPersistenceResult<std::sync::MutexGuard<'_, rusqlite::Connection>> {
        self.connection
            .lock()
            .map_err(|_| "ask_store_lock_poisoned: pending_asks connection lock poisoned".to_string())
    }
}

/// Unix epoch milliseconds, stamped on `Answered` transitions (`answered_at_ms` is display/audit
/// only — it is never compared against a deadline).
fn ask_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn ask_enum_to_text<T: Serialize>(value: &T) -> AskPersistenceResult<String> {
    let encoded = serde_json::to_value(value)
        .map_err(|error| format!("ask_store_encode_failed: {error}"))?;
    encoded
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "ask_store_encode_failed: enum did not encode as a string".to_string())
}

fn ask_enum_from_text<T: serde::de::DeserializeOwned>(
    column: usize,
    text: &str,
) -> rusqlite::Result<T> {
    serde_json::from_value(Value::String(text.to_string())).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn ask_row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<PersistedAsk> {
    let kind_text: String = row.get(7)?;
    let options_json: String = row.get(9)?;
    let state_text: String = row.get(10)?;
    Ok(PersistedAsk {
        request_id: row.get(0)?,
        session_id: row.get(1)?,
        version: row.get(2)?,
        nonce: row.get(3)?,
        run_id: row.get(4)?,
        turn_id: row.get(5)?,
        call_id: row.get(6)?,
        kind: ask_enum_from_text(7, &kind_text)?,
        prompt: row.get(8)?,
        options: serde_json::from_str(&options_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        state: ask_enum_from_text(10, &state_text)?,
        created_at_ms: row.get(11)?,
        answered_at_ms: row.get(12)?,
        multi_select: row.get(ASK_MULTI_SELECT_INDEX)?,
    })
}

const COMPOSITE_PRIMARY_KEY: [&str; 2] = ["session_id", "request_id"];

/// Primary-key columns of an existing `pending_asks` table (`None` when the table is absent).
fn pending_asks_primary_key(
    connection: &rusqlite::Connection,
) -> AskPersistenceResult<Option<Vec<String>>> {
    let mut statement = connection
        .prepare("PRAGMA table_info(pending_asks)")
        .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })
        .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    let mut keyed = rows
        .collect::<rusqlite::Result<Vec<(String, i64)>>>()
        .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    if keyed.is_empty() {
        return Ok(None);
    }
    keyed.retain(|(_, position)| *position > 0);
    keyed.sort_by_key(|(_, position)| *position);
    Ok(Some(keyed.into_iter().map(|(name, _)| name).collect()))
}

/// Landing/payload columns (contract §5). Both are forward-migrated on older stores:
/// `landed_at_ms` is the landing ledger, `answer_json` is the durable answer payload that makes a
/// sweep rebuild factual instead of invented.
fn ensure_ask_columns(connection: &rusqlite::Connection) -> AskPersistenceResult<()> {
    let mut statement = connection
        .prepare("PRAGMA table_info(pending_asks)")
        .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("ask_store_migration_failed: {error}"))?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    if !columns.iter().any(|column| column == "landed_at_ms") {
        connection
            .execute_batch("ALTER TABLE pending_asks ADD COLUMN landed_at_ms INTEGER;")
            .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    }
    if !columns.iter().any(|column| column == "answer_json") {
        connection
            .execute_batch("ALTER TABLE pending_asks ADD COLUMN answer_json TEXT;")
            .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    }
    // 多选声明（ask_user 的 `multiSelect`）：旧库补列时一律落 `0`（单选），与旧行为逐字一致。
    if !columns.iter().any(|column| column == "multi_select") {
        connection
            .execute_batch("ALTER TABLE pending_asks ADD COLUMN multi_select INTEGER NOT NULL DEFAULT 0;")
            .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    }
    Ok(())
}

/// Rebuild a legacy `pending_asks`/// Rebuild a legacy `pending_asks` (single-column `request_id` primary key) into the frozen
/// `(session_id, request_id)` composite key. Rows are copied verbatim; `INSERT OR IGNORE` keeps
/// the migration total when a legacy db somehow already holds composite duplicates.
fn migrate_legacy_pending_asks(connection: &rusqlite::Connection) -> AskPersistenceResult<()> {
    connection
        .execute_batch(
            "ALTER TABLE pending_asks RENAME TO pending_asks_legacy;
             CREATE TABLE pending_asks_v2 (
                 session_id TEXT NOT NULL,
                 request_id TEXT NOT NULL,
                 version INTEGER NOT NULL,
                 nonce TEXT NOT NULL,
                 run_id TEXT,
                 turn_id TEXT NOT NULL,
                 call_id TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 prompt TEXT NOT NULL DEFAULT '',
                 options_json TEXT NOT NULL DEFAULT '[]',
                 state TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 answered_at_ms INTEGER,
                 landed_at_ms INTEGER,
                 answer_json TEXT,
                 PRIMARY KEY (session_id, request_id)
             );
             INSERT OR IGNORE INTO pending_asks_v2 (
                 session_id, request_id, version, nonce, run_id, turn_id, call_id, kind,
                 prompt, options_json, state, created_at_ms, answered_at_ms
             ) SELECT session_id, request_id, version, nonce, run_id, turn_id, call_id, kind,
                      prompt, options_json, state, created_at_ms, answered_at_ms
               FROM pending_asks_legacy;
             DROP TABLE pending_asks_legacy;
             ALTER TABLE pending_asks_v2 RENAME TO pending_asks;",
        )
        .map_err(|error| format!("ask_store_migration_failed: {error}"))?;
    connection
        .execute_batch(ASK_INDEXES)
        .map_err(|error| format!("ask_store_migration_failed: {error}"))
}

/// Shared CAS predicate for both the unscoped and scoped transitions (contract §3/§4).
fn validate_transition(
    stored: &PersistedAsk,
    from: AskLifecycleState,
    expected_version: u64,
    expected_nonce: Option<&str>,
) -> AskPersistenceResult<()> {
    if stored.state != from || stored.state != AskLifecycleState::Pending {
        return Err(format!(
            "ask_state_conflict: Ask `{}` is {:?}, not {:?} (terminal states are final)",
            stored.request_id, stored.state, from
        ));
    }
    if stored.version != expected_version {
        return Err(format!(
            "ask_version_mismatch: Ask `{}` is at version {} but expected {expected_version}",
            stored.request_id, stored.version
        ));
    }
    if let Some(nonce) = expected_nonce {
        if stored.nonce != nonce {
            return Err(format!(
                "ask_nonce_mismatch: Ask `{}` nonce does not match the persisted record",
                stored.request_id
            ));
        }
    }
    Ok(())
}

/// Apply one legal `Pending -> terminal` edge inside the caller's transaction and read back the
/// persisted row (so the returned record is the disk truth, not a computed projection).
fn apply_transition(
    transaction: &rusqlite::Transaction<'_>,
    stored: &PersistedAsk,
    to: AskLifecycleState,
) -> AskPersistenceResult<PersistedAsk> {
    let answered_at_ms = if to == AskLifecycleState::Answered {
        Some(ask_now_ms())
    } else {
        stored.answered_at_ms
    };
    transaction
        .execute(
            "UPDATE pending_asks SET state = ?1, version = ?2, answered_at_ms = ?3 \
             WHERE session_id = ?4 AND request_id = ?5",
            rusqlite::params![
                ask_enum_to_text(&to)?,
                stored.version.saturating_add(1),
                answered_at_ms,
                stored.session_id,
                stored.request_id
            ],
        )
        .map_err(|error| format!("ask_store_write_failed: {error}"))?;
    transaction
        .query_row(
            &format!(
                "SELECT {ASK_COLUMNS} FROM pending_asks \
                 WHERE session_id = ?1 AND request_id = ?2"
            ),
            rusqlite::params![stored.session_id, stored.request_id],
            ask_row_to_record,
        )
        .map_err(|error| format!("ask_store_read_failed: {error}"))
}

impl AskPersistenceStore for SqliteAskStore {
    fn load_pending(&self, session_id: &str) -> AskPersistenceResult<Vec<PersistedAsk>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ASK_COLUMNS} FROM pending_asks \
                 WHERE session_id = ?1 AND state = 'pending' \
                 ORDER BY created_at_ms ASC, request_id ASC"
            ))
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        let rows = statement
            .query_map(rusqlite::params![session_id], ask_row_to_record)
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("ask_store_read_failed: {error}"))
    }

    fn upsert(&self, ask: &PersistedAsk) -> AskPersistenceResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO pending_asks (
                     session_id, request_id, version, nonce, run_id, turn_id, call_id, kind,
                     prompt, options_json, state, created_at_ms, answered_at_ms, multi_select
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(session_id, request_id) DO UPDATE SET
                     version = excluded.version,
                     nonce = excluded.nonce,
                     run_id = excluded.run_id,
                     turn_id = excluded.turn_id,
                     call_id = excluded.call_id,
                     kind = excluded.kind,
                     prompt = excluded.prompt,
                     options_json = excluded.options_json,
                     state = excluded.state,
                     created_at_ms = excluded.created_at_ms,
                     answered_at_ms = excluded.answered_at_ms,
                     multi_select = excluded.multi_select",
                rusqlite::params![
                    ask.session_id,
                    ask.request_id,
                    ask.version,
                    ask.nonce,
                    ask.run_id,
                    ask.turn_id,
                    ask.call_id,
                    ask_enum_to_text(&ask.kind)?,
                    ask.prompt,
                    serde_json::to_string(&ask.options)
                        .map_err(|error| format!("ask_store_encode_failed: {error}"))?,
                    ask_enum_to_text(&ask.state)?,
                    ask.created_at_ms,
                    ask.answered_at_ms,
                    ask.multi_select,
                ],
            )
            .map_err(|error| format!("ask_store_write_failed: {error}"))?;
        Ok(())
    }

    fn load_ask(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> AskPersistenceResult<Option<PersistedAsk>> {
        let connection = self.lock()?;
        connection
            .query_row(
                &format!(
                    "SELECT {ASK_COLUMNS} FROM pending_asks \
                     WHERE session_id = ?1 AND request_id = ?2"
                ),
                rusqlite::params![session_id, request_id],
                ask_row_to_record,
            )
            .optional()
            .map_err(|error| format!("ask_store_read_failed: {error}"))
    }

    fn find_ask(&self, request_id: &str) -> AskPersistenceResult<Option<PersistedAsk>> {
        let connection = self.lock()?;
        connection
            .query_row(
                &format!(
                    "SELECT {ASK_COLUMNS} FROM pending_asks WHERE request_id = ?1 \
                     ORDER BY session_id ASC LIMIT 1"
                ),
                rusqlite::params![request_id],
                ask_row_to_record,
            )
            .optional()
            .map_err(|error| format!("ask_store_read_failed: {error}"))
    }

    fn transition_scoped(
        &self,
        session_id: &str,
        request_id: &str,
        expected_version: u64,
        expected_nonce: &str,
        from: AskLifecycleState,
        to: AskLifecycleState,
    ) -> AskPersistenceResult<PersistedAsk> {
        self.transition_scoped_inner(
            session_id,
            request_id,
            expected_version,
            expected_nonce,
            from,
            to,
            None,
        )
    }

    fn transition_scoped_with_answer(
        &self,
        session_id: &str,
        request_id: &str,
        expected_version: u64,
        expected_nonce: &str,
        from: AskLifecycleState,
        to: AskLifecycleState,
        answer: &serde_json::Value,
    ) -> AskPersistenceResult<PersistedAsk> {
        self.transition_scoped_inner(
            session_id,
            request_id,
            expected_version,
            expected_nonce,
            from,
            to,
            Some(answer),
        )
    }

    fn load_unlanded_answered_payloads(
        &self,
        session_id: &str,
    ) -> AskPersistenceResult<Vec<(PersistedAsk, Value)>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ASK_COLUMNS}, answer_json FROM pending_asks \
                 WHERE session_id = ?1 AND state = 'answered' AND landed_at_ms IS NULL \
                 ORDER BY answered_at_ms ASC, request_id ASC"
            ))
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        let rows = statement
            .query_map(rusqlite::params![session_id], |row| {
                let record = ask_row_to_record(row)?;
                let payload: Option<String> = row.get(ASK_APPENDED_ANSWER_INDEX)?;
                let answer = payload
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .unwrap_or(Value::Null);
                Ok((record, answer))
            })
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("ask_store_read_failed: {error}"))
    }

    fn is_landed(&self, session_id: &str, request_id: &str) -> AskPersistenceResult<bool> {
        let connection = self.lock()?;
        let landed: Option<Option<i64>> = connection
            .query_row(
                "SELECT landed_at_ms FROM pending_asks \
                 WHERE session_id = ?1 AND request_id = ?2",
                rusqlite::params![session_id, request_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        Ok(matches!(landed, Some(Some(_))))
    }

    fn mark_landed(
        &self,
        session_id: &str,
        request_id: &str,
        at_ms: u64,
    ) -> AskPersistenceResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE pending_asks SET landed_at_ms = ?1 \
                 WHERE session_id = ?2 AND request_id = ?3 AND state = 'answered'",
                rusqlite::params![at_ms, session_id, request_id],
            )
            .map_err(|error| format!("ask_store_write_failed: {error}"))?;
        Ok(())
    }

    fn load_unlanded_answered(
        &self,
        session_id: &str,
    ) -> AskPersistenceResult<Vec<PersistedAsk>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ASK_COLUMNS} FROM pending_asks \
                 WHERE session_id = ?1 AND state = 'answered' AND landed_at_ms IS NULL \
                 ORDER BY answered_at_ms ASC, request_id ASC"
            ))
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        let rows = statement
            .query_map(rusqlite::params![session_id], ask_row_to_record)
            .map_err(|error| format!("ask_store_read_failed: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("ask_store_read_failed: {error}"))
    }

    fn gc_terminal_records(&self, before_ms: u64) -> AskPersistenceResult<usize> {
        // Terminal-only retention GC. `state <> 'pending'` is the hard guard: a pending Ask is
        // session state and must never be reclaimed by time (contract §5).
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM pending_asks \
                 WHERE state <> 'pending' \
                   AND NOT (state = 'answered' AND landed_at_ms IS NULL) \
                   AND COALESCE(landed_at_ms, answered_at_ms, created_at_ms) < ?1",
                rusqlite::params![before_ms],
            )
            .map_err(|error| format!("ask_store_write_failed: {error}"))
    }

    fn supersede_pending_on_user_message(
        &self,
        session_id: &str,
        _at_ms: u64,
    ) -> AskPersistenceResult<usize> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE pending_asks SET state = 'superseded', version = version + 1 \
                 WHERE session_id = ?1 AND state = 'pending'",
                rusqlite::params![session_id],
            )
            .map_err(|error| format!("ask_store_write_failed: {error}"))
    }

    fn gc_orphans(
        &self,
        archived_session_ids: &[String],
        _now_ms: u64,
    ) -> AskPersistenceResult<usize> {
        if archived_session_ids.is_empty() {
            return Ok(0);
        }
        let placeholders = (1..=archived_session_ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let connection = self.lock()?;
        connection
            .execute(
                &format!(
                    "UPDATE pending_asks SET state = 'orphaned', version = version + 1 \
                     WHERE state = 'pending' AND session_id IN ({placeholders})"
                ),
                rusqlite::params_from_iter(archived_session_ids.iter()),
            )
            .map_err(|error| format!("ask_store_write_failed: {error}"))
    }
}

