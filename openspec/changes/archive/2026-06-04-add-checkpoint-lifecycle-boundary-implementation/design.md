## Context

当前仓库已经有三层前置结论：

1. `PA-031` 定义了 canonical lifecycle vocabulary，并明确 `checkpointing` 只代表生命周期边界。
2. `PA-032` 定义了 runtime checkpoint 与 recovery checkpoint 的区分，避免把任何 checkpoint 都冒充成可恢复断点。
3. `PA-033` 为 hooks foundation 定义了 `CheckpointPersistStart / CheckpointPersistEnd`，但还没有真实 runtime 锚点可挂。

因此现在最缺的不是更多术语，而是把 checkpoint persist boundary 做成可发射、可持久化、可 reload、可测试的事实。

## Goals / Non-Goals

**Goals**

- 为 turn 正常完成链路补真实 `checkpointing` boundary
- 让 `turn.checkpoint_persisted` 在 runtime / trace / reload 中可观察
- 让 hooks `CheckpointPersistStart / End` 与真实 lifecycle 锚点对齐
- 保持 runtime checkpoint 与 recovery checkpoint 的合同分层不被破坏
- 以 `src-tauri` 的 runtime / session / execution checkpoint / control plane 为最小后端闭环
- 以前端 `runtime store` 作为最小消费闭环，而不是先扩散到更多 UI 组件

**Non-Goals**

- 本轮不实现新的 graph recovery mode
- 本轮不把所有 history / rollback 行为都改造成 checkpoint 事件源
- 本轮不接 hooks runtime dispatch，只补真实 lifecycle boundary
- 本轮不把前端 UI 全面重构为新的 checkpoint 面板

## Decisions

### 1. checkpoint boundary 先落在 turn 终态持久化提交边界

做法：

- 对当前 turn 完成后会调用的持久化提交路径补 `CheckpointPersistStart`
- 持久化完成并拿到最终 persisted 结果后补 `CheckpointPersistEnd`
- terminal event 仍由 `TurnFinalizeEnd` 承接，但必须发生在 checkpoint boundary 之后

原因：

- 这是当前代码里最稳定、最容易验证的一段真实提交边界
- 它正好连接 lifecycle、trace persistence、execution checkpoint 与 hooks boundary

### 2. checkpoint lifecycle evidence 必须可 reload

做法：

- persisted trace 至少保留 checkpoint boundary 的 canonical phase / event evidence
- reload 后前端与后端读面能读回这部分事实

原因：

- 如果只发流事件、不保留 persisted evidence，hooks traceability 和 recovery audit 仍然是不完整的

### 3. checkpoint boundary 不自动升级 recovery capability

做法：

- `checkpointing` / `turn.checkpoint_persisted` 只表示“发生了持久化提交边界”
- 是否可恢复仍由 `checkpointKind / recoveryMode / resumable / replayable` 决定

原因：

- 避免这张实现卡反向污染 `PA-032` 已经澄清的恢复合同

### 4. 最小落地范围优先覆盖 normal completion 与 tool follow-up completion

做法：

- 先保证无工具与有工具两条正常完成链路都具备 checkpoint boundary
- failed / cancelled 若没有真实提交边界，不强行伪造 `turn.checkpoint_persisted`

原因：

- 完成态是最稳定且最有 persisted trace 价值的路径
- 失败/取消路径是否具备 checkpoint persist 需要依据真实写入点，不能为了对称性编造事件

### 5. phase/status 与事实源优先级必须保持兼容

做法：

- 在 `checkpointing` 边界期间，execution checkpoint 可以投影为 UI `connecting`
- 但不得过早把 `status` 提前终结为 `completed`
- persisted trace 与 execution checkpoint 若同时存在，必须维持既有“运行中优先 runtime_control，reload 后优先 persisted truth”的合同

原因：

- 当前前端已经依赖 `phase / status / projectedRuntimePhase` 的组合
- 若只补事件、不守住优先级，UI 很容易出现 phase 抖动或 checkpoint boundary 被 terminal 吞没

## Implementation Outline

1. 在 `src-tauri/src/agent/runtime.rs` 为 turn 持久化提交点补 `checkpointing` phase 与 `turn.checkpoint_persisted` 事件发射
2. 在 `src-tauri/src/agent/session.rs` 的 persisted trace 路径中保留 checkpoint lifecycle evidence，并保持历史空字段兼容
3. 在 `src-tauri/src/agent/execution_control.rs` / `src-tauri/src/agent/control_plane.rs` 校准 checkpoint 边界期间的 phase/status/projection 读取
4. 在 `src/stores/runtime.ts` 校验 hydration / reload / event replay 对新增 boundary 的消费
5. 为 hooks canonical binding 增加 runtime 真实路径测试

## Verification Strategy

- Rust 流式 turn 测试覆盖 normal completion 与 tool follow-up completion 的 checkpoint boundary 顺序
- Rust roundtrip 测试覆盖 persisted trace reload 后仍保留 checkpoint lifecycle evidence
- Rust/前端测试覆盖 checkpoint lifecycle 与 recovery capability 不互相冒充
- 验证 backward-compatible default：历史 session 缺少 checkpoint lifecycle evidence 时，读面仍能正常降级
- 任务卡、review 文档与 session 日志同步回写验收证据
