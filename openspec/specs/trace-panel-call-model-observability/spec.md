## ADDED Requirements

### Requirement: Trace panel SHALL expose per-call-model cache hit and TTFT metrics
前端 trace 面板中的每个 `call_model` 节点 SHALL 优先展示该次模型调用自己的 token 与延时指标，而不是只展示整轮 turn 聚合值。

#### Scenario: Final call-model hop only has turn-level aggregate metrics
- **WHEN** trace timeline 中最后一个 `call_model` 节点缺少内联 token/latency 字段，但 turn 级记录仍保留 `cacheHitInputTokens`、`outputTokens`、`firstTokenLatencyMs`、`turnDurationMs`
- **THEN** trace 面板 SHALL 仅对该最后一个 `call_model` 节点回退使用 turn 级聚合值
- **AND** 该节点 SHALL 显示缓存命中、TTFT、输出 token 与耗时

#### Scenario: Multi-hop turn has provider call records
- **WHEN** 某轮 trace 同时包含多个 `call_model` 节点与对应的 `providerCallRecords`
- **THEN** 每个 `call_model` 节点 SHALL 优先展示与自己 index 对齐的单次调用指标
- **AND** 前一个 `call_model` 节点 SHALL NOT 借用后一个 hop 的缓存命中、TTFT 或输出 token

#### Scenario: Provider only returns buffered response latency
- **WHEN** provider call record 的 `latencyKind` 不是 `provider_stream`
- **THEN** trace 面板 SHALL NOT 伪造或展示 TTFT
- **AND** 速度计算 SHALL 继续基于可用的整次调用耗时

### Requirement: Trace panel SHALL show truthful call-model outputs
前端 trace 面板中的 `call_model` 明细 SHALL 如实展示该次模型调用产出的内容，包括工具调用输出与消息输出，而不是只回填最终 assistant 文本。

#### Scenario: Model emits tool calls and no final assistant text in that hop
- **WHEN** 某个 `call_model` 节点之后、下一个 `call_model` 节点之前存在一个或多个 `call_tool` 节点
- **THEN** 该 `call_model` 明细 SHALL 展示这些工具调用作为“本次模型调用的输出”
- **AND** 如果该 `call_model` 节点自身没有消息文本，则明细 SHALL NOT 伪造“模型输出”文本块

#### Scenario: Model emits assistant text in that hop
- **WHEN** 某个 `call_model` 节点自身携带 `text`
- **THEN** trace 面板 SHALL 展示该文本作为这次模型调用的消息输出
- **AND** 文本内容 SHALL 与该 hop 实际 trace/message 数据一致，不得改写为别的 hop 的最终回答

#### Scenario: Model emits both reasoning and output
- **WHEN** 某个 `call_model` 节点同时携带 `reasoningContent` 与 `text`
- **THEN** 明细 SHALL 分别展示 `思考链` 与 `模型输出`
- **AND** 两者 SHALL 继续允许独立复制与审阅

### Requirement: Trace panel SHALL preserve multi-hop output attribution
前端 trace 面板 SHALL 保持多 hop 时的输出归因稳定，使用户能够看出“哪次模型调用触发了哪些工具调用，哪次模型调用产出了最终消息”。

#### Scenario: Tool-followup turn contains two model hops
- **WHEN** 第一跳 `call_model` 只产出工具调用，第二跳 `call_model` 产出最终 assistant 文本
- **THEN** 第一跳明细 SHALL 只展示它触发的工具调用输出
- **AND** 第一跳 SHALL NOT 展示第二跳的最终 assistant 文本
- **AND** 第二跳 SHALL 展示自己的 assistant 文本与 reasoning 内容

#### Scenario: Tool step remains independently inspectable
- **WHEN** 用户同时查看 `call_model` 和 `call_tool` 节点
- **THEN** `call_tool` 节点 SHALL 继续保留独立的执行结果、参数与耗时明细
- **AND** `call_model` 中的工具调用输出展示 SHALL NOT 删除或替代 `call_tool` 节点

### Requirement: Frontend acceptance SHALL be locked by high-level regression tests
本能力的验收 SHALL 通过前端高层联动测试锁定，而不是依赖人工目测。

#### Scenario: Delivery completes
- **WHEN** 本变更完成实现
- **THEN** `tests/HomeSidebar.spec.ts` SHALL 覆盖单 hop、multi-hop、工具调用输出、消息输出、cache hit、TTFT 与 buffered latency 行为
- **AND** `tests/runtime-store.spec.ts` SHALL 覆盖 trace persistence / event hydration 不破坏上述数据
- **AND** 相关前端测试命令 SHALL 全部通过
