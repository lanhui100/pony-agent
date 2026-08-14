# workspace-path-permission Specification

## Purpose

路径级权限边界：core 提供统一路径权限模块（组件级前缀比较 + Windows 大小写折叠 + 可注入 canonicalizer），供所有触碰文件的工具共享，取代各工具内联解析；写操作默认仅允许 workspace 根（递归）+ 受控 tmp（`outside_workspace_write_denied`）；workspace 外读取需显式授权（授权清单持久化，`requires_authorization` 审批语义）；符号链接/穿越 fail-closed；会话级 root 按 workspaceId 在调用时解析（非构造期固定），显式执行上下文贯穿工具链避免并发串扰。

## Requirements

### Requirement: Unified path permission module

The core SHALL provide a single path-permission module shared by all file-touching tools, replacing per-tool inline resolution logic. The module SHALL accept an injectable canonicalizer so security tests can be run hermetically.

#### Scenario: Shared resolution

- WHEN any file tool resolves a path against the workspace
- THEN it SHALL use the unified module
- AND the module SHALL canonicalize the path and verify component-level containment

#### Scenario: Prefix confusion prevention

- WHEN a path shares a textual prefix with the workspace root but is not inside it (for example `/ws-1` vs `/ws-10`)
- THEN the path SHALL be rejected

#### Scenario: Per-session root resolution

- WHEN a session's tool resolves a path
- THEN the workspace root SHALL be resolved from the session's `workspaceId` at call time (not captured at construction)
- AND the tool execution context SHALL carry the `workspaceId` (or a resolved root)
- AND when the context is absent (legacy sessions, direct executor calls) the constructor default root SHALL be used

#### Scenario: Orphan workspace read fallback

- WHEN a session's `workspaceId` has no registry entry and a read is attempted
- THEN the read SHALL classify against the default workspace root (with a warning), keeping the session usable
- AND writes for that session SHALL remain fail-closed

### Requirement: Write permission boundary

Write operations SHALL be allowed only inside the workspace root (recursively) or in a controlled tmp directory; anywhere else they SHALL be denied.

#### Scenario: Write inside workspace

- WHEN a write targets a path inside the workspace root
- THEN it SHALL be allowed

#### Scenario: Write in controlled tmp

- WHEN a write targets a path inside the controlled tmp directory
- THEN it SHALL be allowed

#### Scenario: Write outside workspace

- WHEN a write targets a path outside both the workspace root and the controlled tmp
- THEN it SHALL be denied with an explicit structured error (code `outside_workspace_write_denied`)
- AND the file SHALL NOT be created or modified

#### Scenario: Writing a new file

- WHEN a write targets a not-yet-existing file inside the workspace whose parent exists
- THEN the permission check SHALL succeed (parent canonicalization + component-level prefix check)

#### Scenario: Writing into a brand-new nested directory

- WHEN a write targets a file whose parent directory does not exist
- THEN the check SHALL canonicalize the nearest existing ancestor and verify the remaining suffix contains no `..` or separator components
- AND the write SHALL be allowed (the tool may create the parents)

#### Scenario: Filename traversal suffix

- WHEN a write targets a path whose non-existing suffix contains a `..` component
- THEN the write SHALL be denied before any IO

#### Scenario: Run command cwd

- WHEN a Run command is executed
- THEN its `cwd` SHALL be inside the workspace root or the controlled tmp
- AND writes performed inside the command process SHALL NOT be attributed to this path check (delegated to the sandbox boundary)

### Requirement: Read permission boundary

Read operations SHALL be allowed inside the workspace; outside reads SHALL require an authorization entry or return a structured permission error.

#### Scenario: Read inside workspace

- WHEN a read targets a path inside the workspace root
- THEN it SHALL be allowed

#### Scenario: Authorized external read

- WHEN a read targets a path outside the workspace
- AND an authorization entry exists for that path or an ancestor
- THEN it SHALL be allowed

#### Scenario: Unauthorized external read

- WHEN a read targets a path outside the workspace with no authorization
- THEN it SHALL be denied with a structured permission error (code `requires_authorization`, approval semantics surface)

#### Scenario: File-scoped authorization

- WHEN an authorization exists for a file F
- THEN reads of F SHALL be allowed
- AND reads of F's sibling SHALL be denied

#### Scenario: Directory-scoped authorization

- WHEN an authorization exists for a directory D
- THEN reads of paths under D SHALL be allowed

### Requirement: Authorization registry

The core SHALL persist explicit user-granted read authorizations and support grant, revoke and list.

#### Scenario: Granting authorization

- WHEN a path is authorized explicitly
- THEN it SHALL be persisted
- AND reads under that path SHALL be allowed after restart

#### Scenario: Revoking authorization

- WHEN an authorization is revoked
- THEN subsequent reads under that path SHALL be denied again

#### Scenario: Revocation is exact-path

- WHEN an authorization is revoked and a child path was also independently authorized
- THEN the child authorization SHALL remain effective

#### Scenario: Rejecting write scope this round

- WHEN `authorize_path` is called with scope `"read-write"`
- THEN it SHALL return an explicit "not supported" error
- AND no authorization entry SHALL be created

### Requirement: Symlink and traversal defense

The permission module SHALL fail closed on traversal and symlink escapes.

#### Scenario: Traversal attempt

- WHEN a path contains `..` components that escape the workspace
- THEN `classify_path` SHALL return a `permission_denied` error and the tool SHALL NOT open or stat the target

#### Scenario: Symlink escape

- WHEN a symlink inside the workspace resolves outside the workspace
- THEN the read/write SHALL be denied
- UNLESS the resolved target has an explicit authorization entry

#### Scenario: Hermetic symlink test

- WHEN the canonicalizer is injected to report an outside path for an in-workspace link
- THEN the same deny decision SHALL be produced without creating a real symlink

### Requirement: Windows path comparison

Path comparisons on Windows SHALL be case-normalized component-wise so that `C:\WS` and `c:\ws` match while `/ws-1` and `/ws-10` never match.

#### Scenario: Case-variant match

- WHEN comparing `c:\ws\file` against root `C:\WS` on Windows
- THEN the comparison SHALL be equal (allowed)

#### Scenario: Case-variant prefix confusion

- WHEN comparing `c:\ws-10\file` against root `C:\WS` on Windows
- THEN the comparison SHALL NOT be equal (denied)