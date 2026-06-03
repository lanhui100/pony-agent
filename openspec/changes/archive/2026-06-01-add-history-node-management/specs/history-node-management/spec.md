# History Node Management

## Requirements

### Requirement: Session history must support immutable nodes and explicit branches
The system SHALL represent recoverable conversation history as immutable nodes connected by explicit branches rather than as a latest-only mutable message list.

#### Scenario: A completed turn creates a stable history point
- **WHEN** a turn reaches a stable terminal outcome that should be restorable
- **THEN** the system SHALL persist an immutable history node for that point
- **AND** the node SHALL belong to an explicit branch with a known head

#### Scenario: A new action starts from a historical non-head node
- **WHEN** the user starts a new action from a node that is not the current branch head
- **THEN** the system SHALL create a new branch
- **AND** the new action SHALL advance the new branch head without deleting the previous branch

### Requirement: History checkout must support transcript-only and transcript-and-workspace modes
The system SHALL support checking out a historical node in two modes: transcript-only and transcript-and-workspace.

#### Scenario: Transcript-only checkout is requested
- **WHEN** the user requests checkout of a historical node in transcript-only mode
- **THEN** the visible conversation context SHALL move to that node
- **AND** the workspace state MAY remain unchanged

#### Scenario: Transcript-and-workspace checkout is requested on a host without rollback support
- **WHEN** the user requests transcript-and-workspace checkout
- **AND** the current host cannot roll back workspace state
- **THEN** the system SHALL degrade safely to transcript-only behavior
- **AND** the response SHALL indicate that workspace rollback was not applied

### Requirement: The system must distinguish visible history position from branch latest
The system SHALL separately track the currently visible node and the latest node of the active branch.

#### Scenario: User inspects an earlier node without making a new change
- **WHEN** the user checks out an earlier node and performs no new action
- **THEN** the system SHALL preserve the original branch head
- **AND** the user SHALL be able to restore to that branch latest

### Requirement: Retrieval and runtime views must be reconstructable from a specified history node
The system SHALL support building retrieved context and runtime views from an explicit history node rather than always from the latest session state.

#### Scenario: Historical node is checked out
- **WHEN** the UI or host requests retrieved context for a specified node
- **THEN** the returned context SHALL reflect that node's transcript and run boundary
- **AND** it SHALL NOT silently fall back to latest-only session state

### Requirement: Frontend surfaces must make branch and history state explicit
The Tauri frontend SHALL present enough state for users to understand whether they are at latest, in historical mode, or on a forked branch.

#### Scenario: User is viewing a historical node
- **WHEN** the current visible node is not the active branch head
- **THEN** the UI SHALL indicate that the user is in a historical position
- **AND** the UI SHALL offer actions to restore latest or continue from that point

#### Scenario: A fork has been created
- **WHEN** a new branch is created from a historical node
- **THEN** the UI SHALL distinguish the new branch from the previous branch
- **AND** the UI SHALL allow switching back to the previous branch head
