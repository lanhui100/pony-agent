# Model Monitor Telemetry

## Requirements

### Requirement: The system must expose telemetry aggregates at overview and dimension levels
The system SHALL define a monitoring read model that exposes overview aggregates and dimension aggregates for current local telemetry data.

#### Scenario: A user opens the monitor landing page
- **WHEN** `ModelMonitorPage` requests its summary data from Tauri
- **THEN** the system SHALL return overview aggregates for request counts, token usage, cache usage when available, latency summaries, and retrieval participation
- **AND** it SHALL provide dimension aggregates that can be grouped by at least `provider`, `model`, `tool`, and `session`

### Requirement: Session drill-down must connect aggregates back to trace-backed evidence
The system SHALL define a session drill-down payload that lets a user inspect why a session produced a given cost, latency, cache, or retrieval profile.

#### Scenario: A user selects a session row from the monitor
- **WHEN** the selected session is opened in the monitor drill-down view
- **THEN** the system SHALL expose session-level aggregate metrics together with the session trace timeline
- **AND** it SHALL expose retrieval/build-context summaries needed to explain the model-facing context state for that session

### Requirement: Retrieval observability must be a first-class monitoring concern
The system SHALL expose retrieval observability in the monitor surface rather than leaving it only inside ad hoc trace details.

#### Scenario: A turn used retrieved context before a model call
- **WHEN** the user inspects the relevant session or trace drill-down in the monitor
- **THEN** the system SHALL indicate that retrieval participated in the turn
- **AND** it SHALL expose enough retrieval/build-context summary to understand what class of retrieved context influenced the downstream request

### Requirement: Trace display semantics must be explicit and stable
The system SHALL define the semantic categories used to display runtime trace steps in monitoring surfaces.

#### Scenario: A turn performs retrieval, calls a model, invokes a tool, and then calls the model again
- **WHEN** the trace is rendered in monitor drill-down
- **THEN** the system SHALL distinguish `prepare_retrieval`, `build_context`, `call_model`, `call_tool`, and `return_result` semantics
- **AND** the follow-up model call after the tool SHALL remain a separate `call_model` step rather than being merged into a single opaque turn summary

### Requirement: Tauri must provide `ModelMonitorPage` with summary and drill-down read surfaces
The system SHALL define explicit Tauri-facing read contracts for monitor summary data and selected-session drill-down data.

#### Scenario: Frontend monitor UI loads without replaying raw runtime events
- **WHEN** `ModelMonitorPage` initializes or refreshes its filters
- **THEN** the page SHALL be able to obtain pre-aggregated summary data from Tauri
- **AND** it SHALL be able to request a richer session-scoped drill-down payload for a selected session

### Requirement: `PA-024` must not absorb adjacent runtime or analytics scopes
The system SHALL keep this change limited to monitor semantics and read-plane contracts.

#### Scenario: A future implementation plan is derived from this spec
- **WHEN** engineers break down implementation tasks for `PA-024`
- **THEN** they SHALL NOT treat this change as authority to redesign provider execution, prompt assembly, or remote analytics infrastructure
- **AND** they SHALL treat `PA-025` build-context semantics and `PA-029` cache telemetry capture as upstream inputs to this monitor surface
