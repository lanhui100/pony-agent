# Context Assembly And Cache Strategy

## ADDED Requirements

### Requirement: Formal Context Layering

Pony Agent SHALL define a formal layered context model for provider requests.

The model SHALL include at least:

- `Tools`
- `Base System`
- `Runtime Facts`
- `Project Instructions`
- `Memory Injection`
- `Conversation Carry`
- `Turn-local Volatile Input`

#### Scenario: Layered request contract exists

- **WHEN** a new request-construction contract is documented
- **THEN** each context source SHALL belong to exactly one primary layer
- **AND** the documentation SHALL state whether the layer is expected to be cache-stable, semi-stable, or turn-local volatile

### Requirement: Base System Must Stay Stable

Base system instructions SHALL contain only stable identity, behavior, safety, and output constraints.

#### Scenario: Dynamic session data is excluded from base system

- **WHEN** session summary, run goal, truncation note, or equivalent dynamic state is available
- **THEN** those fields SHALL NOT be injected into `Base System`
- **AND** they SHALL be handled by a later layer or explicit low-frequency refresh boundary

### Requirement: Coding And Work Profiles

Pony Agent SHALL support at least a `coding` profile and a `work` profile for base system construction.

#### Scenario: Profile selection remains stable

- **WHEN** a thread selects a profile
- **THEN** the selected base-system profile SHALL remain stable for that thread unless an explicit mode switch occurs

#### Scenario: Explicit workspace mode overrides inference

- **WHEN** an explicit thread or app-level workspace mode is configured as `coding` or `work`
- **THEN** profile selection SHALL prefer that explicit mode over heuristic inference
- **AND** any heuristic detection SHALL be used only as fallback when no explicit mode is present

#### Scenario: Explicit mode flows through submission and context retrieval

- **WHEN** the user submits a turn after selecting a workspace mode in the product UI
- **THEN** the selected mode SHALL be persisted in app settings, included in turn submission payloads, and available to context retrieval/building
- **AND** the resulting request assembly SHALL use the matching domain profile block

### Requirement: Runtime Facts Are Separate From Base System

Runtime environment facts SHALL be modeled separately from the base system.

#### Scenario: Environment facts do not force base-system rewrite

- **WHEN** OS, shell, surface, sandbox, approval policy, or workspace roots are included in context
- **THEN** they SHALL be injected as `Runtime Facts`
- **AND** the system SHALL NOT require rewriting the base-system identity/instruction block just to express those facts

### Requirement: AGENT.md Uses Workspace Scope Rules

`AGENT.md` or equivalent project instruction files SHALL follow workspace scope rules.

#### Scenario: Directory scope applies

- **WHEN** an `AGENT.md` file exists in a directory
- **THEN** its instructions SHALL apply to that directory and all descendants

#### Scenario: Deeper instructions override shallower instructions

- **WHEN** multiple applicable instruction files exist on the path from workspace root to target path
- **THEN** deeper-scoped files SHALL override shallower-scoped files on conflicts
- **AND** that override SHALL use file-level precedence rather than declaration-level merging

#### Scenario: Prompt-level instructions outrank AGENT.md

- **WHEN** system, developer, or direct user instructions conflict with `AGENT.md`
- **THEN** prompt-level instructions SHALL take precedence

### Requirement: AGENT.md Is Not Part Of Base System

Project instruction files SHALL NOT be folded into the base-system layer.

#### Scenario: Project instructions are injected separately

- **WHEN** `AGENT.md` content is injected
- **THEN** it SHALL be injected as `Project Instructions`
- **AND** the system SHALL preserve a separate record of applicable instruction sources

### Requirement: Workspace-Bound Instruction Refresh

Instruction injection SHALL be refreshed only on explicit workspace-scope boundaries.

#### Scenario: Instruction refresh is boundary-driven

- **WHEN** active cwd, workspace root, or target paths cross into a new applicable instruction scope
- **THEN** the system SHALL refresh `Project Instructions`
- **AND** it SHALL record the refresh reason as a context mutation event

### Requirement: Runtime Facts Must Define Stable-Boundary Triggers

Runtime facts SHALL define which changes trigger a cache-affecting boundary refresh.

#### Scenario: Workspace-root change is explicit boundary

