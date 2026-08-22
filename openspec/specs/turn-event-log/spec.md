# turn-event-log Delta

## ADDED Requirements

### Requirement: turn_events table

The persistence backend SHALL store turn-level process facts in an append-only `turn_events` table.

#### Scenario: Event persistence

- GIVEN a running turn that emits stream chunks, tool calls, and plan updates
- WHEN the turn completes
- THEN every emitted event SHALL be persisted in `turn_events` (unless persistence failed, see failure containment)
- AND each row SHALL carry `(session_id, turn_id, branch_id, seq, event_type, payload, created_at_ms)`

#### Scenario: Sequence contiguity

- GIVEN an empty session
- WHEN the first event is appended
- THEN its `seq` SHALL be 0 (0-based)
- AND the table row count SHALL be 1
- GIVEN a session with N persisted events
- WHEN the next event is appended
- THEN its `seq` SHALL equal N
- AND concurrent appends SHALL be serialized (no duplicate or gap; verified with a two-thread barrier test appending 100 events each)

#### Scenario: Append-only immutability

- WHEN an event has been persisted
- THEN no operation SHALL modify or delete it in place
- AND the event layer SHALL never be truncated by `DEFAULT_HISTORY_LIMIT`
- AND archival SHALL move rows to `turn_events_archive` preserving `seq` and the primary key (fold engines read both transparently)

### Requirement: Event type vocabulary

The change SHALL define a typed event vocabulary with `domain/action` naming, covering at minimum: `turn/start`, `turn/end`, `step/start`, `step/end`, `user/message`, `assistant/chunk`, `assistant/message`, `tool/call`, `tool/result`, `plan/update`, `provider/usage`, `history/squash`, `checkpoint/created`, `checkpoint/checkout`, `fork/created`.

#### Scenario: Typed payloads

- GIVEN an event type
- WHEN it is constructed
- THEN its payload SHALL be a typed struct (Rust enum + struct, serde-serializable)
- AND unknown event types SHALL fail at construction/deserialization (fail loud, data-integrity errors)

#### Scenario: Emission mapping

- GIVEN any emitted `TurnStreamEvent` (all 8 existing names: `turn:started`, `turn:delta`, `turn:output_end`, `turn:completed`, `turn:failed`, `turn:cancelled`, `turn:trace`, `turn:tool`)
- WHEN `emit_event` executes
- THEN the event name SHALL map to a unique `TurnEvent` variant per the mapping table (table-driven test)
- AND `turn:failed`/`turn:cancelled` SHALL map to `TurnEnd` with the corresponding reason (`error`/`cancelled`)
- AND events without a mapping (e.g. `turn:output_end`, `turn:trace`) SHALL be explicitly declared non-persisted
- NOTE (phase-1 scope): runtime emission covers 6 core event types (`turn/start`, `assistant/chunk`, `assistant/message`, `turn/end`, `tool/call`, `tool/result`); `user/message`, `step/start`, `step/end`, `history/squash`, `provider/usage` emit points are deferred to phase 2 (backfill already writes them for legacy sessions)

#### Scenario: Metrics carried by events

- GIVEN a provider call that reports token usage and cache accounting
- WHEN the call settles
- THEN a `ProviderUsage` event SHALL be persisted with usage buckets (input / cache_read / cache_write / output), cache hit/miss tokens, prefix mutation reasons, and latency
- AND the `assistant/message` event SHALL carry its `usage` when the adapter reported token accounting (metadata display only, not totals)
- AND latency SHALL be carried by `step/start` (first token latency) and `turn/end` (turn duration)

#### Scenario: Lossless JSON

- WHEN an event payload is persisted
- THEN it SHALL be lossless JSON (no BigInt, no non-serializable values)
- AND deserialization SHALL round-trip semantically equivalent (field-level equality, not byte-identical)

### Requirement: Buffered persistence with terminal flush

Events SHALL enter an in-memory buffer during streaming and flush in the same transaction as the blob snapshot at turn terminal state (completed / failed / cancelled).

#### Scenario: Stream and buffer

- GIVEN an emitted `TurnStreamEvent`
- WHEN it is emitted
- THEN it SHALL be pushed to the frontend (existing SSE/Tauri path, unchanged)
- AND it SHALL enter the turn event buffer (not yet durable)

#### Scenario: Terminal flush atomicity

- GIVEN a turn reaching a terminal state
- WHEN the flush transaction commits
- THEN the `turn_events` rows and the seq counter SHALL be visible together (event-layer atomicity, guaranteed by the FlushEvents transaction)
- AND a crash before commit SHALL leave neither visible (mid-turn crash loses the unflushed buffer — accepted degradation, explicitly declared)
- NOTE: snapshot (`sessions.session_data`) atomicity with events is NOT promised in this change — the snapshot is written by the existing independent transaction path; unifying event+snapshot into one transaction is deferred to phase 2 (projection layer rewrites the snapshot write path)

#### Scenario: Persistence failure containment

- GIVEN a SQLite write failure during flush
- WHEN the flush executes
- THEN the frontend push SHALL still succeed (persistence failure SHALL NOT break streaming)
- AND the failure SHALL be logged with `(session_id, turn_id, event_count, first_event_type)`
- NOTE: the per-event `persist_failed` marker and retry are deferred to the retry mechanism (later change); this change records failures in logs only (audit closure via log correlation)

#### Scenario: Failure layering

- GIVEN a `turn_events` row with an unknown `event_type` during load
- THEN loading SHALL fail loudly (data-integrity error, no silent skip)
- GIVEN a SQLite IO failure during write
- THEN the write SHALL be contained (logged, stream continues)
- AND the two failure classes SHALL be distinguishable by error type (tests inject each independently)

### Requirement: Existing consumers unaffected

All existing consumers SHALL continue to read the blob snapshot unchanged during the dual-write period.

#### Scenario: Consumer list

- GIVEN the dual-write is active
- THEN the frontend rendering, history loading, checkout, trace views, and model monitor drilldown SHALL continue to read the blob snapshot unchanged
- AND each consumer's regression tests SHALL pass

### Requirement: Legacy backfill

Existing blob snapshots SHALL be backfilled into `turn_events` once, deriving events from blob content.

#### Scenario: Backfill derivation

- GIVEN a legacy session with blob history and trace records
- WHEN backfill runs
- THEN `user/message`, `assistant/message`, `tool/call`, `tool/result`, `provider/usage`, `turn/start`, `turn/end` SHALL be derived from the blob
- AND events whose process detail (stream chunks) is not recoverable SHALL carry a `chunk_missing` marker
- AND turn boundaries SHALL be synthesized by user-message splitting (unpaired messages become single-message turns, marked `turn_boundary_synthetic`)

#### Scenario: Backfill data loss acknowledged

- GIVEN a legacy session with more than 24 turns (blob truncated by `DEFAULT_HISTORY_LIMIT`)
- WHEN backfill runs
- THEN only the most recent 24 turns SHALL have events
- AND `store_metadata` SHALL record `backfill_partial`

#### Scenario: Backfill per-session idempotency

- WHEN backfill runs twice
- THEN the second run SHALL be a no-op for completed sessions (per-session watermark `turn_event_backfill:<session_id>`)
- GIVEN backfill crashes after session 10 of 20
- WHEN backfill reruns
- THEN sessions 1-10 SHALL be skipped and sessions 11-20 SHALL continue from their watermark (no duplicate events)