# PA-068 接入 per-session async turn task 模型

## 状态
- Status: `Ready`
- Priority: `P1`
- Owner: `待定`

## 依赖
- 前置：`PA-065`、`PA-066`
- 强依赖：`PA-067` 的 blocking helper 落位

## Canonical Spec
- `openspec/changes/adopt-per-session-async-turn-tasks/specs/turn-lifecycle-event-contract/spec.md`

## OpenSpec Change
- `openspec/changes/adopt-per-session-async-turn-tasks/`

## 背景
完成 ownership 拆分与 provider async 化后，才真正具备把 turn 作为 per-session / per-run async task 运行的条件。该任务负责把现有 `spawn_blocking + global runtime lock` 模式切到事件驱动 async task 模型。

## 目标
1. 每个 turn 作为独立 async task 执行
2. 多 session 任务真正并发运行
3. 继续复用现有事件系统把状态推送回前端

## In Scope
- `tauri::async_runtime::spawn` 接管 turn / graph run 执行
- per-session / per-run task ownership 建模
- cancellation / resume / terminal cleanup 在 async task 模型下重新接线
- 前端事件消费合同兼容性验证

## Out of Scope
- 首屏渲染与 markdown 渲染优化
- 非 turn 路径的全仓 async 化

## 验收标准
- 多个 session SHALL 能真正并发执行 turn，而不是串行等待
- 前端切换历史会话 SHALL 不再受后台 turn 执行阻塞
- turn event contract SHALL 在 async task 模型下保持兼容
- turn cancellation SHALL 使用 async-native 机制而非 abort 整个 runtime
- 集成测试 SHALL 验证多 session 并发执行无串扰
- 旧的 `spawn_blocking` + global lock 模式 SHALL 被彻底移除

## 下一步动作
1. 先实现最小 per-session task spike
2. 用 graph run stream 路径替换现有 blocking spawn
3. 收口 cancellation / terminal event / cleanup

## 断点续跑提示
- `src-tauri/src/tauri_adapter.rs`
- `crates/pony-agent-core/src/agent/control_plane.rs`
- `crates/pony-agent-core/src/agent/runtime/mod.rs`
- `crates/pony-agent-core/src/agent/graph.rs`

## 当前进展
- 已完成

## 完成摘要
- 创建 `TurnTaskRegistry`：per-session 异步任务追踪、自动取消旧任务、`abort_all` 挂钩窗口关闭
- `spawn_turn_stream`/`spawn_graph_run_stream` 从 `spawn_blocking` 切换到 `tauri::async_runtime::spawn` + 内层 `spawn_blocking` 的 async task 模型
- `TaskCleanupGuard`（Drop guard）自动从注册表反注册已完成任务
- 多 session 通过 `TurnTaskRegistry` 实现独立任务身份与生命周期管理
- Tauri app 测试 30 项 + TS 测试 231 项全部通过
- 3 轮并行智能体审核后调优：unregister/abort_all 完整接线
- 清理与收口已完成
