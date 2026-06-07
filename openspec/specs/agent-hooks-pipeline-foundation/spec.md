## ADDED Requirements

### Requirement: Hooks SHALL attach only to canonical lifecycle boundaries
Pony Agent 的 hooks SHALL 只允许挂接在 canonical lifecycle boundary 上，而不是任意插入 runtime 内部细节。

#### Scenario: A hook is registered for model invocation
- **WHEN** 系统注册一个与模型调用相关的 hook
- **THEN** 该 hook SHALL 绑定到 `before_model_call` 或 `after_model_response` 等 canonical boundary
- **AND** 系统 SHALL NOT 要求 hook 了解 provider 内部流解析细节

#### Scenario: A hook tries to attach to an internal unstable step
- **WHEN** 某个 hook 试图挂接未被 contract 承诺的内部实现点
- **THEN** 系统 SHALL 拒绝该 hook 注册或将其视为无效配置

#### Scenario: A hook point maps to canonical lifecycle vocabulary
- **WHEN** foundation 为某个 `TurnHookPoint` 声明 lifecycle binding
- **THEN** 该 binding SHALL 只引用 canonical `eventType` vocabulary
- **AND** 该 binding SHALL 只引用 canonical `phase` vocabulary
- **AND** `start/end` boundary SHALL NOT 自行引入新的 phase 名称

#### Scenario: A hook point lands after tool execution
- **WHEN** foundation 解析 `ToolCallEnd`
- **THEN** 它 SHALL 绑定到 `turn.tool_call_completed`
- **AND** 它 SHALL 对齐 `tool_result_integrating`
- **AND** 它 SHALL NOT 回退成 runtime 私有的 `executing_tool` 结束猜测

### Requirement: Hooks SHALL declare class, failure policy, and recovery mode
每个 hook SHALL 显式声明自己的能力类别、失败策略与恢复模式，使主执行链能稳定地处理其行为。

#### Scenario: A guard hook denies execution
- **WHEN** `guard` 类 hook 返回拒绝结果
- **THEN** runtime SHALL 以结构化方式中断后续执行
- **AND** trace SHALL 记录该 hook 的拒绝原因
- **AND** 该 hook SHALL 仅在 `canBlock=true` 时允许产生 `deny`

#### Scenario: An observe hook times out
- **WHEN** `observe` 类 hook 超时或失败
- **THEN** 系统 SHALL 按声明的 failure policy 降级处理
- **AND** 系统 SHALL NOT 默认把整个 turn 标记为 failed，除非该 hook 明确声明会阻断主流程

#### Scenario: A transform hook participates in recovery
- **WHEN** 某个 `transform` hook 被用于上下文或工具参数改写
- **THEN** 它 SHALL 声明 `replay_required` 或 `persisted_effect` 等 recovery mode
- **AND** 恢复链路 SHALL 能区分是否需要重放该 hook

#### Scenario: A hook declares runtime safety and replay evidence requirements
- **WHEN** 某个 hook 被注册到 foundation registry
- **THEN** 它 SHALL 显式声明 `timeoutMs`
- **AND** 它 SHALL 显式声明 `replayRequirements`
- **AND** 它 SHALL 显式声明 `sideEffectPersistenceRequirements`

#### Scenario: A persisted_effect hook omits persistence evidence requirements
- **WHEN** 某个 hook 声明 `persisted_effect`
- **AND** 它没有要求 persistence evidence
- **THEN** foundation registry SHALL 拒绝该 descriptor
- **OR** 将其显式降级为 `replay_required`

### Requirement: Hook ordering and conflict handling SHALL be stable
同一 lifecycle boundary 上的多个 hooks SHALL 按稳定顺序执行，并对结果冲突有固定裁决规则。

#### Scenario: Multiple hooks are registered on the same boundary
- **WHEN** 同一 boundary 上注册了多个 hooks
- **THEN** 系统 SHALL 以稳定、可预测的顺序执行它们
- **AND** 该顺序 SHALL 不因调用面不同而漂移

