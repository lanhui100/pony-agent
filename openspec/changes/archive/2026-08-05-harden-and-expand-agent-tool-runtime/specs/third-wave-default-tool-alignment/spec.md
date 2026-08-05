## MODIFIED Requirements

### Requirement: Plan SHALL have a stable explicit-planning boundary

Pony Agent SHALL 将 `Plan` 定义为显式计划表达、计划更新或计划状态转换的稳定默认工具；执行计划由 planner/graph 后续显式决策，而不是由 `Plan` handler 任意调度 calls。

#### Scenario: The model wants to express a plan without immediate execution
- **WHEN** `Plan` 创建、替换、合并或更新计划步骤
- **THEN** 系统 SHALL 持久化结构化计划状态
- **AND** `Plan` SHALL NOT 直接执行计划步骤

#### Scenario: Plan drives follow-up execution
- **WHEN** planner/graph 根据当前计划选择后续执行
- **THEN** 每个执行步骤 SHALL 作为独立工具调用经过 dispatcher
- **AND** 执行细节 SHALL 通过统一 `ToolPlan` 与 `child_results` 暴露
- **AND** `Plan` SHALL NOT 接受通用 `calls` 数组作为任意批量执行入口

### Requirement: Ask SHALL remain a controlled clarification boundary

Pony Agent SHALL 将 `Ask` 定义为向用户或宿主请求澄清、确认或补充输入的可持久化控制工具。

#### Scenario: The model needs clarification
- **WHEN** `Ask` 被用于请求补充信息
- **THEN** runtime SHALL 生成结构化 interaction request
- **AND** graph/session SHALL 进入可恢复 waiting_user 状态
- **AND** SHALL NOT 把问题回显当成工具成功完成

#### Scenario: A pending Ask survives reload and resumes once
- **WHEN** Ask 已产生 interaction request 且应用在回答前 reload
- **THEN** session/graph/checkpoint SHALL 保留 pending request、原 assistant tool call 与状态版本
- **AND** 有效回答 SHALL 恢复原 run 并以原 tool call id 注入唯一终态结果
- **AND** 重复、取消或过期回答 SHALL NOT 再次恢复该 run

#### Scenario: Ask interacts through host mediation
- **WHEN** host 支持交互请求
- **THEN** host SHALL 展示请求并把回答关联到稳定 request id
- **AND** runtime SHALL 使用该回答恢复原 run

#### Scenario: Ask runs without host mediation
- **WHEN** 当前 host 不具备交互能力
- **THEN** `Ask` SHALL 返回 `interaction_unavailable` 或等价受控失败
- **AND** SHALL NOT 静默丢弃请求
- **AND** SHALL NOT 在消息投递之外产生副作用

## ADDED Requirements

### Requirement: ToolSearch SHALL elevate deferred tools for one turn

Pony Agent SHALL 让 ToolSearch 的选择结果能够把完整工具 schema 提升到当前 turn 的模型工具表。

#### Scenario: The model selects a deferred candidate
- **WHEN** ToolSearch 返回并选中一个 deferred tool reference
- **THEN** 后续 provider request SHALL 包含该工具完整 schema
- **AND** elevation SHALL 默认在 turn 结束时失效
- **AND** trace SHALL 记录 elevation evidence

#### Scenario: ToolSearch selects a candidate
- **WHEN** ToolSearch 收到当前 registry snapshot 中的 candidate reference 作为 select 参数
- **THEN** runtime SHALL 校验 descriptor/source revision、权限与当前 turn
- **AND** SHALL 在同一 user turn 的下一 provider hop 使用更新后的 `TurnToolView`
- **AND** turn 结束、source replacement、reload 或 retry SHALL 使过期 elevation 失效
