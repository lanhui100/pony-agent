# PA-080 Workspace 路径权限边界

## Basic Info

- ID: PA-080
- Status: Ready
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-08
- Updated At: 2026-08-08
- OpenSpec Change: `workspace-path-permission-boundary`（3 路对抗审核已通过，2026-08-09）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订）

## Background

现有权限模型只有 scope 级（`ToolPermissionScope`：WorkspaceRead/Write/Execute，`agent/tools.rs:199`），没有路径级判定。Workspace 限定文件夹后需要回答：workspace 之外能否读（需显式授权）、能否写（默认拒绝）。这是安全敏感改动，涉及符号链接/路径穿越防护与授权清单持久化。

## Goal

建立路径级权限边界：写权限默认仅授予 workspace 根（递归）+ 受控 tmp 目录；workspace 之外读取需显式授权（授权清单持久化，复用 `requires_approval` 审批语义）；所有判定基于 canonicalize 后前缀校验，防 `..` 穿越与符号链接逃逸。

## Scope

- 路径级权限判定模块（canonicalize + 前缀校验 + 授权清单）
- 授权清单持久化（SQLite/JSON，用户显式批准的路径）
- 与既有 `ToolPermissionScope`/`ApprovalRequired` 集成（scope 判定 + 路径判定组合）
- tmp 目录限定为受控子目录（workspace 内 `.tmp/` 或系统临时目录的受控子目录）
- 安全测试：路径穿越、符号链接、大小写、UNC/卷边界
- 后端单测 + 回归集

## Non-Goals

- 不做完整 SandboxBackend / Job Object containment（PA-077 承接）
- 不做前端授权 UI（记录为后续候选；先支持审批语义返回）
- 不改变既有工具名与结果合同
- 不做多 workspace 权限矩阵（PA-079 提供数据基础后按需扩展）

## Acceptance Criteria

1. 写操作（Write/Edit）路径与 Run 的 `cwd` 在 workspace 根或受控 tmp 内 → 放行；之外 → 拒绝（结构化错误 `outside_workspace_write_denied`）。进程内任意写入由 PA-077 沙箱/containment 边界承接。
2. 读操作路径在 workspace 内 → 放行；之外 → 授权清单命中放行，否则返回结构化错误 `requires_authorization`（用户 `authorize_path` 后重发；本轮不接 Ask）。
3. 授权清单持久化，重启后生效；撤销入口存在（API 级）。
4. 符号链接指向 workspace 外、`..` 穿越、前缀混淆（`/ws-1` vs `/ws-10`）全部拒绝。
5. 既有工具回归集（tool_router_regression 13 项）不回归。
6. Rust 单测 + 回归集 + 前端 vitest 全绿。

## Review Plan

- @consultant：权限模型架构、与既有 scope/approval 语义的组合边界、fail-closed 原则
- @code-reviewer：路径规范化、TOCTOU、符号链接/挂载点逃逸、授权清单存储安全
- @tester：对抗用例矩阵（穿越/软链/大小写/UNC/并发授权变更）

## Current Progress

- 3 路对抗审核（2026-08-09）已完成，findings 全部采纳；spec/proposal/design/tasks 已修订（Windows 组件级大小写折叠 + 双断言、写新文件复用 `prepare_workspace_file_path`、可注入 canonicalizer + hermetic symlink 测试、scope 仅 read、Run 只判 cwd、错误码断言等）。详见 `02_REVIEWS/2026-08-09-pa078-081-spec-review.md`。
- 无未解决 P0/P1，spec 通过审核，可进入实现。
- **实现完成（2026-08-13）**：`path_permission.rs`（885 行）统一路径判定 + 18 项对抗测试；工具接入（读/写/Run cwd 统一经 `classify_path`）；`AuthorizeStore` 持久化（SQLite `store_metadata` key=`path_authorizations.v1` + JSON fallback）；宿主 `authorize_path`/`revoke_authorization`/`list_authorizations`；`docs/concurrency/lock-ordering.md` 登记 `path_authorizations` 锁（与 sessions_rwlock 同级、先 registry 后 authorize）；runtime 构建时从 SessionStore 共享授权存储给 governed executor。
- **验证（2026-08-13）**：`npm run cargo:check:shared` 通过；core lib 772 全绿（含 path_permission 18 项对抗测试）；tool_router_regression 13 + session_regression 5 + provider_registry_regression 8 + src-tauri lib 6 全绿；前端 vitest 377 全绿。

## Next Action

- 提交实现（feat(pa-080)）；实现后 3 路对抗审核（@consultant 权限模型 / @code-reviewer 路径安全 / @tester 对抗矩阵复核）；PA-081（侧边栏树）在其后启动。

## Blockers

- 依赖 PA-079 的 workspace 数据模型（至少最小可用版）

## Resume Hint

- 先读 `crates/pony-agent-core/src/agent/sandbox.rs`、`tools.rs` 的 `ToolPermissionScope`/`ApprovalRequired`、`docs/concurrency/lock-ordering.md`
