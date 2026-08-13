# workspace-data-model-and-registry

## Background

当前 `workspace_root` 是单一 `current_dir()`（`agent/tools.rs:936` 默认值），会话全局平铺：`SessionOverview`（`src/types/runtime.ts:799`）与 `SessionState`（`agent/session.rs:291`）都没有 workspace 归属字段。Workspace 多项目能力的第一步是建立数据模型：workspace 注册表（id/name/root）、会话归属字段、现有会话迁移策略。SQLite `sessions` 表以 `session_data TEXT` 存整个 `SessionState`（serde `#[serde(default)]` 兼容新增字段），`store_metadata` 表可承载注册表（`sqlite_session.rs:105`，已有 `attachment_assets`/`mcp_source_snapshots` 等同模式键）。

3 路对抗审核（2026-08-09）确认的关键约束：

- **无 `create_session` host command**：前端 `createSession()` 是纯本地状态（`runtime.ts:1667`），会话由 `run_turn`/`start_graph_run_stream` 携带 `TurnInput` 懒创建；`TurnInput`（前后端）都没有 `workspaceId`。因此"会话归属 workspace"需要明确的线上传输：给 `TurnInput` 增加 `workspaceId`，首次持久化时写入 `SessionState.workspace_id`。
- **激活 workspace 双持久化冲突**：原设计 PA-079 有 `workspace_activate`（后端记录激活状态），PA-081 又有 localStorage。本卡采纳意见：激活是前端呈现/建会话的 UI 状态，单一真相源放在前端 localStorage（PA-081 拥有）；**PA-079 移除 `workspace_activate`**（不作为激活真相源）。
- **默认 workspace id 必须固定**：默认 id 为保留字 `"default"`（豁免 `ws-<slug>` 规则），`None → "default"` 投影与此一致，PA-081 消费同一常量，避免 `workspaceById("default")` 找不到记录。
- **命名去歧义**：既有 `WorkspaceRef`/`WorkspaceRefKind`/`HistoryNode.workspace_ref`（`session.rs:88-111`）是 git/host-snapshot 回滚引用，与本卡的 `workspace_id`（项目目录概念）含义不同，需在文档中显式区分。

## Goals

- 建立 `WorkspaceRegistry`：多 workspace（id/name/root_path）注册、列出、持久化（SQLite `store_metadata`，JSON backend 兼容），默认 workspace 始终存在。
- 会话归属：`SessionState` / `SessionOverview` / `SessionSnapshot` 增加 `workspace_id` 字段（serde default 向后兼容）；**传输通道 = `TurnInput.workspaceId`**，首次持久化盖章。
- 现有会话加载后归属默认 workspace，不丢数据（以 serde roundtrip 断言验证，而非模糊的"不丢数据"）。
- 前端 TS 类型与 store 透传 `workspaceId`，UI 行为不回归；前端 3 个本地 `SessionOverview` 构造器同步补 `workspaceId`。

## Non-goals

- 不做路径级权限判定（PA-080 承接）。
- 不做侧边栏树 UI（PA-081 承接）。
- 不做激活 workspace 的后端持久化（激活为前端 localStorage 单一真相源，PA-081 拥有；后端如需可后续加，不阻塞本卡）。
- 不做多 workspace 缓存隔离策略（记录为后续候选；PA-025/029 的 stable prefix 语义本轮保持）。
- 不引入 workspace 间会话迁移/复制。
- 不做多 workspace 写并发模型。

## Scope

- `crates/pony-agent-core`：`WorkspaceRecord` / `WorkspaceRegistry`（新模块 `agent/workspace.rs`）、`SessionState.workspace_id`、`TurnInput.workspace_id`、SessionBackend 元数据读写扩展（`read_metadata`/`write_metadata`）、默认 workspace 初始化、`create_session` 时从 TurnInput 盖章。
- 宿主：workspace 注册表 Tauri command（`workspace_list` / `workspace_create`）+ 会话创建绑定（TurnInput 透传）+ **承接 PA-078 `import_attachment` 的 `workspace_id` → 注册表 root 解析**（缺省进程 root；实现后 `import_attachment` 的非默认 workspaceId 由拒绝改为按注册表解析）。
- 前端：`SessionOverview.workspaceId`、`TurnInput.workspaceId`、runtime store 透传、3 个本地构造器补字段。
- 测试：注册表持久化 roundtrip、默认归属、旧数据 serde roundtrip 兼容、重复 root 拒绝（canonical 比较）、回归集。

## Risks

- 既有会话无 workspace_id：默认归属默认 workspace，迁移逻辑必须幂等且不触发全量重写（PA-069-D 的 upsert 语义）；"不丢数据"以 serde roundtrip 断言（旧 blob 加载后所有既有字段原样往返、`workspace_id` 解析为默认）作为可测定义。
- 双后端（SQLite/JSON）一致性：注册表在 SQLite 时存 `store_metadata`（key=`workspaces.v1`）；JSON backend 存独立文件；读取端统一投影。损坏时 fail-safe 回退到"仅默认 workspace"（root = `current_dir()`），不阻塞启动。
- `workspace_id` 命名/语义漂移：id 采用稳定字符串（默认 `"default"` 保留字，其余 `ws-<slug>`），root_path 规范化绝对路径（Windows 去 `\\?\` 前缀，统一 canonical 形式）；与既有 `WorkspaceRef` 回滚概念显式区分。
- 持久化生命周期：注册表写入走 `SessionBackend::read_metadata/write_metadata` 扩展（复用 `store_metadata` 的 `INSERT OR REPLACE`），每次变更即写，与既有 full-store save 不互相覆盖；新增锁（registry `RwLock`）须登记进 `docs/concurrency/lock-ordering.md` 规范顺序。

## Validation

- Rust 单测：注册表 CRUD + 持久化 roundtrip + 重复 root 拒绝（canonical 比较、Windows 大小写归一）+ 相对路径规范化 + 非目录 root 拒绝 + 旧会话 serde roundtrip 兼容 + 损坏回退 + 回归集。
- 前端 vitest、`npm run build`、`npm run cargo:check:shared`。
- 回归集：`cargo test --test tool_router_regression`（宿主 crate，13 项）。
