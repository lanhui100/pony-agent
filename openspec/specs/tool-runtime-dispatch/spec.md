# tool-runtime-dispatch Spec

## ADDED Requirements

### Requirement: Every executable tool SHALL be represented by one registry descriptor

Pony Agent SHALL 以单一 registry descriptor 表达工具身份、schema、暴露策略、权限声明、执行策略与 handler 来源。

#### Scenario: A builtin tool is registered
- **WHEN** runtime 注册 builtin 工具
- **THEN** provider、planner、capability registry、trace 与 frontend contract view SHALL 从同一 descriptor 投影
- **AND** SHALL NOT 通过工具数组长度或平行名称表猜测其身份

### Requirement: All tool calls SHALL use one dispatcher lifecycle

Pony Agent SHALL 让 builtin、MCP、skill、dynamic 和 composite child 调用经过同一 dispatcher 生命周期。

#### Scenario: A composite invokes a child tool
- **WHEN** composite handler 请求执行一个 child tool
- **THEN** child SHALL 独立经过解析、校验、权限、hooks、预算、执行与 telemetry
- **AND** composite SHALL NOT 直接调用 child handler 或内部 router 方法

#### Scenario: A dispatch is cancelled
- **WHEN** turn、session 或 host 取消正在执行的工具
- **THEN** dispatcher SHALL 向 handler 传播取消
- **AND** SHALL 产生结构化 cancelled 结果与终态证据

### Requirement: Invocation origin SHALL authorize executability

Pony Agent SHALL 将模型可见性与调用授权分离，并让 dispatcher 依据不可伪造的 invocation origin 执行授权。

#### Scenario: A model guesses an internal or un-elevated deferred name
- **WHEN** provider 返回不在当前 `TurnToolView` 中的 internal、hidden 或 deferred descriptor 名称
- **THEN** dispatcher SHALL 在 handler 前返回 `tool_not_authorized_for_origin`
- **AND** SHALL NOT 把“已注册”或“可被 host 调用”视为模型调用许可

#### Scenario: A composite calls an internal child
- **WHEN** composite 通过 `ChildDispatch` 调用 internal descriptor
- **THEN** child context SHALL 包含 parent call lineage 与 allowed-child authority
- **AND** dispatcher SHALL 拒绝没有该 authority 的同名顶层调用

### Requirement: Mutable hooks SHALL be re-authorized against final inputs

Pony Agent SHALL 在任何可改写 hook 之后对最终 identity 和参数重新校验、规范化、鉴权与 sandbox 决策。

#### Scenario: A hook rewrites a policy-sensitive argument
- **WHEN** pre-dispatch hook 将 path、URL、command、capability id 或其他安全相关参数改写
- **THEN** dispatcher SHALL 对改写后的输入重新执行 schema、工具级校验与 policy evaluation
- **AND** approval 或 pending request SHALL 绑定最终参数摘要
- **AND** hook SHALL NOT 在最终鉴权后继续改写这些输入

### Requirement: Tool outcomes SHALL separate execution from control

Pony Agent SHALL 使用结构化 `ToolOutcome` 表达有限执行状态与正交 control outcome；handler 不得把 pending control 伪装为普通 tool result。

#### Scenario: A tool requests user interaction or approval
- **WHEN** handler 或 policy 返回 waiting-user、waiting-host 或 approval-required control outcome
- **THEN** runtime SHALL 在生成 provider follow-up 前持久化并暂停该 invocation
- **AND** SHALL NOT 把该 pending control outcome 编码为 `ToolResult.status`
- **AND** 恢复后 SHALL 以原 tool call id 生成唯一终态 tool result

### Requirement: Nested dispatch SHALL be bounded

Pony Agent SHALL 为 nested tool dispatch 设置深度、调用数、时间、并发和输出预算。

#### Scenario: A composite exceeds a nested budget
- **WHEN** composite 超出任一剩余预算或形成调用环
- **THEN** dispatcher SHALL 停止新增 child 执行
- **AND** SHALL 返回结构化 budget 或 cycle failure

#### Scenario: Parallel children race for a shared budget
- **WHEN** 多个 child 并发申请 calls、bytes、deadline、output 或 concurrency budget
- **THEN** dispatcher SHALL 使用共享原子账本预留资源
- **AND** SHALL NOT 让多个 child 因复制旧余额而超卖预算
- **AND** partial result SHALL 按稳定 child order 表达未启动或取消的步骤

### Requirement: Exposure SHALL control model visibility independently from executability

Pony Agent SHALL 区分 direct、internal、deferred 和 hidden/dispatch-only 工具。

#### Scenario: A deferred tool has not been elevated
- **WHEN** provider 构建当前 turn 的模型工具表
- **THEN** 未提升的 deferred tool SHALL NOT 进入模型可见 schema
- **AND** registry MAY 保留其可执行 descriptor

#### Scenario: ToolSearch elevates a tool
- **WHEN** 当前 turn 选择一个 deferred tool candidate
- **THEN** runtime SHALL 仅为当前 turn 加入该工具完整 schema
- **AND** trace SHALL 记录 elevation 来源与工具身份
