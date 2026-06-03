# Design: Model Monitor and Telemetry Aggregation Surface

## Context

The repository already states that `PA-024` should own:

- retrieval observability
- trace display semantics
- monitor surfaces

It should not reopen the questions already delegated elsewhere:

- `PA-025` owns how retrieved context is mapped into prompt/request and how build context is explained
- `PA-029` owns call-level cache telemetry capture and prefix-change explanation

`PA-024` therefore acts as the consumer-facing read plane that turns those runtime signals into a coherent monitoring experience.

## Design Goals

1. Make runtime telemetry readable as an operational surface, not only as raw per-turn trace payloads
2. Keep retrieval observability connected to model behavior, tool behavior, and cache behavior in the same monitor flow
3. Define a stable semantic contract for trace display so frontend and backend stop inferring step meaning ad hoc
4. Let `ModelMonitorPage` consume Tauri-provided read models with minimal UI-side reconstruction
5. Preserve compatibility with the current single-user local app architecture

## Non-Goals

- redesign request assembly or provider execution
- invent new cache telemetry primitives beyond `PA-029`
- replace existing trace panel interactions everywhere in one pass
- build alert rules, anomaly scoring, or scheduled reporting
- define remote ingestion, warehouse schemas, or hosted observability infrastructure

## Proposed Design

### 1. Monitoring read model layers

`PA-024` defines four read layers:

1. overview aggregates
2. dimension aggregates
3. session drill-down
4. trace detail semantics

The intent is to avoid making `ModelMonitorPage` derive product meaning from raw event streams.

### 2. Overview aggregates

The monitor overview should expose rollups for a selected local scope such as all loaded sessions or a selected session set.

Minimum aggregates:

- total request count
- total model call count
- total tool call count
- total input/output tokens
- cached input tokens and cache-hit ratio when available
- total and percentile-oriented latency summaries
- retrieval participation counts, such as how often retrieval contributed context to a turn

Overview data is descriptive, not prescriptive. It supports diagnosis, not automated action.

### 3. Provider / model / tool / session dimension aggregates

The monitor should support explicit group-by read models for:

- provider
- model
- tool
- session

Each dimension row should be able to expose at least:

- request or call counts
- token usage
- cache hit usage when available
- latency summaries
- error/failure counts when represented in the underlying trace/session store

Dimension semantics:

- provider aggregate answers "which provider carried the workload"
- model aggregate answers "which concrete model produced the workload characteristics"
- tool aggregate answers "which tools were invoked and what follow-up cost they induced"
- session aggregate answers "which conversation/run accumulated the cost and latency profile"

### 4. Retrieval observability as a monitor dimension

Retrieval observability should not remain a hidden detail inside the turn trace.

The monitor surface should expose, per session and per drill-down view:

- whether retrieval participated in the turn
- the current retrieved-context state summary that affected the request
- build-context layer summaries needed to understand what the model actually received
- retrieval-related trace steps and their relation to downstream model calls

This change does not redefine retrieval algorithms. It defines how their effects are read.

### 5. Trace display semantics

`PA-024` defines the semantic categories that trace UIs must honor. Minimum categories:

- `receive_input`
- `prepare_retrieval`
- `build_context`
- `call_model`
- `call_tool`
- `return_result`

Semantic rules:

- retrieval preparation and build-context explanation are distinct steps even when adjacent
- a model step represents one provider call, not a whole turn summary
- a tool step represents one invoked tool call or a tightly grouped tool-call unit if runtime already groups it
- follow-up model calls after tools remain `call_model` steps, with request-kind metadata from `PA-029`
- return/result steps summarize terminal user-visible output and turn completion state, not duplicate call-level token metrics

This gives monitor UI and existing trace UI one shared language for step meaning.

### 6. Tauri read surface for `ModelMonitorPage`

`ModelMonitorPage` should consume explicit Tauri read models rather than rebuild aggregates from scattered frontend state.

The backend-facing contract should provide two classes of payload:

- monitor summary payloads for overview and dimension tables
- session drill-down payloads for a selected session, including trace timeline plus retrieval/build-context summaries

Design expectations:

- summary reads should be cheap enough for page load and filter refresh
- drill-down reads may be more detailed and session-scoped
- the page should not require replaying every raw event just to render aggregate cards

This is a read contract only. It does not imply new runtime write paths beyond what telemetry/session storage already records.

### 7. Frontend information architecture

`ModelMonitorPage` should be defined as a monitoring workspace with:

- top summary cards
- dimension tables or segmented lists for provider/model/tool/session pivots
- a selected-session panel or route state for drill-down
- trace and retrieval detail views that explain aggregate anomalies

Recommended interaction flow:

1. user opens monitor overview
2. user scans aggregate cards
3. user pivots by provider/model/tool/session
4. user selects a session row
5. user inspects trace semantics and retrieval/build-context detail for that session

### 8. Dependencies and sequencing

`PA-024` depends on existing and adjacent work as follows:

- `PA-029` supplies cache-hit and request-kind telemetry inputs
- `PA-025` supplies build-context layer explanation inputs
- existing session and trace persistence supply the raw history store

This change should document the monitor read plane after those primitives exist, without absorbing their implementation details.

## Data Model Impact

Expected touched concepts:

- session trace history read model
- per-call telemetry read model
- dimension aggregate structs for provider/model/tool/session
- retrieval/build-context summary structs exposed to monitor drill-down
- Tauri command/event payloads for `ModelMonitorPage`

## Acceptance Design Notes

The design should be considered satisfied when:

- aggregate and drill-down contracts are explicit enough that frontend and backend can implement independently
- retrieval observability is integrated into the same read experience as model and tool telemetry
- trace categories are stable enough to avoid duplicate UI-specific interpretations
- the page can explain both "where did the cost go" and "what retrieval/context state drove this behavior"

## Risks

### Risk: Monitor scope turns into a runtime rewrite

Mitigation:

- treat `PA-024` as a read-plane change
- consume `PA-025` and `PA-029` outputs rather than redefining them

### Risk: Aggregates become misleading if semantics differ between turns

Mitigation:

- preserve session drill-down as the source of truth
- require dimension rows to be trace-backed, not standalone invented metrics

### Risk: Retrieval observability duplicates build-context UI in a second place

Mitigation:

- define retrieval/build-context summaries as shared monitor inputs
- keep detailed request-shape semantics anchored to `PA-025`
