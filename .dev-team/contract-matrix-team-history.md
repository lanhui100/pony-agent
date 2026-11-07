# Contract Matrix — Team History (R1 re-frozen by Lead)

Status: **R1 Frozen / additive amendment after Gate-1 FAIL**. Owner: L2-P Protocol Agent. Source of truth: this matrix plus stub signatures in `crates/pony-agent-core/src/agent/team_history.rs`. R1 supersedes the initial frozen semantics only where explicitly marked; all existing public functions remain additive.

## 1. Goal / Non-goals

### Goal
- Expose a read-only, cross-session aggregate of team tasks, teammate members, and agent-team sessions for TUI `/tasks`, `/agents`, and agent-team views.
- Rebuild the historical snapshot from the **full union** of local `sessions.db` session IDs found in the `sessions` table and `turn_events`, covering `tool/call` + `tool/result` events whose tool names are `team_task_create`, `team_task_update`, `team_task_list`, `spawn_teammate`, `list_agents`, `send_message`, `drain_inbox`.
- Current-process live state wins; historical records are a read-only complement.
- Aggregation runs in an async refresh job/cache; TUI renders `加载中 / 离线` and never blocks the turn loop.
- Every historical row is explicitly marked `is_historical = true`; UI Enter/action handlers reject historical rows and must not call `drive_teammate` or any team mutation.

### Non-goals
- No writes to `turn_events`, `sessions`, `TEAM_TASKS`, `TEAMMATES`, inboxes, or any tool state.
- No DDL / schema creation / migration / version backfill during historical discovery; `ensure_schema` is forbidden.
- No new slash command, no new tool, no `tools.rs` `team_task_*` semantic change, no `tui.rs`/`orchestration.rs` mutation by Protocol Agent.
- No global `SessionStore::new()` / production DB path dependency inside pure injectable seams; no `team_task_create()`-generated UUID assumption in tests.
- No synchronous 16k-event parsing on the TUI/turn-loop thread.
- No claim that `todo!("NotImplemented")` is an implementation: all public functions are IMPL-ABSENT until Executor completes them.
- No modification to `.dev-team/nfr-baseline.json`; no deletion or renumbering of existing keys.

## 2. Frozen public API (R1 exact signatures)

Module path: `crate::agent::team_history` (Lead has assembled `pub mod team_history;` in `agent/mod.rs`; Protocol Agent did not edit `mod.rs`). Existing public types are reused, not copied: `crate::agent::orchestration::{TeamTaskItem, TeammateMember}`, `crate::agent::session::{SessionOverview, SessionStore}`, `crate::agent::turn_event::TurnEvent`.

```rust
pub struct TeamHistoryTaskRow {
    pub task: TeamTaskItem,
    pub source_session_id: Option<String>,
    #[serde(default)] pub is_historical: bool,
}
pub struct TeamHistoryMemberRow {
    pub member: TeammateMember,
    pub source_session_id: Option<String>,
    #[serde(default)] pub is_historical: bool,
}
pub struct TeamHistorySession {
    pub session_id: String,
    pub title: String,
    pub updated_at_ms: u64,
    #[serde(default)] pub in_sessions_table: bool,
    #[serde(default)] pub in_turn_events: bool,
}
pub struct TeamHistoryDiagnostics {
    pub skipped_payload_count: u64,
    pub skipped_session_count: u64,
    pub db_unavailable: bool,
}
pub struct TeamHistorySnapshot {
    pub tasks: Vec<TeamHistoryTaskRow>,
    pub members: Vec<TeamHistoryMemberRow>,
    pub sessions: Vec<TeamHistorySession>,
    pub diagnostics: TeamHistoryDiagnostics,
}
pub struct TeamHistoryRefreshRequest { pub db_path: String, pub trace_id: String }
pub enum TeamHistoryRefreshStatus { Loading, Ready, Offline }
pub struct TeamHistoryCacheState {
    #[serde(default)] pub status: TeamHistoryRefreshStatus,
    #[serde(default)] pub snapshot: TeamHistorySnapshot,
}

pub fn team_history_schema_probe(db_path: &Path) -> bool;
pub fn team_history_discover_sessions(db_path: &Path) -> Vec<TeamHistorySession>;
pub fn team_history_snapshot_from_db_path(db_path: &Path) -> TeamHistorySnapshot;
pub fn team_history_snapshot_from_store(store: &SessionStore) -> TeamHistorySnapshot;
pub fn team_history_snapshot_from_events<F>(
    sessions: &[SessionOverview],
    load_events: F,
) -> TeamHistorySnapshot
where
    F: Fn(&str) -> Vec<(u64, String, TurnEvent)>;
pub fn merge_team_history(
    history: &TeamHistorySnapshot,
    live_tasks: &[TeamTaskItem],
    live_agents: &[TeammateMember],
) -> TeamHistorySnapshot;
pub fn team_history_tasks_for_tui(
    snapshot: &TeamHistorySnapshot,
    live: &[TeamTaskItem],
) -> Vec<TeamHistoryTaskRow>;
pub fn team_history_members_for_tui(
    snapshot: &TeamHistorySnapshot,
    live: &[TeammateMember],
) -> Vec<TeamHistoryMemberRow>;
pub fn team_history_cache_from_job_result(
    result: Option<TeamHistorySnapshot>,
) -> TeamHistoryCacheState;
pub fn team_history_task_action_allowed(row: &TeamHistoryTaskRow) -> bool;
pub fn team_history_member_action_allowed(row: &TeamHistoryMemberRow) -> bool;
```

