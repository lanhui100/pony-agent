# event-sourcing-closeout Delta

## ADDED Requirements

### Requirement: Sync turn entry emits events

The synchronous `run_turn` entry (Tauri command, non-Tauri harness, graph sync fallback) SHALL emit an event sequence equivalent to the streaming entry, persisted via the same event channel. Invariant: **every emitted `turn/start` SHALL have a paired `turn/end`** — every early-exit path (prepare_turn failure, hook fail-turn, plan failure, provider failure) SHALL emit `turn/end` (reason=Error) before returning.

#### Scenario: Entry equivalence (completed)

- GIVEN the same logical turn executed via `run_turn` and `start_turn_stream` in separate sessions
- WHEN both persisted event streams are compared by event type sequence
- THEN the sync stream SHALL match the streaming stream except for `assistant/chunk` events and sink push
- AND comparison SHALL exclude the exemption field set defined in the Snapshot parity requirement

#### Scenario: Failed turn pairing

- GIVEN a sync turn that fails at any early-exit point (prepare/hook/plan/provider)
- WHEN its event stream is inspected
- THEN it SHALL contain `turn/end` with reason=Error
- AND no `turn/start` SHALL exist without a paired `turn/end`

#### Scenario: Cleared-cache rebuild equivalence

- GIVEN completed turns from both entries in their respective sessions
- WHEN each session's trace table is cleared and views are rebuilt from events
- THEN both rebuilds SHALL match their pre-clear snapshots on the agreed field set (exemption list excluded)

### Requirement: History derived from events

`append_turn` SHALL materialize history entries from the turn's UserMessage/AssistantMessage events rather than trusting caller-provided text; persistence SHALL use incremental message commands instead of whole-blob upserts.

Prerequisites: event flush ordering SHALL be atomic — the event-table transaction commits **before** the in-memory buffer is cleared; materialization reads by `(session_id, turn_id)`. Fallback to caller-provided text is permitted **only** when the backend has no event support; unavailability of events for any other reason (buffer cleared before table commit, partial flush) SHALL be treated as an error (logged, observable), not a silent fallback.

#### Scenario: Event-sourced materialization

- GIVEN a completed turn with emitted UserMessage/AssistantMessage events (post-flush)
- WHEN `append_turn` materializes history
- THEN the appended entries SHALL derive from the events (text, attachments, reasoning_content)
- AND the `sessions` blob row SHALL NOT be updated for message-only changes (observed via persist-command interception: zero whole-store writes)

#### Scenario: Fallback restricted to non-event backends

- GIVEN a backend without event support (e.g. MemoryBackend)
- WHEN `append_turn` is invoked
- THEN it SHALL fall back to caller-provided text (existing behavior preserved)
- AND given an event-capable backend whose buffer/table are momentarily unavailable
- WHEN `append_turn` is invoked
- THEN it SHALL log and surface the anomaly rather than silently falling back

### Requirement: Snapshot parity with explicit exemptions

Rebuilding views from events SHALL match stored snapshots on the agreed field set. The agreed field set and exemption list SHALL be defined as constants in this change (single source of truth for tests):

- **Agreed field set**: timeline entry (kind, label, state, sequence, text, tool_activities, token metrics) + trace record (phase, provider name/model, token fields, turn_duration_ms, event_type).
- **Exemption list** (each with reason): `updated_at`/`event_id`/`emitted_at_ms` (clock semantics); `title`/`session_id` (runtime decoration, backfilled by annotate); `prepare_retrieval` entry (requires payload judgment unavailable from ref-only events); build_context provider six-field metadata (no event carrier); failed/cancelled last-hop state (converges after Step lifecycle events land, then removed from this list).

Parity tests SHALL consume the exemption constants programmatically (agreed = full field set − exemptions), and each exemption SHALL carry a reverse probe asserting the deviation currently exists (preventing exemption rot).

#### Scenario: Parity matrix

- GIVEN the scenario matrix (no-tool / single-tool / multi-hop / failed / cancelled / squash / fork-checkout / multi-turn session / legacy+new mixed rows)
- WHEN rebuilt views are compared against stored snapshots on the agreed field set
- THEN zero differences SHALL exist outside the exemption list

### Requirement: Step lifecycle events

Every provider call SHALL emit `step/start` before the call and `step/end` at settlement; assistant chunks SHALL carry the logical hop step (`step` is 0-based; legacy chunks without step default to 0).

#### Scenario: Multi-hop rebuild

- GIVEN a turn with ≥2 provider calls and tool calls between them
- WHEN the timeline is rebuilt from events
- THEN the number of call_model entries SHALL equal the provider call count
- AND each chunk's text SHALL aggregate into its own hop's call_model entry

### Requirement: Event schema versioning

The event store SHALL record `events.schema_version`; reads SHALL validate it. A **missing key SHALL be treated as version 1 (current) and backfilled on first write** — legacy stores remain readable. Version mismatch SHALL return an explicit error (no partial view). Malformed or unknown required events SHALL fail loud: the session SHALL be marked event-stream-degraded and the error surfaced through the host read plane (load APIs), instead of being silently skipped. The ignorable-event list is a constant, currently empty.

#### Scenario: Legacy store without version key

- GIVEN a store created before this change (no `events.schema_version` key)
- WHEN events are loaded
- THEN loading SHALL succeed (treated as version 1)
- AND the key SHALL be backfilled on the next write

#### Scenario: Version mismatch

- GIVEN a store whose recorded schema_version differs from the current EVENT_SCHEMA_VERSION
- WHEN events are loaded
- THEN an explicit error SHALL be returned (no partial view)

#### Scenario: Malformed payload

- GIVEN an event row whose payload fails to deserialize
- WHEN events are loaded for a session
- THEN the session SHALL be marked event-stream-degraded and the error surfaced via the host load API
- AND the event SHALL NOT be silently skipped (ignorable list currently empty)

## MODIFIED Requirements

### Requirement: Watermark as cursor version (from session-cursor-view-contract)

`cursor_version` SHALL be derived from the session event watermark (seq水位即版本); history-control conflict detection SHALL compare watermarks. The wire field name is retained. **Watermark monotonicity invariant**: checkout/fork/squash SHALL emit their history-control events (checkpoint/checkout, fork/created, history/squash) so the watermark strictly increases — rollback operations move the *projection position* but never rewind the *event log watermark*. This preserves optimistic-concurrency semantics (no ABA window): any interleaved change strictly advances the version.

#### Scenario: Stale detection under monotonic watermark

- GIVEN a history-control command carrying a stale expected version
- WHEN the command is validated against the current watermark-derived version
- THEN a conflict error SHALL be raised

#### Scenario: Rollback does not rewind the version

- GIVEN a session at watermark W where a checkout to an earlier node occurs
- WHEN the checkout commits (emitting checkpoint/checkout)
- THEN the resulting cursor version SHALL be > W (strictly advanced)
- AND a concurrent command holding expected=W SHALL be rejected (no ABA window)

### Requirement: Trace cache stores observation by reference only (from trace-event-projection)

New live trace cache writes SHALL NOT embed the build_context_observation payload; the reference SHALL be backfilled in the same transaction that externalizes the ContextObservation event.

#### Scenario: No duplicate storage on live path

- GIVEN a completed live turn with a build context observation
- WHEN the trace cache row is inspected
- THEN raw_json SHALL NOT contain the observation payload
- AND the row's observation reference SHALL load a payload identical to the original
