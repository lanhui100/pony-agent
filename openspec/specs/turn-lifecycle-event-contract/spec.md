## ADDED Requirements

### Requirement: Runtime SHALL expose a canonical turn lifecycle
Pony Agent SHALL 为每个 turn 暴露统一的生命周期阶段语义，使执行、trace、checkpoint 与前端消费基于同一套 phase vocabulary。

#### Scenario: A normal turn completes after tool follow-up
- **WHEN** 一个 turn 经历上下文构建、模型调用、工具执行、工具结果整合并最终返回 assistant 结果
- **THEN** 该 turn SHALL 按 canonical phase 依次经过 `preparing`、`building_context`、`calling_model`、`streaming_response`、`executing_tool`、`tool_result_integrating`、`calling_model`、`streaming_response`、`checkpointing` 与 `completed`
- **AND** 该 turn SHALL 只拥有一个终态 `completed`

#### Scenario: A turn fails before any tool execution
- **WHEN** turn 在首次模型调用阶段直接失败
- **THEN** 该 turn SHALL 进入 `failed`
- **AND** 该 turn SHALL NOT 再被标记为 `completed` 或 `cancelled`

#### Scenario: A running turn is stopped by the user
- **WHEN** 用户对运行中的 turn 发起停止或取消
- **THEN** 该 turn SHALL 进入 `cancelled`
- **AND** 后续读面与 trace SHALL 以 `cancelled` 作为唯一终态

### Requirement: Turn event stream SHALL be canonical and ordered
每个 turn 的 lifecycle event stream SHALL 使用统一事件名与稳定顺序，而不是由不同调用面各自发明阶段信号。

#### Scenario: Stream sink emits lifecycle events
- **WHEN** runtime 向 turn event sink 发射事件
- **THEN** 每个事件 SHALL 包含 `eventId`、`eventType`、`eventVersion`、`sessionId`、`turnId`、`sequence` 与 `emittedAtMs`
- **AND** 同一个 turn 的 `sequence` SHALL 单调递增

#### Scenario: Runtime emits terminal event
- **WHEN** 一个 turn 结束于 `completed`、`failed` 或 `cancelled`
- **THEN** 事件流 SHALL 发出与终态一致的 terminal event
- **AND** terminal event 之后 SHALL NOT 再出现新的非 terminal lifecycle event

### Requirement: Canonical event vocabulary SHALL be explicit and stable
turn event stream SHALL 使用显式、稳定的一级事件名集合，避免不同调用面各自发明近义事件。

#### Scenario: Canonical event vocabulary is defined
- **WHEN** `PA-031` 交付 canonical lifecycle contract
- **THEN** 一级事件名集合 SHALL 至少包含 `turn.created`、`turn.phase_changed`、`turn.context_built`、`turn.model_call_started`、`turn.first_token`、`turn.output_delta`、`turn.tool_call_started`、`turn.tool_call_completed`、`turn.trace_updated`、`turn.checkpoint_persisted`、`turn.completed`、`turn.failed` 与 `turn.cancelled`
- **AND** 后续扩展事件 SHALL NOT 改写这些既有事件的语义

### Requirement: Model hops and tool hops SHALL be represented explicitly
一个 turn 内的多次模型调用与工具调用 SHALL 被 canonical contract 明确表达，而不是依赖前端通过消息或 trace 回推。

#### Scenario: Turn contains multiple model hops
- **WHEN** 一个 turn 因 tool follow-up 包含两次或更多 `call_model`
- **THEN** 事件流与 phase/trace contract SHALL 能区分每次 model hop
- **AND** 前一个 hop SHALL NOT 吞并后一个 hop 的输出或指标

#### Scenario: Turn contains a tool execution
- **WHEN** 某次 model hop 产出工具调用
- **THEN** lifecycle contract SHALL 显式记录 tool execution boundary
- **AND** 工具执行完成后 SHALL 显式记录回到 follow-up integration / next model hop 的边界

### Requirement: Checkpointing phase SHALL not imply recovery capability
`checkpointing` phase SHALL 只表达 turn 生命周期边界，不单独承诺该检查点具备跨重启恢复能力。

#### Scenario: A turn enters checkpointing
- **WHEN** turn 进入 `checkpointing`
- **THEN** 该 phase SHALL 仅表示正在发生检查点或持久化提交边界
- **AND** 调用方 SHALL NOT 因此自动推断已经生成 `recovery checkpoint`
- **AND** recovery-capable checkpoint 的语义 SHALL 由 recovery contract 单独定义

### Requirement: Frontend consumption SHALL prefer canonical lifecycle data
前端 runtime/store 与调试读面 SHALL 优先消费 canonical lifecycle contract，而不是继续发明新的隐式 lifecycle 推导规则。

#### Scenario: Canonical lifecycle data is available
- **WHEN** 后端读面已提供 canonical phase 与 event data
- **THEN** 前端 SHALL 直接消费这些字段
- **AND** 前端 SHALL NOT 为这些已有字段重新推导冲突的 lifecycle 语义

#### Scenario: A regression test validates lifecycle consumption
- **WHEN** `PA-031` 完成交付
- **THEN** Rust 与前端测试 SHALL 覆盖正常完成、失败、取消和多 hop 四类 turn 生命周期
- **AND** 验证记录 SHALL 进入任务卡与会话日志
