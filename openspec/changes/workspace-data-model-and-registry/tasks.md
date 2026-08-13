# Tasks

- [x] 新建 `crates/pony-agent-core/src/agent/workspace.rs`：`WorkspaceRecord` + 注册逻辑（`DEFAULT_WORKSPACE_ID="default"`、root canonical 校验 + Windows 前缀归一、重复 root/非目录拒绝、id 生成、resolve）。
- [x] `PersistedStore` 增加 `workspaces` 字段（serde default）；SQLite `store_metadata` key=`workspaces` 读写；File/Memory backend 随 full-store 自动持久化。
- [x] `SessionStore`：`workspaces` 加载 + 确保默认 workspace 始终存在（root=current_dir）；`list_workspaces` / `create_workspace` / `resolve_workspace_root` / `stamp_workspace_id`（首次盖章幂等）。
- [x] `SessionState.workspace_id: Option<String>`（serde default）；`SessionSnapshot`/`SessionOverview` 投影透传（None → 默认）；`default_snapshot_for_session` / checkpoint materialize 继承。
- [x] 前后端 `TurnInput.workspace_id`/`workspaceId`；`apply_governed_turn_context` 首次持久化盖章（run_turn + stream 双路径覆盖）。
- [x] 宿主 Tauri command：`workspace_list` / `workspace_create`（最小面，control plane `workspace_commands.rs`）；**不实现 `workspace_activate`**。
- [x] 承接 PA-078：`import_attachment` 的 `workspace_id` 改为按 `WorkspaceRegistry` 解析目标 root（缺省进程 root）；未注册 id → 明确错误。
- [x] 前端：`SessionOverview.workspaceId`/`TurnInput.workspaceId` 类型 + `submitTurn` payload 携带 workspaceId（本轮 null→默认）+ 3 个本地 SessionOverview 构造器补字段（workspaceId: null）。
- [x] Rust 单测：注册表 CRUD/重复 root/非目录/缺失 root/默认解析/前缀归一（workspace.rs 5 例）+ SessionStore 默认存在/CRUD/盖章幂等/旧会话 serde roundtrip + SQLite workspaces roundtrip（+4 例）。
- [x] 回归：core lib 749 + tool_router_regression 13 + session_regression 5 + provider_registry_regression 8 + 前端 vitest 377 + `npm run build` + `cargo:check:shared` 全绿。

## Validation Notes

- 3 路对抗审核（2026-08-09）已采纳：`TurnInput.workspaceId` 线上传输（T-2/C-6/R-16）、激活移除后端持久化（C-4/T-7）、默认 id 保留字 `"default"`（C-5）、serde roundtrip 断言迁移（T-5）、重复 root canonical 比较 + 非目录拒绝（T-15）、持久化走 PersistedStore/store_metadata（C-10 目标一致：不重复写、不互覆盖）、命名去歧义（R-19）、前端 3 个构造器（C-17）、`import_attachment` 注册表解析 owner（F-2/T-2）。
- 实现完成（2026-08-09）：全部任务完成并通过全量验证。下一步：实现后 3 路对抗审核 → 验证收口。
