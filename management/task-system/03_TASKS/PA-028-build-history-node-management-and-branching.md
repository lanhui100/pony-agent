# PA-028 构建历史节点管理、撤销恢复与分支化运行

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`
- Started At: `2026-06-01`
- Completed At: `2026-06-01`

## 目标
为 Pony Agent 补齐跨端通用的历史节点管理能力，在 core 统一支持历史节点、分支、游标、checkout、restore、fork 与 branch switch，并在 Tauri 前端提供可操作的历史管理交互。

## 实际交付
- Rust core 在 `session/runtime/control_plane` 打通历史图数据模型、持久化、查询、checkout、restore、fork、switch branch。
- retrieval / runtime view / turn input 支持 `nodeId`，可以按指定历史节点重建上下文与运行态。
- 历史 checkout 支持 `transcript_only` 与 `transcript_and_workspace`，并在宿主不支持工作区回滚时显式降级。
- Tauri 前端 runtime store、类型定义与 `HomeSessionSidebar` 已支持历史模式提示、恢复分支头、从历史节点分叉、切换分支和节点跳转。
- OpenSpec proposal / design / tasks / spec 已与实现保持一致。

## 关键设计结论
- 历史记录建模为不可变 `HistoryNode` 图，而不是可变线性会话历史。
- “撤销”语义是 checkout 到历史节点，不删除后续节点。
- 分支是显式实体，`latest` 以分支 head 定义，不以全局最后节点偷代。
- 工作区回滚能力通过宿主抽象暴露，不把 core 绑定到某个具体前端或 Git 实现。
- 在历史节点上继续产生新动作时自动形成 fork；未产生新动作时可以恢复到原分支 head。

## 验收结果
- 已支持区分当前可见节点、当前活动分支与当前分支 head。
- 已支持历史 checkout、恢复分支 head、从历史节点分叉、切换旧分支。
- 已支持基于历史节点读取 session runtime view 与 retrieved context。
- 已支持前端历史态提示与分支操作入口。
- 已支持工作区回滚不可用时降级为仅对话撤销，并向前端返回降级信息。

## 验证记录
- Rust:
  - `cargo test --manifest-path src-tauri/Cargo.toml history_commands_and_runtime_view_follow_persisted_history_graph --target-dir target-check-tests-b`
  - `cargo test --manifest-path src-tauri/Cargo.toml appending_from_historical_node_creates_a_fork_and_preserves_main_branch_head --target-dir target-check-tests-b`
  - `cargo test --manifest-path src-tauri/Cargo.toml restore_and_switch_history_branch_move_cursor_between_branch_heads --target-dir target-check-tests-b`
- Frontend:
  - `cmd /c npm run test:unit -- tests/HomeSessionSidebar.spec.ts tests/runtime-store.spec.ts`

## 关联实现
- [session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)
- [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)
- [spec change](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-01-add-history-node-management)