### R1 signature decisions
- **No `Result`**: all functions are total and non-throwing at the contract boundary. A failed store/session read is represented by empty historical contribution plus diagnostics; TUI still renders live state.
- `is_historical: bool` is a first-class assertable field on both row types. Historical row: `true`, `source_session_id = Some(session_id)`; live row: `false`, `source_session_id = None`. `serde(default)` keeps old fixtures decodable, but every constructor in implementation must set the field explicitly.
- **`AgentRosterEntry` readonly field**: Protocol Agent's write scope excludes `crates/pony-agent-core/src/bin/tui/protocol.rs`, so no field is added there by L2-P. Executor must add an assertable `pub is_historical: bool` (or Lead-approved equivalent `readonly`) to `AgentRosterEntry` and set it from `TeamHistoryMemberRow.is_historical`. Live rows remain `false`; historical rows `true`. The Enter branch at `tui.rs:10180-10195` must check the selected entry before `enter_teammate_session`: if `is_historical || source_session_id.is_some()` then reject with a status notice, never call `drive_teammate`, `enter_teammate_session`, or any team-tool action. `source_session_id.is_some()` is a defensive second signal; the primary contract is the boolean.
- `team_history_schema_probe(db_path: &Path)` opens a **read-only** connection and only queries `sqlite_master` / `PRAGMA table_info`; no DDL, no `INSERT OR IGNORE`, no `ensure_schema`, no `migrate_from_json`. It returns `false` for a missing/corrupt/unsupported schema.
- `team_history_discover_sessions(db_path: &Path)` opens `file:<db_path>?mode=ro` (or `rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI`) and computes the deduplicated union of `SELECT conversation_id FROM sessions` and `SELECT DISTINCT session_id FROM turn_events`. `store.list_sessions()` is **not** sufficient and must not be the discovery source: 7/14 tasks may be non-persistable in the in-memory session map but still present in `turn_events`.
- `TeamHistorySession` preserves `in_sessions_table` / `in_turn_events` so tests can assert discovery provenance. IDs found only in `turn_events` are included with `title = ""`, `updated_at_ms` derived from `MAX(created_at_ms)` for that session, `in_sessions_table=false`, `in_turn_events=true`. IDs found in `sessions` preserve `title` / `updated_at_ms` verbatim.
- `team_history_snapshot_from_db_path(db_path: &Path)` is the production aggregate entry used by the async refresh job; it must open `?mode=ro`, call the read-only schema probe, and never call `ensure_schema`. `team_history_snapshot_from_store(&SessionStore)` remains an additive compatibility seam over already-loaded store data and is not used to open SQLite.
- `team_history_snapshot_from_events<F>` remains an injectable pure seam. It accepts discovered sessions and a callback returning `(seq, branch_id, TurnEvent)` triples. It does not call `SessionStore::new()` or read `TEAM_TASKS` / `TEAMMATES`.
- `merge_team_history` remains the explicit live-injection seam for same-id coverage without process singletons / UUID nondeterminism. TUI wrappers use the same merge semantics.
- `TeamHistoryRefreshRequest` carries `db_path` + `trace_id` into the async job; `TeamHistoryCacheState` carries `Loading` / `Ready` / `Offline` plus the latest snapshot. `team_history_cache_from_job_result(None)` yields `Offline` with an empty snapshot; `Some(snapshot)` yields `Ready`.
- `team_history_task_action_allowed` / `team_history_member_action_allowed` are pure guard seams: return `false` for `is_historical == true`; live rows may remain eligible for existing behavior. Enter/action handlers must call the guard or equivalent field check.
- There is **no separate parse function**. Acceptance fixtures construct `TurnEvent::ToolCall` / `ToolResult` values with JSON strings in `arguments` / `result`.
- All public functions in the stub are `todo!("NotImplemented")`; calling one panics and is valid red-phase evidence (`IMPL-ABSENT`).

