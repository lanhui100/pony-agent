# workspace-data-model Delta

## ADDED Requirements

### Requirement: Workspace rename and deletion

The workspace registry SHALL support renaming a registered workspace and deleting its registration. Deletion SHALL be registry-scoped: disk directories, session logs and session data remain untouched, while ownership of every accounted session is rewritten to the default workspace in the same persisted operation.

#### Scenario: Renaming validates input

- WHEN a rename requests a blank name, a name longer than 64 characters, or a duplicate of an existing workspace name
- THEN the operation SHALL fail with a descriptive error
- AND the workspace id and root path SHALL remain unchanged

#### Scenario: Rename is display-only

- GIVEN sessions stamped with the renamed workspace's id
- WHEN the rename commits
- THEN those sessions SHALL keep their membership (id-based accounting is unaffected)

#### Scenario: Deletion rejects protected ids

- WHEN a delete request targets the default workspace id or an id absent from the registry
- THEN the operation SHALL fail with a descriptive error

#### Scenario: Deletion rewrites member ownership

- GIVEN a workspace whose account lists N sessions
- WHEN the deletion commits
- THEN all N sessions SHALL carry `workspaceId = default` in persisted state
- AND `resolve_workspace_root` for each of those sessions SHALL return the default root afterwards
- AND attachment imports SHALL succeed without override paths

#### Scenario: Persistence roundtrip

- WHEN rename or delete commits
- THEN the registry change SHALL survive an app restart via the existing store_metadata channel

### Requirement: Session display-title override

A session SHALL support a durable display-title override that takes precedence over the first-user-message-derived title at every projection point. The precedence applies to top-level title fields (session list rows, snapshot headers); historical node listings MAY retain the titles captured at their commit time.

#### Scenario: No override keeps derived title

- **GIVEN** a fresh session without an override
- **WHEN** its first user message persists
- **THEN** the effective title derives from that first user message (first non-blank line, whitespace-collapsed, truncated at 28 characters with an ellipsis)
- **AND** an empty history falls back to the default session title

#### Scenario: Override survives turn persistence

- GIVEN a session with a stored override
- WHEN turn persistence recomputes projection metadata (both the history-derived branch and the empty-history trace branch)
- THEN the effective title SHALL remain the override

#### Scenario: Override survives history checkout

- GIVEN a renamed session
- WHEN the user checks out a history node committed before the rename
- THEN the snapshot's top-level title field SHALL equal the override
- AND the sidebar SHALL show the override title
- AND historical node list entries SHALL keep their commit-time titles

#### Scenario: Session rename validates input

- WHEN a session rename requests a blank/whitespace-only title or exceeds 64 characters
- THEN the operation SHALL fail with a descriptive error and the stored override SHALL be unchanged
- AND renaming to the identical current title SHALL succeed as a no-op

#### Scenario: Operations on unknown sessions fail closed

- WHEN a rename or archive command names a session absent from the store
- THEN the operation SHALL fail without creating any session record

#### Scenario: Legacy blobs parse unchanged

- GIVEN persisted session records written before the field existed
- WHEN they load
- THEN deserialization SHALL default the override to none
- AND new writes SHALL omit the field when unset

### Requirement: Session archive flag

A session SHALL carry a durable archived flag that grouping surfaces honor by hiding the session everywhere while its log and data remain on disk. Clearing the flag later suffices to surface the session again under its then-current ownership (ownership rewrites performed by workspace deletion are not reversed; restore UI itself is out of scope for this change).

#### Scenario: Archive projects into overview

- WHEN an archive persists for a session
- THEN the session overview projection SHALL report `archived = true`

#### Scenario: Archive is idempotent

- GIVEN an already-archived session
- WHEN archive is requested again
- THEN the operation SHALL succeed as a no-op

#### Scenario: Hidden across restarts

- GIVEN an archived session
- WHEN the app restarts and reloads the session catalog
- THEN the session SHALL still be reported archived
