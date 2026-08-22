# workspace-sidebar-tree-navigation Delta

## ADDED Requirements

### Requirement: Workspace-grouped session list

The session sidebar SHALL group conversation entries under their owning workspace, rendered as a two-level tree (workspace group, then conversations).

#### Scenario: Grouping by workspace

- GIVEN sessions belonging to different workspaces
- WHEN the sidebar renders the session list
- THEN sessions SHALL be grouped under their workspace group
- AND each group SHALL show the workspace name and session count

#### Scenario: Legacy sessions

- GIVEN a session without a `workspaceId`
- WHEN the sidebar renders
- THEN the session SHALL appear under the default workspace group

#### Scenario: Unsaved current session grouping

- WHEN the current session is not yet persisted and the active workspace is not the default
- THEN the "new conversation" entry SHALL appear under the active workspace group

#### Scenario: Current session grouping

- WHEN the current session belongs to a workspace whose group is collapsed
- THEN the group SHALL be expanded automatically
- AND the current session SHALL be highlighted
- AND auto-expand SHALL apply only when no user toggle for that group has been recorded in this boot (an explicit user collapse SHALL win)

#### Scenario: Orphan workspace

- WHEN a session's `workspaceId` has no entry in the workspace registry
- THEN the session SHALL render under an "未分组" group
- AND the session SHALL remain switchable and deletable

#### Scenario: Show all groups

- WHEN the user triggers "显示全部"
- THEN the per-group conversation cap SHALL be lifted across all groups

### Requirement: Collapsible workspace groups

Each workspace group SHALL support expand/collapse, with state persisted locally.

#### Scenario: Toggle group

- WHEN the user clicks a workspace group header
- THEN the group SHALL expand or collapse
- AND the state SHALL be restored on next app start (remount-persistence, jsdom-tested)

#### Scenario: Active group default expand

- WHEN a workspace becomes active and has no persisted collapse value
- THEN its group SHALL default to expanded

#### Scenario: Stale collapse state ignored

- WHEN the persisted collapse map contains a key with no matching workspace
- THEN the stale entry SHALL be ignored without crashing

### Requirement: Workspace management entry

The sidebar SHALL expose an entry to create a new workspace and to activate a workspace. Activation SHALL be persisted in local storage as the single source of truth (the backend keeps no activation record).

#### Scenario: Create workspace

- WHEN the user creates a workspace with a name and root path
- THEN it SHALL appear as a new group
- AND it SHALL become active (new sessions SHALL be created under it after activation)

#### Scenario: Activate workspace

- WHEN the user activates a workspace
- THEN subsequent new sessions SHALL belong to it
- AND the activation SHALL persist across restarts (local storage)

#### Scenario: Switching does not hide groups

- WHEN the user switches the active workspace
- THEN other workspace groups SHALL remain visible (only the target of new-session creation changes)

#### Scenario: Browser mode degradation

- WHEN running in browser mode without a host
- THEN only the default group SHALL be shown
- AND the workspace management entry SHALL be disabled or hidden

### Requirement: Empty workspace state

A workspace with no conversations SHALL show an explicit empty state instead of a blank area.

#### Scenario: Empty group

- WHEN a workspace group has no sessions
- THEN the group SHALL show a hint text (for example "暂无对话")