## 3. Data source, full discovery, and event interpretation

- Physical tables: `sessions(conversation_id, title, updated_at_ms, session_data, ...)` and `turn_events(session_id, turn_id, branch_id, seq, event_type, payload, created_at_ms, ...)`, both created in `crates/pony-agent-core/src/agent/sqlite_session.rs`.
- **R1 full discovery (red line)**: discover `session_id` set = `sessions.conversation_id` ∪ distinct `turn_events.session_id`. Deduplicate by exact `session_id`. This is required to include all historical sessions, including non-persistable/in-memory-only task-board sessions whose team events are still in SQLite. `store.list_sessions()` may enrich metadata but never narrows discovery.
- **Read-only guarantee (red line)**: every SQLite open in the refresh job uses `?mode=ro` / `SQLITE_OPEN_READ_ONLY`. Schema validation uses only read-only probes (`sqlite_master`, `PRAGMA table_info`). `ensure_schema`, `migrate_from_json`, `CREATE TABLE`, `INSERT OR IGNORE INTO store_metadata`, and any PRAGMA that writes are forbidden. A missing DB file is `db_unavailable=true`, not a created empty DB.
- `payload` is JSON matching `crate::agent::turn_event::TurnEvent` (`event_type` = `tool/call` / `tool/result`; enum uses `#[serde(tag = "type")]`). `ToolCall.arguments` and `ToolResult.result` are JSON text. Pair by `call_id` within a session; replay by ascending `seq`.
- Tool-name whitelist: `team_task_create`, `team_task_update`, `team_task_list`, `spawn_teammate`, `list_agents`, `send_message`, `drain_inbox`. Other tool names ignored without diagnostics.
- Result envelope: existing tool router `{"ok":true,"tool":"<name>","result":<payload>}`; erring results ignored.
- Task reconstruction: `team_task_create` result → `TeamTaskItem`; `team_task_update` result → later same-id item (higher revision / later event wins); `team_task_list` result `{"tasks":[...]}` → board snapshot; later list supersedes earlier per-task rows.
- Member reconstruction: `spawn_teammate` result `{"member":...}` → `TeammateMember`; `list_agents` result `{"agents":[...]}` → roster snapshot; later list supersedes earlier per-target rows. `send_message` / `drain_inbox` are in scope for status/roster evolution but must never mutate live inbox. Unknown status values are skipped as malformed.
- `source_session_id: Option<String>` remains assertable. `is_historical` is the authoritative readonly flag; `source_session_id.is_some()` is a defensive UI check. UI may render session tail, but the display string is not the contract.

## 4. Merge, dedup, ordering, and readonly propagation

| Surface | Live priority | Dedup key | Stable ordering |
| --- | --- | --- | --- |
| `/tasks` | `live_tasks` wins over historical same-id rows | `TeamHistoryTaskRow.task.task_id` | `task.created_at_ms` ascending, then `task.task_id` ascending |
| `/agents` and agent-team view | `live_agents` wins over historical same-target rows | `TeamHistoryMemberRow.member.target` | `member.target` ascending |
| `sessions` | Discovery union; no live override | `TeamHistorySession.session_id` | `updated_at_ms` descending, then `session_id` ascending |

- Historical-only rows: `source_session_id = Some(...)`, `is_historical = true`. Live rows: `source_session_id = None`, `is_historical = false`. Live rows are not copied into `sessions`.
- Same historical id across sessions: later event by `seq` (ties by `session_id`) wins; winning row keeps its own source and `is_historical=true`.
- No mutation of `history`, `live_tasks`, `live_agents`, `snapshot`, or process state during merge.
- Current-session behavior byte-for-byte unchanged: existing live rows keep original fields, `is_historical=false`, no source suffix; cross-session rows append by stable ordering key.
- Every historical row projected into `AgentRosterEntry` or a TUI task row must carry `is_historical=true`. Enter/action paths failing to observe this flag are contract violations.

## 5. Failure semantics, async cache, and diagnostics

