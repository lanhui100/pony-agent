# first-wave-tool-surface Spec

## ADDED Requirements

### Requirement: The system SHALL define a first-wave model-visible tool surface

Pony Agent SHALL 冻结首批长期模型可见工具面，避免产品级工具名称在实现期持续漂移。

#### Scenario: The model receives the default tool surface

- **WHEN** 系统向模型暴露首批长期工具面
- **THEN** 它 SHALL 使用产品级工具名
- **AND** SHALL NOT 默认直接暴露 `workspace_*` 这类内部执行原语名

#### Scenario: The first-wave tool list is fixed

- **WHEN** 系统定义首批模型可见基础工具面
- **THEN** 该工具清单 SHALL 包括：
  - `Plan`
  - `Read`
  - `Search`
  - `List`
  - `Edit`
  - `Write`
  - `Run`
  - `Ask`

### Requirement: Legacy workspace primitives SHALL remain internal or compatibility aliases

Pony Agent SHALL 将旧 `workspace_*` 名称收口为内部原语或迁移期兼容别名，而不是继续把它们当作长期产品接口。

#### Scenario: Existing workspace primitives remain in runtime

- **WHEN** runtime 继续使用 `workspace_*` 原语执行真实能力
- **THEN** 系统 MAY 保留这些原语
- **AND** 它们 SHALL 被视为内部执行原语或兼容别名，而不是长期首批产品级工具名

#### Scenario: Legacy aliases remain bounded during migration

- **WHEN** 某个旧 `workspace_*` 名称在迁移期仍被保留
- **THEN** 系统 SHALL 明确它是兼容别名还是纯内部原语
- **AND** SHALL 定义其退出默认模型可见面的条件
- **AND** 前端主展示 SHALL 优先显示产品级工具名，而不是旧原语名

### Requirement: The first-wave surface SHALL cover the minimum base-agent loop

Pony Agent SHALL 让首批工具面覆盖基础 agent 的最小闭环能力，以支撑计划、读取、搜索、修改、执行与澄清交互。

#### Scenario: A base coding agent needs the minimum tool loop

- **WHEN** 系统定义首批基础工具面
- **THEN** 它 SHALL 至少覆盖计划、读取、搜索、列表、修改、写入、执行与提问这类最小闭环能力

#### Scenario: Each first-wave tool has a stable boundary

- **WHEN** 系统暴露首批工具面中的任一工具
- **THEN** 它 SHALL 具有清晰稳定的功能边界
- **AND** `Read` SHALL 用于读取文件或片段
- **AND** `Search` SHALL 用于搜索 workspace 内容
- **AND** `List` SHALL 用于列出文件或结构
- **AND** `Edit` SHALL 用于定向修改已有内容
- **AND** `Write` SHALL 用于新建或整体覆写
- **AND** `Run` SHALL 用于执行受控运行步骤
- **AND** `Ask` SHALL 用于澄清、确认或补充输入
- **AND** `Plan` SHALL 用于显式计划表达、计划更新或计划驱动执行编排

#### Scenario: A first-wave tool lacks a fully landed primitive

- **WHEN** 某个首批工具的底层 primitive 尚未完全落地
- **THEN** 系统 SHALL 仍冻结其产品级工具名与功能边界
- **AND** SHALL 以结构化“未落地 / 不可用 / 待迁移”语义处理其兼容状态
- **AND** SHALL NOT 因 primitive 尚未收口而临时发明新的产品级工具名

### Requirement: Composite tools SHALL expose child execution through ToolPlan-compatible structures

Pony Agent SHALL 要求复合工具通过统一 `ToolPlan` 与子步骤结果结构暴露执行细节，而不是依赖底层 primitive 语义。

#### Scenario: A composite tool expands into multiple child steps

- **WHEN** 某个模型可见工具在运行时展开多个子步骤
- **THEN** 系统 SHALL 通过统一 `ToolPlan` 与子步骤结果结构暴露这些执行细节
- **AND** SHALL NOT 要求前端或 trace 直接理解旧 `workspace_batch` 命名语义

#### Scenario: Plan remains both visible and protocol-compatible

- **WHEN** `Plan` 作为首批工具运行
- **THEN** 它 MAY 只产出计划而不执行
- **AND** 若其驱动后续执行，执行细节 SHALL 通过统一 `ToolPlan` 与 `child_results` 暴露

#### Scenario: Ask interacts through user or host mediation

- **WHEN** `Ask` 向用户或宿主请求澄清、确认或补充输入
- **THEN** 其权限与交互语义 SHALL 复用 `PA-047` 的 permission / host mediation 合同
- **AND** SHALL NOT 被当作任意泛化执行入口

### Requirement: Tool aggregation SHALL prefer product-level canonical names

Pony Agent SHALL 以产品级 canonical tool name 作为跨原语聚合的稳定主键，确保 trace、telemetry 与前端读面一致。

#### Scenario: Trace aggregates Read activity from multiple primitives

- **WHEN** 同一个产品级工具映射到多个底层原语
- **THEN** trace、telemetry 与前端聚合 SHALL 默认按产品级 canonical tool name 进行

#### Scenario: Naming layers remain explicit

- **WHEN** 某次工具调用被模型、runtime、trace 与前端共同消费
- **THEN** 系统 SHALL 能区分：
  - model-visible tool name
  - canonical product tool name
  - execution primitive name
- **AND** SHALL 默认以 canonical product tool name 作为聚合主键
