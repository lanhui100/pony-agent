## Why

`PA-031`、`PA-032` 与 `PA-033` 已经把 turn lifecycle、trace/recovery contract 与 hooks boundary 说清楚，但 `checkpointing` 仍主要停留在文档和 contract 层：

- hooks 已声明 `before_checkpoint_persist / after_checkpoint_persist`
- lifecycle 母文档已声明 `checkpointing` phase 与 `turn.checkpoint_persisted`
- recovery contract 也已强调 checkpoint boundary 不自动等于 recovery capability

但 runtime 真正完成持久化前后，还没有稳定发射与持久化这组边界。结果是：

- hooks 缺少真实锚点
- trace / reload 看不到 checkpoint boundary 是否发生
- execution checkpoint / 前端读面仍只能侧向猜测持久化阶段

因此需要单独落一张实现卡，把 `checkpointing` boundary 从 contract 变成事实源。

## What Changes

- 为 turn 正常完成链路补齐 checkpoint persist start/end 的 lifecycle 发射面
- 让 persisted trace / session snapshot 能保留 checkpoint lifecycle evidence
- 让 execution checkpoint / session hydration 读面能消费这组 evidence
- 明确该 boundary 只表达生命周期事实，不自动升级为 recovery checkpoint

## Capabilities

### New Capabilities

- `checkpoint-lifecycle-boundary`

### Modified Capabilities

- `turn-lifecycle-event-contract`
- `trace-persistence-and-recovery-contract`
- `agent-hooks-pipeline-foundation`
