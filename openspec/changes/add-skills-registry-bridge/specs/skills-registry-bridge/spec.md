# Skills Registry Bridge

## Requirements

### Requirement: Skills must enter Pony Agent through the unified capability-registry boundary
The system SHALL treat skills as a capability-composition layer above the normalized capability registry rather than as host-private scripts or a second discovery system.

#### Scenario: A host registers reusable skills
- **WHEN** Pony Agent discovers or refreshes skill manifests
- **THEN** the system SHALL normalize them through a skills registry bridge
- **AND** planner/runtime SHALL consume normalized skill facts rather than raw manifest bodies or host-private implementation details

### Requirement: Skills must preserve the semantics of composed capabilities
The system SHALL not flatten all referenced capabilities into a single pseudo-tool abstraction.

#### Scenario: One skill references a tool, a resource, and a prompt template
- **WHEN** the bridge models the skill
- **THEN** it SHALL preserve which referenced entries are `tool`, `resource`, and `prompt_template`
- **AND** runtime SHALL not treat the whole skill as an opaque side-effecting tool if its internal steps include read-only fetches or prompt expansion

### Requirement: Planner must depend only on normalized skill facts
The system SHALL keep skill internals and underlying transport details below planner-facing decision boundaries.

#### Scenario: Planner evaluates whether a skill is relevant
- **WHEN** planner reads available skills for a turn or run
- **THEN** it SHALL rely on normalized labels, descriptions, schemas, safety facts, provenance, and high-level composition facts
- **AND** it SHALL NOT require direct knowledge of MCP transport details, host connector specifics, or imperative skill implementation steps

### Requirement: Skill registration and execution surfaces must remain host-agnostic
The system SHALL expose consistent read/write and invocation boundaries for skill sources and skill usage.

#### Scenario: A host refreshes one skill source snapshot
- **WHEN** the host applies a normalized snapshot for one skill `sourceId`
- **THEN** the shared skill registry SHALL replace the prior skill set for that `sourceId`
- **AND** runtime-visible skill resolution SHALL observe the same refreshed source and skill data

### Requirement: Skills must reuse normalized permission and failure contracts
The system SHALL not invent a separate approval or failure taxonomy for skills.

#### Scenario: A skill references guarded capabilities
- **WHEN** host or runtime attempts to use the skill
- **THEN** the bridge SHALL expose whether approval is required and whether execution is host-mediated
- **AND** failure results SHALL distinguish at least skill resolution failure, source unavailability, permission denial, malformed composition, and underlying capability execution failure

### Requirement: Skill usage must be observable through the existing monitoring chain
The system SHALL emit enough bridge-level observability for hosts and monitoring surfaces to understand which skill ran and which capabilities it composed.

#### Scenario: A turn selects one skill that triggers multiple capabilities
- **WHEN** telemetry or inspection reads the resulting activity
- **THEN** the system SHALL expose which skill and source were used
- **AND** it SHALL indicate which normalized capabilities or capability kinds were triggered underneath the skill

### Requirement: This change must not absorb hooks, workflow, marketplace, or planner redesign scope
The system SHALL keep `PA-021` limited to the skills registry bridge and composition boundary.

#### Scenario: Engineers plan follow-up work after the skills bridge exists
- **WHEN** implementation tasks are derived from this spec
- **THEN** they SHALL treat lifecycle hooks as downstream `PA-022`
- **AND** they SHALL treat workflow mode as downstream `PA-026`
- **AND** they SHALL NOT use this change to redesign planner, graph progression, or a marketplace platform
