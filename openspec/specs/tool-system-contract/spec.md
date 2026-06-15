# tool-system-contract Spec

## ADDED Requirements

### Requirement: The system SHALL separate model-visible tool names from internal execution primitives

Pony Agent 的工具系统 SHALL 明确区分“模型可见工具协议层”和“内部执行原语层”，不得把底层实现名直接当作长期产品级模型协议。

#### Scenario: Existing workspace primitives remain reusable without becoming the public tool surface

- **GIVEN** runtime 已经存在 `workspace_read_file`、`workspace_read_file_segment`、`workspace_list_files`、`workspace_search_text` 等底层能力
- **WHEN** 系统定义长期工具协议
- **THEN** 这些底层能力 MAY 继续作为内部执行原语存在
- **AND** 系统 SHALL NOT 要求它们直接以同名形式作为长期模型可见工具协议

#### Scenario: Product-level tools use stable short names

- **WHEN** 系统向模型暴露首批产品级工具
- **THEN** 这些工具 SHALL 使用稳定、简洁、面向产品的英文名
- **AND** SHALL 与内部执行原语名分层，而不是简单复刻底层实现名

### Requirement: The system SHALL use a three-level tool identity model

Pony Agent SHALL 区分模型可见工具名、跨来源聚合标识与底层执行原语名。

#### Scenario: A product-level tool resolves to more than one primitive

- **WHEN** 某个模型可见工具根据输入或上下文解析到不同底层执行原语
- **THEN** 系统 SHALL 保留稳定的模型可见工具名
- **AND** SHALL 使用独立的 canonical tool name 作为跨来源聚合标识
- **AND** SHALL 使用独立的 execution primitive 记录最终执行原语

#### Scenario: Trace aggregates tool activity across different sources

- **WHEN** trace、telemetry 或 monitor 需要聚合同一逻辑工具的不同执行来源
- **THEN** 它们 SHALL 默认按 canonical tool name 聚合
- **AND** SHALL NOT 默认按 execution primitive 聚合

### Requirement: The system SHALL define a unified ToolDefinition contract

Pony Agent SHALL 为所有工具来源定义统一 `ToolDefinition` 合同。

#### Scenario: A built-in tool is exposed to the model

- **WHEN** 系统注册一个 builtin tool
- **THEN** 该工具 SHALL 至少提供模型可见工具名、canonical tool name、execution primitive、描述、输入 schema、输出 schema、分类、暴露策略与显示元数据

#### Scenario: A capability-backed tool is surfaced through runtime

- **WHEN** runtime 通过 capability bridge 暴露一个 capability-backed tool
- **THEN** 该能力 SHALL 能映射到同一套 `ToolDefinition` 合同
- **AND** SHALL NOT 发明第二套平行工具定义结构

### Requirement: The system SHALL define a unified ToolCall contract

Pony Agent SHALL 为所有工具调用定义统一 `ToolCall` 合同。

#### Scenario: A composite tool carries an explicit plan

- **WHEN** 某个工具调用带有显式 `ToolPlan`
- **THEN** 该计划 SHALL 通过统一 `ToolCall` 一等字段表达
- **AND** SHALL NOT 回退为仅靠隐式 JSON 约定

#### Scenario: A tool call participates in telemetry and trace

- **WHEN** runtime、trace 或 telemetry 记录某次工具调用
- **THEN** 它们 SHALL 能读取统一的模型可见工具名、canonical tool name、execution primitive、arguments、kind 与 exposure 元数据

### Requirement: The system SHALL define a unified ToolResult contract

Pony Agent SHALL 为所有工具执行结果定义统一 `ToolResult` 合同。

#### Scenario: A tool returns a successful structured result

- **WHEN** 某个工具执行成功
- **THEN** 系统 SHALL 能提供结构化 `status / summary / data / child_results / artifacts / duration_ms`
- **AND** 这些字段 SHALL 同时适用于模型 follow-up、trace 与前端展示

#### Scenario: A composite tool returns nested execution details

- **WHEN** 某个 composite tool 完成多个子步骤
- **THEN** 系统 SHALL 能把子步骤结果附着到统一 `ToolResult.child_results` 结构中
- **AND** SHALL NOT 要求前端或 trace 重新发明第二套嵌套结果协议

#### Scenario: Main result data and external artifacts are both present

- **WHEN** 某个工具同时返回主结果数据与外显附件/引用对象
- **THEN** 系统 SHALL 区分 `data` 与 `artifacts`
- **AND** SHALL NOT 把子步骤结果、主结果数据与附件对象混装进同一个万能字段

