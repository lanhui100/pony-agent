# Design: Cache Hit Telemetry and Prefix Stabilization

## Context

The current runtime already models build context in three layers, but the actual provider request path still has two structural gaps:

1. observability is aggregated too late
2. volatile request notes still influence the earliest part of the request too often

This change addresses both without expanding into full context compaction.

## Design Goals

1. Make cache behavior explainable at provider-call granularity
2. Preserve current answer quality and request semantics where possible
3. Improve prefix stability without introducing complex new session machinery
4. Keep the design compatible with future compaction and graph-session work

## Non-Goals

- redesign the whole session format
- introduce background compaction
- split planner/executor sessions now
- optimize every future request class in one pass

## Proposed Design

### 1. Call-level cache telemetry

Augment the runtime trace model so each provider call records:

- request kind
- input tokens
- cache hit input tokens
- cache miss input tokens when derivable
- output tokens
- latency
- optional prefix mutation reasons

Minimum request kinds in this change:

- `initial_request`
- `tool_followup`

This lets turn-level aggregates remain available while trace UIs and audits can inspect call-level behavior.

### 2. Prefix mutation reasons

Introduce a small, explicit enum/list for why a request prefix differed from the previous stable shape.

First-pass reasons:

- `session_summary_changed`
- `run_goal_changed`
- `long_term_memory_changed`
- `image_note_changed`
- `truncation_note_changed`
- `history_boundary_shifted`
- `native_transcript_boundary_shifted`

This is diagnostic metadata, not user-facing prompt content.

### 3. First-pass prefix stabilization

Keep the stable prefix narrowly defined:

- base system prompt
- provider capability note
- stable tool definition export

Treat the following as non-stable annotations unless explicitly required:

- session summary text
- graph name or conversation-link notes
- run goal note
- long-term memory summary note
- image capability note
- truncation note

The design intent is not to remove these facts entirely. It is to keep them out of the most cache-critical prefix whenever the request format allows that separation.

### 4. Preserve PA-025 observation contract

`BuildContextObservation` remains the canonical explanation of what was actually sent. This change extends its interpretability; it does not replace it.

The existing three-layer model remains:

- stable prefix
- semi-stable context
- volatile input

This change sharpens which fields are allowed in each layer and records why layer contents changed.

## Data Model Impact

Likely touch points:

- provider response usage extraction
- per-call runtime trace records
- turn-level aggregate token usage
- session persistence of trace history
- frontend trace types consuming request-kind-specific metrics later

## Execution Freeze

### In scope now

1. `telemetry.rs` shared types for call-level cache telemetry
2. `session.rs` trace persistence extension with backward-compatible defaults
3. `runtime.rs` per-call record generation and request-kind classification
4. `context.rs` mutation-reason recording and stable-prefix narrowing
5. `provider.rs` fallback observation correction only

### Explicitly out of scope now

- provider usage schema redesign
- new build-context observation layers
- auto-compaction and summarization machinery
- planner/executor dual-session architecture
- sub-agent cache reuse design

## Implementation Order

1. Land telemetry fields and request classification first
2. Add mutation-reason recording next
3. Tighten prefix field placement after telemetry proves the baseline
4. Validate persistence and regression compatibility before any UI expansion
5. Keep `PA-024` as the consumer of the resulting metrics rather than embedding monitor semantics here

## Verification Strategy

Minimum verification focus:

- runtime sync path keeps turn aggregate and adds per-call records
- runtime stream path keeps turn aggregate and adds per-call records
- context assembly records mutation reasons for summary/goal changes and truncation boundary shifts
- stable prefix excludes image/truncation notes by default
- session history roundtrips the new fields
- legacy session history defaults missing new fields safely
- frontend store preserves `requestKind` and `prefixMutationReasons`

## Risks

### Risk: Prefix stabilization changes output quality

Mitigation:

- keep first-pass changes narrow
- preserve semantic content even when repositioning it
- require regression coverage on request assembly and trace surfaces

### Risk: New metrics still mislead because providers differ

Mitigation:

- treat cache miss tokens as optional when provider APIs do not report them directly
- preserve raw per-call usage alongside derived labels

### Risk: Scope creeps into compaction

Mitigation:

- explicitly block auto-compaction from this change
- keep tasks limited to telemetry and current-path request stabilization