- **Clean fixture**: well-formed whitelisted payloads produce `diagnostics.skipped_payload_count == 0`. **Bad payload**: malformed JSON / wrong shape / unpaired call-result / unknown envelope is skipped without panic, increments `skipped_payload_count >= 1`, and does not abort other events/sessions.
- **DB unavailable / corrupted DB**: no DB, DB corruption, failed `?mode=ro` open, or failed readonly schema probe yields empty historical contribution (`tasks=[]`, `members=[]`, `sessions=[]`), `diagnostics.db_unavailable == true`, cache status `Offline`, live state unchanged. Clean empty readable DB: `db_unavailable=false`, counters zero, cache status `Ready` with empty snapshot.
- **Async refresh (R1 red line)**: the 16k-event scan must run in a background job, not in the turn loop or TUI render path. Before first result, cache status `Loading` and TUI shows `加载中`; on failure/None, `Offline` and TUI shows `离线`; on success, `Ready`. `TeamHistoryRefreshRequest.trace_id` is required in TUI stderr JSON diagnostics with `session_id` context. A slow job must not stall input or streaming.
- **No `Result` / no `unwrap` across boundary**: implemented aggregate never panics for bad local data. `todo!("NotImplemented")` is only the frozen stub state.
- **Degraded session**: `load_turn_events_checked` may mark a session degraded; aggregate must not mutate that marker. Skip degraded session, increment `skipped_session_count`, keep other sessions.
- **Readonly action gate**: `team_history_task_action_allowed` / `team_history_member_action_allowed` return false for historical rows. TUI Enter handler must reject `is_historical=true` before `drive_teammate` / `enter_teammate_session` / team mutation.

## 6. UI constraints

- `/tasks`, `/agents`, and agent-team view display cross-session rows plus live rows.
- Cross-session rows labeled with source session tail. Existing `session_tail` uses last 4 chars (`crates/pony-agent-core/src/bin/tui.rs:1266-1271`); reuse convention, do not redefine in core module.
- Historical rows are read-only: `is_historical=true`; no claim/complete/update/edit/drive actions. `AgentRosterEntry` must expose an assertable `is_historical` / `readonly` field; Enter at `tui.rs:10180-10195` checks it and refuses with a status notice.
- Current-session rows keep original rendering/fields (`source_session_id=None`, `is_historical=false`); no new slash command, no new keyboard mode.
- TUI shows `加载中` while `TeamHistoryRefreshStatus::Loading`, `离线` while `Offline`; `Ready` renders from cached snapshot. Refresh failure never clears or mutates live rows.
- `tools.rs` team tool semantics, `orchestration.rs` live state, and current-session byte layout are not altered by this contract.

## 7. NFR baseline applicability (reference only; file unchanged)

The frozen `.dev-team/nfr-baseline.json` is **not modified**. `dev-nfr-lint.py` Exit 0 re-verified. Existing fields applicable:

| Existing baseline field | Frozen value / shape | Applicability to this read-only local SQLite async job |
| --- | --- | --- |
| `external_call_timeout_ms` | `3000` | No network call; local SQLite job must be bounded/cancellable and never block turn loop. |
| `retry` | `{ "max_attempts": 1, "backoff_cap_ms": 1000, "backoff_ms": 500 }` | At most one attempt / zero retries for DB reads; failure becomes `Offline`. |
| `concurrency_lock` / `lock_scope_bounded` / `lock_reentrancy_forbidden` | `mutex`, `true`, `true` | Cache uses short `Mutex` scopes only; never hold across `await`, SQLite I/O, or callbacks; no reentrant acquisition. |
| `logging` | `{ "format": "json", "trace_id": true, "trace_id_required": true, "trace_id_field": "trace_id" }` | Core module emits no logs. TUI stderr unified sink logs refresh start/failure with `trace_id`, `session_id`; no new baseline key. |
| `resource_release` | existing array | `rusqlite::Connection` / `Statement` / `Rows` scoped and RAII-released; async job handle cleaned up; no descriptor/pool retained. |
| `lock_acquisition_timeout_ms` / `deadlock_watchdog_timeout_ms` / `lock_forbidden_calls` | existing values | Cache/job locks bounded; forbidden graph/checkpoint loaders not called. |

No additive key requested or approved; no existing key deleted/renumbered.

## 8. Acceptance assertion checklist