#### Scenario: Multiple transform hooks patch the same target
- **WHEN** 多个 `transform` hook 对同一输入目标产生 patch
- **THEN** 系统 SHALL 使用固定冲突裁决规则
- **AND** 该裁决结果 SHALL 可进入 trace / audit

### Requirement: Hook effects SHALL flow through controlled interfaces
hooks SHALL 通过结构化结果影响系统，而不是直接任意修改 runtime、session、checkpoint 或 trace store。

#### Scenario: A transform hook modifies tool arguments
- **WHEN** transform hook 要修改工具参数
- **THEN** 它 SHALL 返回结构化 patch 或等价受控结果
- **AND** lifecycle orchestration 层 SHALL 决定如何应用该结果

#### Scenario: A hook returns a structured result
- **WHEN** hooks foundation 对某个 hook 结果进行归一化
- **THEN** 结果 SHALL 被约束为 `observe / allow / deny / patch / side-effect-request` 五类之一
- **AND** `deny` SHALL 携带结构化原因
- **AND** `patch` SHALL 携带目标与操作摘要
- **AND** `side-effect-request` SHALL 声明是否要求持久化证据

#### Scenario: A hook attempts direct store mutation
- **WHEN** 某个 hook 试图直接改写内部 session/trace/checkpoint store
- **THEN** 该行为 SHALL 被视为越界
- **AND** hooks foundation SHALL 不以此作为正式扩展模型

#### Scenario: A side-effect hook requests external work
- **WHEN** `side_effect` hook 返回 `side-effect-request`
- **THEN** canonical runtime contract applier SHALL 决定是否执行该请求
- **AND** 该请求 SHALL NOT 创建新 lifecycle phase
- **AND** 该请求 SHALL NOT 直接触发嵌套 turn 调度或绕开 canonical runtime path 的 model/tool hop

### Requirement: Recovery modes SHALL map to persistence evidence
hooks 的恢复模式 SHALL 对应明确的最小持久化证据，避免实现阶段临场发明恢复口径。

#### Scenario: Hook declares persisted_effect
- **WHEN** 某个 hook 声明 `persisted_effect`
- **THEN** 系统 SHALL 为该 hook 持久化已应用效果的引用或归一化摘要
- **AND** 若缺少该证据，系统 SHALL 将其视为 `replay_required`

#### Scenario: Hook declares replay_required
- **WHEN** 某个 hook 声明 `replay_required`
- **THEN** recovery 链路 SHALL 至少能读取该 hook 的标识、boundary、顺序号与输入摘要，以支持重放判定

### Requirement: Hook execution SHALL be traceable and auditable
hook 的执行过程、耗时、结果与副作用 SHALL 进入 trace / audit 读面，确保工业项目中的可调试性与可审计性。

#### Scenario: Hook executes during a turn
- **WHEN** 某个 hook 在 turn 生命周期中被执行
- **THEN** trace SHALL 能显示该 hook 的名称、类型、阶段、结果与耗时
- **AND** 排障者 SHALL 能判断该 hook 是否改变了执行结果

#### Scenario: Hook trace records roundtrip through persisted turn trace
- **WHEN** 某个 turn trace 已携带 hook trace records
- **THEN** `TurnStreamEvent -> TurnResult -> TurnTraceRecord -> SessionSnapshot` 链路 SHALL 不丢失这些 records
- **AND** reload 之后的 persisted turn trace SHALL 仍能读到同一组 hook trace records
- **AND** 前端 hydration/store clone 路径 SHALL 能无损消费这些 records

#### Scenario: Delivery completes
- **WHEN** `PA-033` 完成交付
- **THEN** 测试 SHALL 覆盖 hook 分类、边界、失败语义、恢复语义与 traceability
- **AND** 验收证据 SHALL 被回写到任务卡与 review 文档
