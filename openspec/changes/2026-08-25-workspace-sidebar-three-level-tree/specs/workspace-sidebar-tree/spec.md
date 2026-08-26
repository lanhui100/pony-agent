# workspace-sidebar-tree-navigation Delta

## REMOVED Requirements

### Requirement: Collapsible workspace groups

**Reason**: 需求定稿工作区为不可折叠的一级菜单；组折叠交互与其 localStorage 持久化整体废弃（残留 key 由既有剪枝逻辑消化、停止写入）。随本条一并退役的还有旧「Current session grouping」的自动展开语义与激活驱动的建会话交互（残余定位见 Workspace management entry 的 MODIFIED 文本）。分区内 >5 条的预览上限与全局「显示全部」解除不属折叠语义，予以保留。

## MODIFIED Requirements

### Requirement: Workspace-grouped session list

The session sidebar SHALL render a single three-level tree: one fixed non-collapsible "工作区" section header (first level), one row per registered non-default workspace (second level), and each such workspace's conversations beneath it (third level). Sessions whose owning workspace is the default, sessions without a `workspaceId`, and sessions whose `workspaceId` has no registry entry SHALL render as header-less rows in a flat zone directly beneath the section header. The default workspace SHALL NOT render a second-level group row.

#### Scenario: Default workspace renders no group row

- **GIVEN** the default workspace is present in the registry together with conversations stamped to it
- **WHEN** the tree renders
- **THEN** no 「默认工作区」 second-level row exists
- **AND** those conversations appear in the header-less flat zone

#### Scenario: Grouping by workspace

- **GIVEN** sessions belonging to registered non-default workspaces
- **WHEN** the sidebar renders the tree
- **THEN** each such workspace renders exactly one row showing its folder icon, display name and visible-session count
- **AND** its conversations render as third-level rows in `updatedAtMs` descending order with `conversationId` ascending as tie-break

#### Scenario: Flat zone ordering

- **GIVEN** the flat zone contains default-owned sessions, sessions without `workspaceId`, and orphan sessions
- **WHEN** the tree derives
- **THEN** they merge into one header-less zone sorted by `updatedAtMs` descending then `conversationId` ascending
- **AND** the transient unsaved entry is pinned above the zone regardless of timestamps

#### Scenario: Counts exclude archived

- **GIVEN** a workspace owning 3 persisted and 1 archived conversation
- **WHEN** the tree renders
- **THEN** its count badge shows 3
- **AND** archived conversations contribute to neither counts nor rendered rows nor ordering inputs anywhere in the tree

#### Scenario: Section header is fixed

- **WHEN** the sidebar renders the "工作区" section header
- **THEN** it offers no collapse affordance and the tree always renders expanded
- **AND** persisted per-group collapse keys neither affect rendering nor get written again

#### Scenario: Unsaved current session grouping

- **WHEN** the current transient session's explicit creation target is a registered non-default workspace
- **THEN** it appears under that workspace's row
- **AND** when the target is the default workspace it is pinned at the flat-zone top

#### Scenario: Legacy orphan blobs

- **GIVEN** a pre-change database containing sessions whose `workspaceId` has no registry entry
- **WHEN** the app boots
- **THEN** those sessions render in the flat zone
- **AND** reads fall back to the default root per the path-permission contract

#### Scenario: Show all groups

- **WHEN** the user triggers "显示全部"
- **THEN** the per-partition conversation cap lifts across the flat zone and every workspace group simultaneously

### Requirement: Workspace management entry

The sidebar section header SHALL expose an add-workspace icon button using a semantic folder-plus glyph with hover tooltip 「添加工作区」, and each non-default workspace row SHALL expose rename and delete actions via its three-dot menu. Creating a workspace registers an existing directory; deletion removes only the registry entry and rewrites member ownership to the default workspace. User-visible activation affordances (激活 badge, 当前工作区 indicator, activation-driven creation) are retired by this change; the persisted activation state remains solely as internal fallback context normalized on registry changes.

#### Scenario: Add-workspace button

- **WHEN** the user hovers the section header's trailing icon button
- **THEN** a tooltip reading 「添加工作区」 appears beside a semantic folder-plus glyph
- **AND** clicking it opens a directory picker restricted to existing directories
- **AND** the create form prefills the name with the picked directory's basename (editable)

#### Scenario: Create validates like rename

- **GIVEN** the create form submitted with a blank name, a name over 64 characters, or a name duplicating an existing workspace
- **WHEN** the user confirms
- **THEN** the backend rejects the request with a descriptive error and nothing is registered

#### Scenario: Rename workspace

- **WHEN** the user picks 重命名 from a workspace row's menu
- **THEN** an editable field validates trim-non-empty, ≤64 characters, and uniqueness against every OTHER workspace's current name
- **AND** confirming persists the new display title while the workspace id and root path stay unchanged

#### Scenario: Delete workspace requires confirmation

- **WHEN** the user picks 删除工作区 from the menu
- **THEN** an asynchronous confirmation popover states all consequences: the registration is removed; disk folders and conversation histories remain; its conversations move to the flat zone and their subsequent file operations run against the default workspace root (a turn already in flight completes against its start-time root)
- **AND** the destructive action runs only after explicit confirmation
- **AND** the shared asynchronous confirmation discipline applies (in-flight controls disabled, Escape/outside click cancels without interrupting the request, repeated confirms are absorbed, failures surface in the popover for retry)