### Requirement: The system SHALL define structured tool failure kinds

Pony Agent SHALL 为工具失败定义统一结构化失败种类，而不是仅依赖自由文本错误。

#### Scenario: A file path is malformed

- **WHEN** 某个工具因输入不合法而失败
- **THEN** 该失败 SHALL 映射为 `invalid_input`

#### Scenario: A tool is denied by policy

- **WHEN** 某次工具调用被权限或边界策略拒绝
- **THEN** 系统 SHALL 至少能区分 `permission_denied`、`approval_required` 与 `out_of_scope`
- **AND** 自由文本错误说明 SHALL 仅作为补充信息

#### Scenario: A capability-backed action fails because its source is unavailable

- **WHEN** 某个 capability-backed tool 的来源不可用
- **THEN** 该失败 SHALL 能映射回统一失败种类
- **AND** SHALL NOT 被吞并成模糊的一般失败文本

### Requirement: The system SHALL define a structured tool error object

Pony Agent SHALL 为工具失败提供结构化错误对象，而不是只有失败 kind 和自由文本。

#### Scenario: A tool fails with structured error details

- **WHEN** 某个工具执行失败
- **THEN** `ToolResult.error` SHALL 至少包含 `kind` 与 `message`
- **AND** 若系统已知额外失败上下文，则 SHOULD 保留 `details`、`retryable` 或 `source`

### Requirement: The system SHALL define unified tool kinds

Pony Agent SHALL 定义稳定的工具分类体系。

#### Scenario: A tool is categorized for planner and presentation use

- **WHEN** 系统为工具附加分类元数据
- **THEN** 至少 SHALL 支持 `read / search / write / execute / plan / interactive / composite / external`
- **AND** planner、telemetry、前端展示与权限合同 SHALL 复用该分类

### Requirement: The system SHALL define unified exposure strategies

Pony Agent SHALL 定义稳定的工具暴露策略。

#### Scenario: A tool is not meant to be shown to the model by default

- **WHEN** 某个工具只适合作为内部原语或待发现能力
- **THEN** 系统 SHALL 支持 `internal` 或 `deferred` 暴露策略
- **AND** SHALL NOT 强迫所有已注册工具都默认进入模型可见工具表

#### Scenario: A tool is part of the default visible tool surface

- **WHEN** 某个工具属于长期模型可见工具面
- **THEN** 系统 SHALL 能把它标记为 `model_visible`

#### Scenario: A deferred tool is elevated for the current turn

- **WHEN** discover/search/host mediation 路径把某个 deferred 工具提升为当前 turn 可见
- **THEN** 该提升 SHALL 默认只作用于当前 turn
- **AND** trace SHALL 能记录该工具从 deferred 转为 visible

### Requirement: Display metadata SHALL remain presentation-layer data

工具中文短显示名与等价展示元数据 SHALL 属于展示层，而不是底层唯一工具标识。

#### Scenario: Frontend renders a tool activity

- **WHEN** 前端渲染一次工具活动
- **THEN** 它 MAY 使用中文短显示名展示
- **AND** runtime 与 trace 仍 SHALL 保留稳定英文工具标识

#### Scenario: The system localizes tool labels

- **WHEN** 系统增加或调整展示语言
- **THEN** 展示层元数据 MAY 变化
- **AND** 底层工具标识 SHALL 保持稳定，不因本地化而改变

### Requirement: Policy metadata MAY exist before approval semantics are fully defined

本 change MAY 为后续审批与权限合同保留策略元数据字段，但其正式语义 SHALL 由后续权限 change 收口。

#### Scenario: Tool definitions reserve approval-related metadata

- **WHEN** 某个工具定义包含审批或权限相关占位元数据
- **THEN** 系统 MAY 保留这类字段
- **AND** 本 change SHALL NOT 单独把它们扩展为完整审批策略真相源

### Requirement: Builtin, capability, skill, and composite tools SHALL reuse one tool contract family

Pony Agent SHALL 要求不同来源的工具复用同一套工具合同族，而不是各自发明第二套协议。

#### Scenario: A tool-only skill runs through runtime

- **WHEN** runtime 执行一个由 skill 组合出的工具能力
- **THEN** 该执行结果 SHALL 能映射回统一 `ToolResult` 与 `ToolFailureKind`

#### Scenario: A builtin tool and a capability-backed tool both appear in trace

- **WHEN** trace 或 monitor 同时观察 builtin 与 capability-backed 工具活动
- **THEN** 它们 SHALL 能通过统一工具定义、结果和失败合同被聚合与展示
