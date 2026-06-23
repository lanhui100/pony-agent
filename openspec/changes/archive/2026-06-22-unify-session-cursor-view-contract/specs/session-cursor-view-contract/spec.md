# Session Cursor View Contract

## Requirements

### Requirement: The system must separate recoverable history, current cursor, and rendered view
The system SHALL model session recovery using three distinct layers: immutable history graph, a single authoritative cursor, and a derived view returned to clients.

#### Scenario: A client requests the current session view without specifying a node
- **WHEN** a client requests the current session view for a session without an explicit `nodeId`
- **THEN** the host SHALL resolve the returned view from the authoritative session cursor
- **AND** the client SHALL NOT be required to infer the visible node from cached UI state

#### Scenario: A client requests a session view for an explicit historical node
- **WHEN** a client requests a session view with an explicit `nodeId`
- **THEN** the host SHALL derive the returned view from that node
- **AND** the returned view SHALL identify whether it matches or diverges from the active branch head

### Requirement: Checkpoint graph and cursor must not share the same responsibility
The system SHALL treat checkpoints as recoverable history points and the cursor as the single source of truth for the currently visible position.

#### Scenario: Branch topology and branch head authority are resolved from the history graph
- **WHEN** the system needs to determine the latest node of a branch or whether the visible node diverges from branch latest
- **THEN** the `HistoryGraph` SHALL be the sole authority for branch topology and branch head state
- **AND** the `Cursor` SHALL express the current visible node, active branch, and view mode without becoming a second authority for branch latest

#### Scenario: A historical checkout is performed
- **WHEN** a user checks out a historical node
- **THEN** the system SHALL update the session cursor to that node
- **AND** it SHALL preserve the branch head in graph-derived authority if the checked-out node is not the branch head

#### Scenario: A client restores the branch head after historical inspection
- **WHEN** a user restores the active branch head
- **THEN** the system SHALL update the cursor back to the branch head
- **AND** it SHALL not require the client to reconstruct the latest position from prior transcript state

### Requirement: Clients must modify history state through host commands rather than local reconstruction
The system SHALL require clients to issue explicit host commands for checkout, restore, fork, and branch switching instead of directly mutating local transcript state and inferring cursor meaning afterward.

#### Scenario: A client performs a checkout command
- **WHEN** a client issues a checkout command for a node
- **THEN** the host SHALL persist the resulting cursor state
- **AND** the host SHALL expose a read model that subsequent clients or surfaces can reload consistently

#### Scenario: Multiple surfaces access the same session
- **WHEN** Tauri, TUI, CLI, or HTTP consumers load the same session
- **THEN** each surface SHALL observe a consistent cursor-derived view for that session
- **AND** none of the surfaces SHALL need a private fallback algorithm to reconstruct history mode

### Requirement: Cursor mutations must be versioned for multi-surface concurrency
The system SHALL define a version or revision mechanism for authoritative cursor mutations so concurrent surfaces do not silently overwrite each other.

#### Scenario: A stale surface submits a cursor mutation after another surface has already moved the cursor
- **WHEN** a client submits a cursor mutation based on an outdated cursor revision
- **THEN** the host SHALL reject or explicitly reconcile the stale mutation rather than silently overwriting newer state
- **AND** the host SHALL return enough conflict information for the client to refresh and retry intentionally

### Requirement: Local cache must be a performance aid, not a semantic authority
The system SHALL define local cache as a UX/performance optimization layer and SHALL NOT allow it to override the authoritative cursor semantics of formal host-backed sessions.

#### Scenario: Cached local transcript diverges from host cursor state
- **WHEN** a client cache contains transcript or UI state that diverges from the host cursor-derived view
- **THEN** the host-derived view SHALL win for formal host-backed sessions
- **AND** the client SHALL discard or downgrade conflicting local recovery assumptions

### Requirement: The host command and read model must be multi-surface ready
The system SHALL expose a host-level command/read model that can be shared across GUI, TUI, CLI, and HTTP surfaces.

#### Scenario: A host returns a cursor-derived session view
- **WHEN** the host returns a session view to any surface
- **THEN** the read model SHALL include enough host-projected fields to avoid client-side reconstruction of history mode
- **AND** the contract SHALL define host-owned equivalents of at least resolved visible node, active branch, active branch head, and whether the view is currently at branch latest

#### Scenario: A future HTTP consumer integrates with session history controls
- **WHEN** an HTTP surface integrates session history commands
- **THEN** it SHALL be able to use the same conceptual `checkout / restore branch head / fork / switch branch / load view` contract as the GUI host
- **AND** the canonical contract SHALL avoid embedding desktop-only UI assumptions

### Requirement: The contract must preserve compatibility with existing history-node semantics
The system SHALL extend, not contradict, the existing history-node-management contract by clarifying that graph structure and cursor state are related but distinct authorities.

#### Scenario: Ownership between the history-node-management contract and this contract is resolved explicitly
- **WHEN** both canonical specs apply to the same session history behavior
- **THEN** `history-node-management` SHALL own graph invariants, branch topology, and checkout mode semantics
- **AND** `session-cursor-view-contract` SHALL own authoritative cursor semantics, host command/read-model requirements, multi-surface consistency, and fallback retirement rules
- **AND** implementers SHALL NOT duplicate the same recovery rule in both specs with divergent wording
