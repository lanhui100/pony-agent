# third-wave-default-tool-alignment Spec

## Scope

This canonical spec captures the stable product-level contract for the third-wave default tool alignment.
It focuses on the long-lived boundary, sequencing, and non-goals for:

- `Plan`
- `Ask`
- `MCP Resource`
- `ToolSearch`
- `Run`

Implementation notes, temporary compatibility choices, and per-session review details SHOULD remain in change artifacts or task-system records rather than being expanded here.

## ADDED Requirements

### Requirement: The system SHALL align a third wave of default tools as one implementation package

Pony Agent SHALL 把第三波默认工具对齐实现视为同一张正式实现任务，而不是重新拆散成互不约束的零散修补。

#### Scenario: A planning document considers the next default-tool improvements

- **WHEN** 系统规划下一轮默认工具实现
- **THEN** 它 SHALL 将以下 5 个工具视为同一波次成员：
  - `Plan`
  - `Ask`
  - `MCP Resource`
  - `ToolSearch`
  - `Run`

#### Scenario: Engineers try to split the package back into unrelated slices

- **WHEN** 后续实现尝试只抽取其中一部分而不说明波次关系
- **THEN** 文档 SHALL 仍明确这些工具属于同一轮默认工具对齐范围
- **AND** SHALL 说明拆分是否只是执行顺序拆分，而不是产品边界拆分

### Requirement: Plan SHALL have a stable explicit-planning boundary

Pony Agent SHALL 将 `Plan` 定义为显式计划表达、计划更新或计划驱动执行编排的稳定默认工具。

#### Scenario: The model wants to express a plan without immediate execution

- **WHEN** `Plan` 只产出计划
- **THEN** 系统 SHALL 允许其不直接执行后续步骤

#### Scenario: Plan drives follow-up execution

- **WHEN** `Plan` 触发后续执行
- **THEN** 执行细节 SHALL 继续通过统一 `ToolPlan` 与 `child_results` 暴露
- **AND** `Plan` SHALL NOT 被视为任意批量执行入口

### Requirement: Ask SHALL remain a controlled clarification boundary

Pony Agent SHALL 将 `Ask` 定义为向用户或宿主请求澄清、确认或补充输入的稳定默认工具。

#### Scenario: The model needs clarification

- **WHEN** `Ask` 被用于请求补充信息
- **THEN** 它 SHALL 面向澄清、确认或补充输入
- **AND** SHALL NOT 被视为任意执行入口

#### Scenario: Ask interacts through host mediation

- **WHEN** `Ask` 需要把请求传递给用户或宿主
- **THEN** 它 SHALL 复用既有 permission / host mediation 合同

#### Scenario: Ask runs without host mediation

- **WHEN** `Ask` 所在环境不具备 host mediation
- **THEN** 它 SHALL fallback 到明确的受控行为
- **AND** SHALL NOT 静默丢弃请求
- **AND** SHALL NOT 在消息投递之外产生副作用

### Requirement: MCP Resource SHALL be treated as a default read-only bridge tool

Pony Agent SHALL 将 `MCP Resource` 视为默认工具面中的正式只读桥接资源工具，而不是仅保留为实现层入口。

#### Scenario: The system reads a capability-backed resource

- **WHEN** `MCP Resource` 被调用
- **THEN** 它 SHALL 面向 capability-backed resource 的只读获取
- **AND** SHALL 与普通 workspace `Read` 保持边界分离

#### Scenario: Engineers attempt to mix resource reads with generic execution

- **WHEN** 某个设计把 `MCP Resource` 混入普通工具执行或写操作
- **THEN** 系统 SHALL 拒绝这种边界混淆

### Requirement: ToolSearch SHALL remain a structured deferred-discovery tool

Pony Agent SHALL 将 `ToolSearch` 定义为 deferred / dynamic tool discovery 的结构化默认工具。

#### Scenario: The system searches for available tools

- **WHEN** `ToolSearch` 被用于发现 deferred 或 dynamic 工具
- **THEN** 它 SHALL 返回结构化候选结果
- **AND** SHALL NOT 复用普通文本搜索结果结构

#### Scenario: ToolSearch returns structured candidates

- **WHEN** `ToolSearch` 返回候选工具结果
- **THEN** 每个候选项 SHALL 至少包含：
  - `tool_name`
  - `description`
  - `source`
  - `confidence`

#### Scenario: ToolSearch is compared with Search

- **WHEN** 系统同时存在 `Search` 与 `ToolSearch`
- **THEN** `Search` SHALL 继续承担内容检索职责
- **AND** `ToolSearch` SHALL 继续承担工具发现职责
- **AND** SHALL NOT 将二者混为一谈

### Requirement: Run SHALL keep its canonical product name while tightening shell-execution semantics

Pony Agent SHALL 在第三波默认工具对齐中继续保持 `Run` 作为 canonical product tool name，并收紧其 shell-execution 语义。

#### Scenario: The system aligns Run with shell-style default execution

- **WHEN** 系统在第三波中收口 `Run`
- **THEN** 对外 canonical tool name SHALL 继续保持为 `Run`
- **AND** `RunShell` MAY 作为内部实现能力名存在
- **AND** SHALL NOT 被提升为新的默认对外工具名
- **AND** `Run` SHALL delegate to `RunShell` with contract validation

#### Scenario: Run executes a real command

- **WHEN** `Run` 执行真实命令
- **THEN** 该能力 SHALL 至少稳定暴露：
  - `cwd`
  - `timeout`
  - `exit_code`
  - `stdout`
  - `stderr`

#### Scenario: Run encounters a high-risk command

- **WHEN** `Run` 请求执行高风险命令
- **THEN** 它 SHALL 默认拒绝或进入更高审批
- **AND** SHALL NOT 被定义成任意宿主控制入口

### Requirement: The third-wave package SHALL define sequencing explicitly

Pony Agent SHALL 为第三波默认工具对齐定义稳定的收口顺序，而不是让实现者各自猜测。

#### Scenario: The system defines the closure order

- **WHEN** 系统安排第三波默认工具对齐
- **THEN** 默认收口顺序 SHALL 为：
  - `Plan / Ask`
  - `MCP Resource / ToolSearch`
  - `Run`

#### Scenario: The system explains why these tools are grouped together

- **WHEN** 用户或工程师审查第三波任务
- **THEN** 文档 SHALL 能解释：
  - 为什么 `Plan / Ask` 属于默认工具合同缺口
  - 为什么 `MCP Resource / ToolSearch` 属于默认工具桥接成员
  - 为什么 `Run` 属于默认工具合同升级成员
- **AND** SHALL 能说明 `Run` 与 `Plan / Ask` 存在执行前编排或审批联动，而不是仅因“都属于默认工具”而同波

### Requirement: The third-wave change SHALL keep larger tool surfaces out of scope

Pony Agent SHALL 在第三波默认工具对齐中显式排除更大能力面，避免范围再次膨胀。

#### Scenario: The change considers larger capability families

- **WHEN** 系统评估 Browser、Thread、Automation、Workflow、LSP 或 marketplace 等能力
- **THEN** 本 change SHALL 明确它们不属于当前第三波默认工具对齐实现范围