1. **A1 — two sessions aggregate**: two discovered sessions, each with one task and one member, produce `snapshot.tasks.len() == 2`, `snapshot.members.len() == 2`, each row `source_session_id == Some(...)`, `is_historical == true`.
2. **A2 — live overrides same id**: live `task-1` / `executor` override history; exactly one row per id/target, equals live value, `source_session_id == None`, `is_historical == false`.
3. **A3 — bad payload isolation**: clean `skipped_payload_count == 0`; malformed payload in session A + valid rows in B: no panic, `skipped_payload_count >= 1`, all valid B rows remain.
4. **A4 — empty DB**: empty discovery and event callback return default snapshot semantics (`tasks=[]`, `members=[]`, `sessions=[]`, `db_unavailable=false`, counters zero), cache `Ready`.
5. **A5 — DB unavailable**: no DB / corruption / failed readonly probe → empty historical rows, `db_unavailable == true`, cache `Offline`, live rows unchanged.
6. **A6 — current-session byte stability**: no history ⇒ TUI wrappers output live rows with `source_session_id=None`, `is_historical=false`, frozen sort order; no extra label/suffix on live rows.
7. **A7 — deterministic ordering**: tasks `created_at_ms` then `task_id`; members `target`; sessions `updated_at_ms` desc then `session_id`.
8. **A8 — no global-state dependency**: injectable event/live seams work without production `SessionStore` or `TEAM_TASKS`/`TEAMMATES`; parallel tests isolated.
9. **A9 — additive contract**: existing tools and public types unchanged; no new slash command.
10. **A10 — NFR**: `dev-nfr-lint.py .dev-team/nfr-baseline.json` Exit 0; baseline hash/content unchanged.
11. **A11 — red-phase classification**: all stub functions panic `NotImplemented` → `IMPL-ABSENT`; Lead assembled `pub mod team_history;` and `cargo check --lib` Exit 0 after R1 stub edit requires re-verification. After implementation no `todo!()` remains and A1–A10, A12–A18 pass.
12. **A12 — session metadata one-to-one**: `snapshot.sessions` contains every discovered session ID (not just `list_sessions()`), preserving metadata where available; no filter/synthesis.
13. **A13 — diagnostics exactness**: clean `skipped_payload_count == 0`; malformed `>= 1`; no-DB/corrupt `db_unavailable == true` with empty history.
14. **A14 — full discovery union**: fixture has `sessions` row `S1` with no events and `turn_events` row `S2` absent from `sessions`; both IDs appear in `snapshot.sessions`, with provenance `S1.in_sessions_table=true`, `S2.in_turn_events=true`, and S2 events are parsed.
15. **A15 — read-only probe**: no DB file / corrupt DB / missing schema returns `false` from `team_history_schema_probe` and creates no file/table; a valid DB probe does not modify schema or `store_metadata`; all production opens use `?mode=ro`.
16. **A16 — readonly flag and Enter gate**: historical member row has `is_historical=true`; `team_history_member_action_allowed(row) == false`; projected `AgentRosterEntry.is_historical == true`; Enter handler refuses without calling `drive_teammate` / `enter_teammate_session`. Live row guard returns true / existing behavior unchanged.
17. **A17 — async cache**: `TeamHistoryCacheState::default()` is `Loading`; `team_history_cache_from_job_result(None)` is `Offline`; `Some(snapshot)` is `Ready`; 16k event scan is not executed on turn/TUI thread (job-based fixture or injected cache state).
18. **A18 — `ensure_schema` forbidden**: grep/static assertion that `team_history.rs` implementation contains no `ensure_schema`, `CREATE TABLE`, `INSERT OR IGNORE`, or writable PRAGMA; readonly schema probe only.

## 9. Coverage gaps / explicit blind spots

- `dev-stub-lint.py` uses Python AST plus TS/JS heuristic; for `.rs` it takes fallback `errs=[]`, so Exit 0 is file-discovery pass, **not** Rust AST purity proof. `rustfmt --edition 2021 --check` and Lead `cargo check --lib` are syntax/type gates. R1 added new stub functions; Lead must rerun `cargo check --lib`.
- `team_history_snapshot_from_events` callback cannot distinguish empty stream from read failure; failure diagnostics owned by `team_history_snapshot_from_db_path` / store entry.
- `TeammateStatus` transitions from `send_message` / `drain_inbox` are best-effort historical projection; no live inbox replay/mutation.
- Archived sessions are not filtered because user asked for all local historical sessions.
- `AgentRosterEntry` readonly field is an Executor-side TUI-protocol change; L2-P cannot edit `tui/protocol.rs`. R1 contract requires it, but its compile-level assertion belongs to Test/Executor/Lead.
- No `Observability.yaml` / docs change in Protocol Agent write scope; logging remains TUI-stderr unified without new core log surface.

## 10. Change ledger

- 2026-11-07 L2-P: initial freeze; additive API; NFR baseline referenced read-only; no baseline key added/deleted/renumbered.
- 2026-11-07 Lead-approved additive: A12 sessions one-to-one mapping, A13 DB-unavailable/clean/bad-payload diagnostics exactness; `pub mod team_history;` assembled by Lead.
- 2026-11-07 R1 re-freeze after Gate-1 FAIL: full discovery union, `?mode=ro` + readonly schema probe, `is_historical` flag + Enter readonly gate, async refresh cache `Loading/Ready/Offline`; A14–A18 added; NFR baseline still unchanged.
