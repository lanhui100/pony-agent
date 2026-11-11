# PA-079 Workspace 数据模型与注册表

## Basic Info

- ID: PA-079
- Status: Done
- Priority: P1
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-08
- Updated At: 2026-08-08
- OpenSpec Change: `workspace-data-model-and-registry`（3 路对抗审核已通过，2026-08-09；**已归档** `openspec/changes/archive/2026-08-09-workspace-data-model-and-registry/`，canonical spec `openspec/specs/workspace-data-model/spec.md`）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订；已完成收口）

## Background

当前 `workspace_root` 是单一 `current_dir()`（`agent/tools.rs:936`），会话全局平铺（`SessionOverview` 无 workspace 归属字段，`src/types/runtime.ts:799`）。Workspace 多项目能力的第一步是建立数据模型：workspace 注册表（id/name/root）+ 会话归属字段 + 持久化迁移。

## Goal

建立 Workspace 数据模型：多 workspace 注册表（持久化）、会话归属 workspace、现有会话默认归属迁移策略；为 PA-080（权限边界）与 PA-081（侧边栏树）提供数据基础，同时保持与缓存命中约束（ADR-0007）兼容。

## Scope

- 后端：`WorkspaceRegistry`（id/name/root_path，持久化到 SQLite/JSON）
- `SessionOverview`/`SessionSnapshot` 增加 `workspaceId` 字段
- 创建会话时绑定 workspace；现有会话迁移（默认 workspace 归属）
- 前端 TS 类型同步 + store 透传
- 后端单测 + `npm run cargo:check:shared`

## Non-Goals

- 不做路径级权限判定（PA-080 承接）
- 不做侧边栏树 UI（PA-081 承接）
- 不做多 workspace 切换的缓存隔离策略（记录为后续候选）
- 不引入多 workspace 写并发模型

## Acceptance Criteria

1. Workspace 注册表可创建/列出/持久化，root_path 校验为绝对路径。
2. 会话创建时可指定 workspaceId，缺省落到默认 workspace。
3. 现有会话加载后归属默认 workspace，不丢数据、不破坏回归集。
4. 前端类型与 store 透传 workspaceId，UI 无回归。
5. Rust 单测 + 回归集 + 前端 vitest 全绿。

## Review Plan

- @consultant：模型边界、与缓存命中约束的兼容性、迁移策略
- @code-reviewer：持久化实现、默认归属逻辑、回归风险
- @tester：迁移/归属/边界用例设计

## Current Progress

- 3 路对抗审核（2026-08-09）已完成，findings 全部采纳；spec/proposal/design/tasks 已修订。详见 `02_REVIEWS/2026-08-09-pa078-081-spec-review.md`。
- **实现完成（2026-08-09）**：`workspace.rs` 注册表、`PersistedStore.workspaces` 持久化、`SessionStore` workspace 方法（默认存在/CRUD/resolve/盖章）、`SessionState.workspace_id` + 投影、前后端 `TurnInput.workspaceId` 传输 + 首次盖章、宿主 `workspace_list`/`workspace_create`、`import_attachment` 注册表解析。
- **实现后审核（2026-08-09）**：@tester 有条件通过（无 P0），P1（首轮盖章丢失 bug + e2e、双后端损坏回退）+ P2 全部修复；coordinator 自审补重复 root 大小写归一。详见 `02_REVIEWS/2026-08-09-pa079-implementation-review.md`。（@consultant/@code-reviewer 实现后审核在后台运行，其 findings 作为增量修订并入。）
- 验证：core lib 754 + 回归 8/5/13 + 前端 vitest 377 + build + cargo:check:shared + OpenSpec 全绿。

## Next Action

- 已收口（2026-08-13）：任务状态 Done；OpenSpec change 已归档（`openspec/changes/archive/2026-08-09-workspace-data-model-and-registry/`）；canonical spec 已同步 `openspec/specs/workspace-data-model/spec.md`；实现已提交（`a118786`/`0f7bf94`/`57519c4` 等）。下一步推进 **PA-080（路径权限边界，P0，安全敏感）**。

## Blockers

- 无（前置 PA-078/PA-079 均已完成；PA-080 依赖本卡数据模型）

## Resume Hint

- 先读 `crates/pony-agent-core/src/agent/sqlite_session.rs`、`src/types/runtime.ts` 的 `SessionOverview`、`src/lib/runtime/sessions.ts`
