# workspace-data-model-and-registry Delta

## ADDED Requirements

### Requirement: Workspace registry

The core SHALL maintain a persistent registry of workspaces, each identified by a stable id with a name and an absolute root path.

#### Scenario: Creating a workspace

- WHEN a workspace is created with a name and an absolute root path
- THEN it SHALL be assigned a stable id
- AND it SHALL be persisted across restarts
- AND a duplicate root path SHALL be rejected (compared on canonical form, case-normalized on Windows)

#### Scenario: Rejecting a non-directory root

- WHEN a root path exists but is not a directory
- THEN creation SHALL fail with an explicit error

#### Scenario: Listing workspaces

- WHEN the registry is queried
- THEN all persisted workspaces SHALL be returned with id, name and root path
- AND the default workspace SHALL always exist

#### Scenario: Persistence roundtrip

- WHEN the app restarts after workspace creation
- THEN the registry SHALL restore all previously created workspaces

#### Scenario: Default workspace id

- GIVEN the registry
- THEN the default workspace id SHALL be the reserved literal `"default"`
- AND the default workspace SHALL be present on first boot (root = current workspace root)

### Requirement: Session workspace ownership

Every session SHALL belong to exactly one workspace. The ownership value SHALL travel from the frontend to the backend through `TurnInput.workspaceId` and be stamped into `SessionState.workspace_id` on first persist.

#### Scenario: Session creation binds a workspace

- WHEN a turn is submitted with `workspaceId` in its `TurnInput`
- THEN the persisted `SessionState.workspace_id` SHALL equal that value

#### Scenario: Session creation defaults to default workspace

- WHEN a turn is submitted without an explicit `workspaceId`
- THEN the session SHALL be bound to the default workspace

#### Scenario: Existing sessions keep working

- GIVEN sessions persisted before this change
- WHEN they are loaded
- THEN each pre-existing field SHALL round-trip unchanged (title, summary, history length and content, turnCount, updatedAtMs, attachmentAssets)
- AND `workspace_id` SHALL resolve to the default workspace

#### Scenario: Overview projection

- WHEN session overviews are listed
- THEN each overview SHALL carry its `workspaceId`
- AND the frontend SHALL receive it additively (no command rename; existing fields unchanged)

### Requirement: Workspace root path normalization

Workspace root paths SHALL be stored as canonical absolute paths in a single canonical form (Windows `\\?\` prefix normalized away).

#### Scenario: Relative root input

- WHEN a relative root path is provided
- THEN it SHALL be canonicalized to an absolute path before storage

#### Scenario: Invalid root input

- WHEN the root path does not exist
- THEN creation SHALL fail with an explicit error