#### Scenario: Delete rewrites ownership

- **GIVEN** the deleted workspace owned N conversations
- **WHEN** deletion commits
- **THEN** every owned conversation's `workspaceId` is rewritten to the default workspace id before persistence completes
- **AND** attachment imports and tool executions in those conversations resolve to the default workspace root without error

#### Scenario: Deleting the active context falls back safely

- **WHEN** the deleted workspace was referenced by persisted activation state or a live creation target
- **THEN** both fall back to the default workspace before the next submission
- **AND** stale local-storage activation keys are cleared

#### Scenario: Default workspace is not deletable

- **WHEN** a delete request names the default workspace
- **THEN** the backend rejects it regardless of any frontend affordance

#### Scenario: Browser mode degradation

- **WHEN** running in browser mode without a host
- **THEN** the whole workspace management surface is hidden (add-workspace button, per-row new-conversation buttons, workspace rename/delete menus)
- **AND** conversation three-dot menus remain available but only the locally-supported 删除 entry is usable (重命名/归档 entries hidden, since neither has a browser-mode persistence path)
- **AND** the collapsed rail renders none of these management surfaces anyway, so it needs no further degradation rules

## ADDED Requirements

### Requirement: Explicit creation targeting

Every new-conversation entry point SHALL carry an explicit target workspace resolved at invocation time, replacing all activation-driven targeting. The top-level button targets the default workspace; a row-scoped button targets exactly its own workspace without changing any other entry point's target.

#### Scenario: Top-level button lands in the flat zone

- **GIVEN** any prior interactions with workspace rows
- **WHEN** the user clicks the top-level 新对话 button
- **THEN** the created session targets the default workspace and appears pinned in the flat zone

#### Scenario: Row-scoped create keeps the global target stable

- **GIVEN** the user clicks a workspace row's pure-icon new-conversation button
- **WHEN** the new session is created inside that workspace
- **AND** the user afterwards clicks the top-level 新对话 button
- **THEN** that subsequent top-level session still targets the default workspace (the flat zone)

#### Scenario: Creation target outlives deleted workspace

- **GIVEN** a live creation target (including an unsaved transient session) references workspace W
- **WHEN** W is deleted
- **THEN** the creation target renormalizes to the default workspace
- **AND** the next submission stamps the default id, never W's id

### Requirement: Conversation row operations

Each conversation row SHALL expose a three-dot menu containing 重命名, 归档 and 删除对话. Third-level titles truncate on a single line with ellipsis, coexisting with the backend-derived title's own 28-character truncation, and hovering reveals the full stored title. Renaming overrides the auto-derived title durably; archiving hides the row everywhere behind an irreversible-until-further-notice confirmation.

#### Scenario: Title truncation and hover

- **GIVEN** a conversation whose stored title exceeds the row width (with or without the backend's trailing ellipsis)
- **WHEN** the row renders
- **THEN** the title truncates to a single line with CSS ellipsis
- **AND** hovering the row reveals the full stored title verbatim

#### Scenario: Menu anatomy and guards

- **WHEN** a persisted conversation renders
- **THEN** its menu contains exactly 重命名 / 归档 / 删除对话
- **AND** running or submitting conversations disable the entries with reason tooltips (「对话运行中，暂不能执行该操作」 / 「正在提交，请稍候」)
- **AND** the transient blank entry exposes no menu

#### Scenario: New conversations derive their titles

- **GIVEN** a fresh session without an override whose first user message is sent
- **WHEN** the first turn persists
- **THEN** the displayed title derives from that first user message (first non-blank line, whitespace-collapsed, truncated at 28 characters with an ellipsis; empty history falls back to the default title)

#### Scenario: Rename persists across turns

- **GIVEN** a renamed conversation
- **WHEN** further turns are submitted (both the history-derived and trace-derived projection branches run)
- **THEN** the stored title equals the override
- **AND** history checkout or branch switching does not resurrect the derived title in the sidebar or the main view header

#### Scenario: Archive confirmation copy

- **WHEN** the user picks 归档
- **THEN** the confirmation states that the conversation disappears from the sidebar and cannot be restored from the UI in the current version (disk history remains)

#### Scenario: Archive hides everywhere and survives restart

- **WHEN** an archive commits
- **THEN** the row leaves every grouping surface immediately
- **AND** after an app restart the conversation is still hidden
- **AND** its session log and data remain untouched on disk

#### Scenario: Archiving the active conversation

- **GIVEN** the archived conversation is the currently open one and is idle
- **WHEN** the archive commits
- **THEN** the main view falls back to another available conversation automatically

#### Scenario: Asynchronous confirmation discipline

- **WHEN** any destructive confirmation popover in this change (删除工作区 / 归档 / 删除对话) has a request in flight
- **THEN** its confirm/cancel controls are disabled with a pending indicator
- **AND** Escape or outside clicks count as cancel without interrupting the request
- **AND** repeated confirms are absorbed as no-ops

#### Scenario: Failure after in-flight cancellation degrades gracefully

- **GIVEN** the user cancelled a destructive confirmation while its request was in flight (popover closed)
- **WHEN** that request subsequently fails
- **THEN** the failure SHALL surface exactly once through a non-popover surface (an inline message on the owning row)
- **AND** it SHALL not reopen the closed popover
