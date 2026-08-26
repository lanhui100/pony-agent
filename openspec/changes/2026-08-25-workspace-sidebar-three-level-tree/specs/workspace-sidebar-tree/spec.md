# workspace-sidebar-tree-navigation Delta

## REMOVED Requirements

### Requirement: Collapsible workspace groups

**原因**：需求定稿工作区为不可折叠的一级菜单；单一树 IA 下组折叠交互与其 localStorage 持久化整体废弃（残留 key 由剪枝逻辑消化、停止写入）。组内 >5 条仍保留分区预览上限与全局「显示全部」解除，不属于折叠语义。

## MODIFIED Requirements

### Requirement: Workspace-grouped session list

The session sidebar SHALL render a single three-level tree: one fixed non-collapsible "工作区" section header (first level), one row per registered workspace (second level), and that workspace's conversations beneath it (third level). Sessions without an owning registered workspace SHALL render as header-less rows in a flat zone directly beneath the section header.

#### Scenario: Grouping by workspace

- GIVEN sessions belonging to registered workspaces
- WHEN the sidebar renders the tree
- THEN each workspace SHALL render exactly one row showing its folder icon, name and session count
- AND its conversations SHALL render as third-level rows in `updatedAtMs` descending order with `conversationId` ascending as tie-break

#### Scenario: Legacy sessions

- GIVEN a session without a `workspaceId`
- WHEN the sidebar renders
- THEN the session SHALL appear in the header-less flat zone at the top of the tree

#### Scenario: Orphan workspace id

- GIVEN a session whose `workspaceId` has no entry in the workspace registry
- WHEN the sidebar renders
- THEN the session SHALL appear in the same header-less flat zone (never a separate "未分组" group)
- AND the session SHALL remain switchable and deletable

#### Scenario: Flat zone ordering

- GIVEN the flat zone contains both default and orphan sessions
- WHEN the tree derives
- THEN they SHALL merge into one zone sorted by `updatedAtMs` descending then `conversationId` ascending
- AND the transient unsaved entry SHALL be pinned above the zone regardless of timestamps

#### Scenario: Section header is fixed

- WHEN the sidebar renders the "工作区" section header
- THEN it SHALL NOT offer any collapse affordance and the tree SHALL always render expanded
- AND persisted per-group collapse keys SHALL neither affect rendering nor be written again

#### Scenario: Unsaved current session grouping

- WHEN the current transient session targets a workspace
- THEN it SHALL appear under that workspace's row (or pinned at the flat-zone top when the target is the default workspace)

#### Scenario: Show all groups

- WHEN the user triggers "显示全部"
- THEN the per-partition conversation cap SHALL be lifted across the flat zone and every workspace group simultaneously

### Requirement: Workspace management entry

The sidebar section header SHALL expose an add-workspace icon button, and each workspace row SHALL expose rename and delete actions via its three-dot menu. Creating a workspace registers an existing directory; deletion removes only the registry entry. The top-level new-conversation button SHALL always target the default workspace (the flat zone); the per-row button creates explicitly within its own workspace without changing any other creation target state.

#### Scenario: Add-workspace button

- WHEN the user hovers the section header's trailing icon button
- THEN a tooltip SHALL read 「添加工作区」
- AND clicking it SHALL open a directory picker restricted to existing directories
- AND the create form SHALL prefill the name with the picked directory's basename (editable)

#### Scenario: Rename workspace

- WHEN the user picks 重命名 from a workspace row's menu
- THEN an editable field SHALL validate trim-non-empty, ≤64 characters, and uniqueness against existing workspace names
- AND confirming SHALL persist the new display title while the workspace id and root path stay unchanged

#### Scenario: Delete workspace requires confirmation

- WHEN the user picks 删除工作区 from the menu
- THEN an asynchronous confirmation popover SHALL state all consequences: the registration is removed; disk folders and conversation histories remain; its conversations move to the flat zone and their subsequent file operations run against the default workspace root
- AND the destructive action SHALL run only after explicit confirmation

#### Scenario: Delete rewrites ownership

- GIVEN the deleted workspace owned N conversations
- WHEN deletion commits
- THEN every owned conversation's `workspaceId` SHALL be rewritten to the default workspace id before persistence completes
- AND attachment imports and tool executions in those conversations SHALL resolve to the default workspace root without error

#### Scenario: Deleting the active workspace

- WHEN the deleted workspace is the active one
- THEN the activation SHALL fall back to the default workspace and the stale local-storage key SHALL be cleared

#### Scenario: Default workspace is not deletable

- WHEN a delete request names the default workspace
- THEN the backend SHALL reject it regardless of any frontend affordance

#### Scenario: Browser mode degradation

- WHEN running in browser mode without a host
- THEN the add-workspace button, per-row buttons and menus SHALL be hidden
- AND conversation rows SHALL keep only locally available operations (delete)
- AND the collapsed rail state SHALL follow the same matrix

## ADDED Requirements

### Requirement: Conversation row operations

Each conversation row SHALL expose a three-dot menu containing 重命名, 归档 and 删除对话. Renaming overrides the auto-derived title durably; archiving hides the row everywhere with an irreversible-until-further-notice confirmation.

#### Scenario: Menu anatomy and guards

- WHEN a persisted conversation renders
- THEN its menu SHALL contain exactly 重命名 / 归档 / 删除对话
- AND running or submitting conversations SHALL disable the entries with a reason tooltip
- AND the transient blank entry SHALL expose no menu

#### Scenario: Rename persists across turns

- GIVEN a renamed conversation
- WHEN further turns are submitted (both the history-derived and trace-derived projection branches run)
- THEN the stored title SHALL equal the override
- AND history checkout or branch switching SHALL NOT resurrect the derived title

#### Scenario: Archive confirmation copy

- WHEN the user picks 归档
- THEN the confirmation SHALL state that the conversation disappears from the sidebar and cannot be restored from the UI in the current version (disk history remains)

#### Scenario: Archive hides everywhere and survives restart

- WHEN an archive commits
- THEN the row SHALL leave every grouping surface immediately
- AND after an app restart the conversation SHALL still be hidden
- AND its session log and data SHALL remain untouched on disk

#### Scenario: Archiving the active conversation

- GIVEN the archived conversation is the currently open one and is idle
- WHEN the archive commits
- THEN the main view SHALL fall back to another available conversation automatically

#### Scenario: Asynchronous confirmation discipline

- WHEN a confirmation popover request is in flight
- THEN confirm/cancel controls SHALL be disabled with a pending indicator
- AND Escape or outside clicks SHALL count as cancel without interrupting the request
- AND repeated confirms SHALL be absorbed as no-ops
