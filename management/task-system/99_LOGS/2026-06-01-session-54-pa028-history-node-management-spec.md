# 2026-06-01 Session 54 - PA-028 历史节点管理实现与验证收口

## 本次完成
- 将 `PA-028` 从 spec 立项推进到实现完成态。
- Rust core 新增并打通历史节点图、分支、历史游标与 checkout/fork/restore/switch branch 语义。
- runtime / retrieved context / session runtime view 支持 `nodeId` 历史重建。
- Tauri 前端补齐历史状态展示、分支恢复、分叉和切支操作。
- OpenSpec 与任务系统文档同步更新到完成态。

## 关键实现点
- `src-tauri/src/agent/session.rs`
  - 新增 `HistoryNode`、`HistoryBranch`、`HistoryCursor`、`WorkspaceRef` 及相关枚举。
  - 为 `SessionState` 与 `SessionSnapshot` 增加历史图与历史游标字段。
  - 落地 `checkout_history_node`、`restore_branch_head`、`fork_from_history_node`、`switch_history_branch`。
  - 修复分叉后主分支 head 被错误污染的问题。
  - 修复 `fork_from_history_node` 返回旧节点视角快照、导致 UI 误判分支的问题。
- `src-tauri/src/agent/runtime.rs`
  - `TurnInput` 增加 `node_id`。
  - `load_session_snapshot_at` 与 `inspect_retrieved_context_at` 支持按历史节点重建。
- `src-tauri/src/agent/control_plane.rs` / `src-tauri/src/lib.rs`
  - 对外暴露历史图加载、checkout、restore、fork、switch branch 命令与 runtime view。
- `src/stores/runtime.ts` / `src/types/runtime.ts`
  - 前端状态增加 `visibleNodeId`、`branchHeadNodeId`、`activeBranchId`、`historyCursorMode`、历史节点与分支集合。
  - turn 提交、runtime view、retrieved context 均支持 `nodeId`。
- `src/components/HomeSessionSidebar.vue`
  - 增加历史模式提示、恢复分支头、从当前节点分叉、切换分支与历史节点列表操作。

## 验证结果
- Rust 定向测试通过：
  - `history_commands_and_runtime_view_follow_persisted_history_graph`
  - `appending_from_historical_node_creates_a_fork_and_preserves_main_branch_head`
  - `restore_and_switch_history_branch_move_cursor_between_branch_heads`
- Frontend 单测通过：
  - `cmd /c npm run test:unit -- tests/HomeSessionSidebar.spec.ts tests/runtime-store.spec.ts`
  - 结果：`54 passed`

## 当前结论
- `PA-028` 已具备可交付实现，不再停留在 spec 阶段。
- 当前仍保留“工作区回滚能力可能降级为仅对话撤销”的宿主能力边界，这属于设计内行为，不是未完成项。
