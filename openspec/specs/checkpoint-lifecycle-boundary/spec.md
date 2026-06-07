## ADDED Requirements

### Requirement: Runtime SHALL emit canonical checkpoint lifecycle boundaries
Pony Agent 的 turn 执行链 SHALL 在真实持久化提交边界发射 canonical checkpoint lifecycle 事件，而不是只在文档里声明 `checkpointing`。

#### Scenario: A turn completes without tool execution
- **WHEN** 一个 turn 直接完成并进入最终持久化提交路径
- **THEN** runtime SHALL 在 terminal event 之前显式经过 `checkpointing`
- **AND** runtime SHALL 发射与该边界一致的 canonical lifecycle evidence

#### Scenario: A turn completes after tool follow-up
- **WHEN** 一个 turn 经历 tool execution 和 follow-up 后完成
- **THEN** runtime SHALL 在 `turn.completed` 之前显式发射 checkpoint persist boundary
- **AND** 该 boundary SHALL 不吞并既有 `turn.tool_call_completed` 或 `turn.model_call_started` 语义

### Requirement: Checkpoint persist boundaries SHALL align with hooks contract
`CheckpointPersistStart / End` SHALL 对应真实 lifecycle boundary，而不是 foundation 内的悬空 contract。

#### Scenario: Runtime reaches the persistence commit boundary
- **WHEN** 一个 turn 到达最终持久化提交边界
- **THEN** hooks `CheckpointPersistStart` SHALL 能对齐 canonical `checkpointing` boundary
- **AND** hooks `CheckpointPersistEnd` SHALL 能对齐 `turn.checkpoint_persisted`

#### Scenario: Terminal finalize follows checkpoint persistence
- **WHEN** 一个 turn 进入 terminal finalize
- **THEN** `TurnFinalizeStart / End` SHALL 发生在 checkpoint persist boundary 之后
- **AND** terminal event SHALL NOT 先于 `turn.checkpoint_persisted`

### Requirement: Persisted trace SHALL retain checkpoint lifecycle evidence
checkpoint lifecycle boundary 既是流事件，也 SHALL 成为 persisted trace / reload 可见的证据。

#### Scenario: Application reloads after a completed turn
- **WHEN** 一个带 checkpoint persist boundary 的 turn 已被持久化，并在稍后 reload session
- **THEN** persisted trace SHALL 仍保留该 turn 的 checkpoint lifecycle evidence
- **AND** 前端 hydration SHALL 能读回这组 evidence

#### Scenario: Runtime has no in-memory stream state
- **WHEN** 前端只能读取 session snapshot / persisted trace
- **THEN** 它 SHALL 仍能判断 checkpoint lifecycle boundary 已发生
- **AND** 不得因缺少内存态而把该 boundary 静默丢失

### Requirement: Checkpoint lifecycle SHALL not imply recovery capability
`checkpointing` 与 `turn.checkpoint_persisted` SHALL 只表达生命周期事实，不得自动承诺 recovery capability。

#### Scenario: Checkpoint lifecycle evidence exists
- **WHEN** 某个 turn 已持有 `checkpointing` 或 `turn.checkpoint_persisted` evidence
- **THEN** 调用方 SHALL NOT 仅凭这组 evidence 推断存在 `recovery` checkpoint
- **AND** recovery capability SHALL 继续以后端 `checkpointKind / recoveryMode / resumable / replayable` 合同为准

#### Scenario: Runtime control checkpoint coexists with checkpoint lifecycle evidence
- **WHEN** 同一个 session 同时可读到 runtime control checkpoint 与 checkpoint lifecycle evidence
- **THEN** 系统 SHALL 能区分“生命周期边界已发生”与“可恢复恢复点可用”是两个不同判断

### Requirement: Delivery SHALL be verified across runtime, reload, and frontend consumption
`PA-034` 的交付 SHALL 由后端、reload 与前端消费三层证据共同验证。

#### Scenario: Delivery completes
- **WHEN** `PA-034` 完成交付
- **THEN** 测试 SHALL 至少覆盖 normal completion、tool follow-up completion、reload roundtrip 与 recovery non-implication
- **AND** 验收证据 SHALL 被写回任务卡、review 文档与 session 日志
