# MCP Capability Bridge

## Requirements

### Requirement: MCP must enter Pony Agent through a capability-ingress bridge
The system SHALL treat MCP as a capability-ingress layer that normalizes external capabilities before planner or runtime consume them.

#### Scenario: A host exposes one or more MCP servers
- **WHEN** Pony Agent discovers MCP-backed capabilities
- **THEN** the system SHALL normalize them through a bridge contract
- **AND** planner/runtime SHALL consume normalized capability facts rather than raw MCP protocol payloads

### Requirement: The capability registry must unify builtin and MCP-backed capabilities
The system SHALL expose builtin and MCP-backed capabilities through a shared registry-facing model.

#### Scenario: Planner inspects available capabilities
- **WHEN** planner or host reads the capability registry
- **THEN** builtin tools and MCP-backed capabilities SHALL share a common normalized shape
- **AND** each entry SHALL preserve provenance so the system can distinguish builtin capabilities from MCP-backed ones

#### Scenario: A host refreshes one MCP source snapshot
- **WHEN** the host applies a normalized snapshot for one `sourceId`
- **THEN** the shared registry SHALL replace the prior MCP-backed capability set for that `sourceId`
- **AND** the runtime execution registry SHALL observe the same refreshed source/capability data

### Requirement: MCP tools, resources, and prompt-like capabilities must remain semantically distinct
The system SHALL not collapse all MCP-backed entries into a single pseudo-tool abstraction.

#### Scenario: An MCP source exposes both executable and read-only artifacts
- **WHEN** the bridge maps MCP-backed entries into normalized capabilities
- **THEN** it SHALL distinguish at least `tool`, `resource`, and `prompt_template` kinds
- **AND** runtime SHALL not treat read-only resources or prompt templates as ordinary side-effecting tool calls

### Requirement: Planner must depend only on capability facts, not MCP wire details
The system SHALL keep MCP transport/session semantics below planner-facing decision boundaries.

#### Scenario: Planner chooses whether a capability is relevant
- **WHEN** planner evaluates available capabilities for a turn or run
- **THEN** it SHALL rely on normalized labels, descriptions, schemas, safety facts, and provenance
- **AND** it SHALL NOT require direct knowledge of MCP protocol envelopes, transport kinds, or host connector specifics

### Requirement: Host and runtime surfaces must normalize permission and failure states
The system SHALL expose consistent permission and failure contracts for MCP-backed capability usage.

#### Scenario: An MCP-backed capability requires approval or fails at invocation time
- **WHEN** host or runtime attempts to use the capability
- **THEN** the bridge SHALL indicate whether approval is required and whether the host mediates execution
- **AND** failure results SHALL distinguish at least source unavailability, permission denial, malformed response, and execution failure

#### Scenario: A source becomes unreachable during refresh
- **WHEN** the host refreshes a normalized MCP source snapshot with `availability = unreachable` or `degraded`
- **THEN** the source SHALL remain inspectable through the host read-plane
- **AND** the source MAY expose an empty capability list without leaking transport-specific failure details upward

### Requirement: MCP capability usage must be observable without leaking protocol internals upward
The system SHALL emit enough bridge-level observability for hosts and monitoring surfaces to understand how MCP-backed capabilities were used.

#### Scenario: A turn fetches one MCP resource and executes one MCP tool
- **WHEN** telemetry or inspection reads the resulting capability activity
- **THEN** the system SHALL expose which source and normalized capability were used
- **AND** it SHALL indicate whether the action was a resource fetch, tool execution, or prompt expansion

### Requirement: This change must not absorb downstream skills or hooks scope
The system SHALL keep `PA-020` limited to MCP capability ingress and registry semantics.

#### Scenario: Engineers plan follow-up work after the bridge exists
- **WHEN** implementation tasks are derived from this spec
- **THEN** they SHALL treat skills composition as downstream `PA-021`
- **AND** they SHALL treat lifecycle hooks integration as downstream `PA-022`