- **WHEN** workspace roots or equivalent thread-level execution roots change
- **THEN** the architecture SHALL classify that as an explicit runtime-facts boundary change
- **AND** it SHALL state whether that change triggers a cache reset or a narrower instruction refresh

### Requirement: Long-Term Memory Is Extensible But Out Of Scope For This Change

The architecture SHALL provide extension points for cross-session long-term memory without requiring the memory product to be implemented in this change.

#### Scenario: Memory interfaces exist without full product delivery

- **WHEN** long-term memory is discussed in architecture artifacts
- **THEN** the system SHALL define extension interfaces or modes for memory injection
- **AND** the change SHALL NOT require full memory write/retrieval implementation to be considered complete

### Requirement: Memory Must Not Pollute Stable Prefix By Default

Long-term memory SHALL NOT enter the stable prefix by default.

#### Scenario: Default memory injection avoids base-system pollution

- **WHEN** cross-session memory content is available
- **THEN** it SHALL default to non-base-system injection modes
- **AND** only explicitly designated durable policy memory may be considered for more stable layers in future work

### Requirement: Conversation Carry Must Prefer Stable Continuation

Conversation carry SHALL prefer provider continuation or other stable-carry mechanisms over per-turn full replay where supported.

#### Scenario: Continuation is preferred over full replay

- **WHEN** the active provider supports a continuation primitive such as `previous_response_id`
- **THEN** the request-construction strategy SHALL prefer that continuation path
- **AND** the architecture SHALL describe full replay as a fallback rather than the preferred steady-state mechanism

#### Scenario: Continuation failure falls back explicitly

- **WHEN** a continuation request fails or is rejected by the provider
- **THEN** the system SHALL fall back to explicit full replay rather than silently abandoning the turn
- **AND** it SHALL emit an observable reason for that fallback

### Requirement: Cache Reset Points Must Be Explicit And Low-Frequency

Any intentional cache-reset point SHALL be explicit, low-frequency, and observable.

#### Scenario: Compaction is treated as explicit cache reset

- **WHEN** the system compacts history or otherwise rewrites the conversation-carry boundary
- **THEN** it SHALL treat that event as an explicit cache-reset point
- **AND** it SHALL record why the reset occurred

#### Scenario: Low-frequency target is documented

- **WHEN** the architecture defines low-frequency cache-reset behavior
- **THEN** it SHALL document a target steady-state expectation of at least 20 consecutive turns without cache reset in the absence of explicit boundary changes, provider errors, or compaction triggers

### Requirement: Dynamic Notes Must Not Be Front-Loaded Every Turn

High-frequency dynamic notes SHALL NOT be front-loaded into the stable request prefix every turn.

#### Scenario: Dynamic notes are excluded from stable prefix

- **WHEN** the system generates session summary text, truncation notes, planner-skill summaries, or temporary diagnostics
- **THEN** those notes SHALL NOT be prepended to the stable prefix on every turn
- **AND** they SHALL instead be placed in later layers or boundary-driven refresh points

#### Scenario: Dynamic note classes have explicit layer assignment

- **WHEN** the architecture refers to session summary text, truncation notes, planner-skill summaries, or temporary diagnostics
- **THEN** it SHALL assign each class to an explicit target layer or explicitly document the allowed implementation choice set

### Requirement: Turn-Local Volatile Input Must Stay At The Tail

Current-turn user input and temporary diagnostics SHALL remain at the tail of the request assembly order.

#### Scenario: Current turn input stays tail-positioned

- **WHEN** user message text, images, or temporary turn-local hints are added
- **THEN** they SHALL be injected after stable and semi-stable layers
- **AND** the architecture SHALL classify them as turn-local volatile input

#### Scenario: Single-turn instruction overrides stay tail-positioned

- **WHEN** a user provides a single-turn instruction override such as “本轮忽略测试文件”
- **THEN** the architecture SHALL classify that override as turn-local volatile input rather than project instructions or conversation carry

### Requirement: Context Observability Must Cover Layering Decisions

The context system SHALL expose observability for layering and mutation decisions.

#### Scenario: Mutation and carry strategy are observable

- **WHEN** context is built for a provider request
- **THEN** the system SHALL be able to report stable prefix text, semi-stable context text, volatile input text, and prefix mutation reasons
- **AND** it SHALL define observability for instruction-scope sources, conversation-carry mode, and context refresh reason
