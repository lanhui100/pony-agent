# tool-permission-contract Spec

## ADDED Requirements

### Requirement: The system SHALL define unified tool permission facts

Pony Agent SHALL 为工具系统定义统一权限事实合同。

#### Scenario: A tool definition exposes approval-related metadata

- **WHEN** 某个工具定义需要声明审批或权限相关事实
- **THEN** 系统 SHALL 通过统一权限事实合同表达
- **AND** SHALL NOT 仅依赖松散自由字段

#### Scenario: Permission facts are exposed as a stable envelope

- **WHEN** 系统暴露统一 `ToolPermissionFacts` 或等价合同
- **THEN** 它 SHALL 至少稳定表达：
  - `permission_scope`
  - `permission_profile`
  - `approval_mode`
  - `host_mediated`
- **AND** 在调用进入决策阶段后 SHALL 能补充：
  - `requires_approval`
  - `decision_source`

### Requirement: Tool definition and permission decision SHALL remain separate layers

Pony Agent SHALL 把权限声明与权限决策分成独立层次，并通过共享 permission envelope 保持二者闭环而不混层。

#### Scenario: A tool declares permission requirements

- **WHEN** 某个工具在定义层声明权限事实
- **THEN** 该层 SHALL 只声明需要哪些事实
- **AND** SHALL NOT 单独决定最终放行结果

#### Scenario: Runtime or host evaluates a call

- **WHEN** 某次工具调用进入权限决策阶段
- **THEN** 系统 SHALL 能输出结构化允许、拒绝或待审批结果
- **AND** SHALL NOT 重新定义工具协议层字段

#### Scenario: Layers exchange one shared permission truth

- **WHEN** 工具定义层、权限决策层与前端读面交换权限信息
- **THEN** 它们 SHALL 复用同一 permission envelope 或等价正式结构
- **AND** ToolDefinition SHALL 提供 definition-time facts
- **AND** decision-time 结果 SHALL 只追加决策事实，而不是生成第二套不兼容字段
- **AND** 前端与 trace SHALL 读取该共享结构，而不是重新推导权限真相

### Requirement: Permission failures SHALL be normalized

Pony Agent SHALL 将权限相关失败统一归一为稳定、机器可读的结构化语义，而不是依赖自由文本解释。

#### Scenario: A tool is denied

- **WHEN** 某次工具调用被拒绝
- **THEN** 系统 SHALL 至少能表达 `permission_denied`
- **AND** SHALL 附带结构化 `decision_source`
- **AND** SHALL 附带结构化 `scope`

#### Scenario: A tool requires approval before execution

- **WHEN** 某次工具调用在放行前需要审批
- **THEN** 系统 SHALL 至少能表达 `approval_required`
- **AND** SHALL 附带结构化 `decision_source`
- **AND** SHALL 附带结构化 `scope`

#### Scenario: A tool escapes allowed workspace or scope

- **WHEN** 某次工具调用越出允许范围
- **THEN** 系统 SHALL 至少能表达 `out_of_scope`
- **AND** SHALL 指出越出的 scope 类型
- **AND** 若越界源于 workspace 边界，SHALL 与 `PA-046` 的 workspace scope 语义保持一致

#### Scenario: Failure semantics remain machine-readable

- **WHEN** 任一权限失败被返回给 runtime、trace 或 frontend
- **THEN** 系统 SHALL 提供结构化失败载荷
- **AND** SHALL NOT 仅返回自由文本加松散错误名

### Requirement: Permission facts SHALL remain conservative across composed tools

Pony Agent SHALL 要求 skill 与 composite tool 在权限聚合时采用保守上收策略，不得用较弱摘要掩盖底层更强要求。

#### Scenario: A skill composes multiple underlying capabilities

- **WHEN** 某个 skill 组合多个底层能力
- **THEN** 系统 SHALL 保守继承其中最强权限要求
- **AND** SHALL NOT 用更弱摘要覆盖底层真实差异

#### Scenario: Permission scopes are aggregated conservatively

- **WHEN** 某个 skill 或 composite tool 聚合多个子步骤权限
- **THEN** 聚合后的 `permission_scope` SHALL 至少覆盖所有子步骤 scope 的并集
- **AND** 只要任一子步骤需要审批，聚合后的 `requires_approval` SHALL 为 `true`
- **AND** 只要任一子步骤要求宿主中介，聚合后的 `host_mediated` SHALL 为 `true`

#### Scenario: A composite tool triggers multiple child steps

- **WHEN** 某个 composite tool 展开多个子步骤
- **THEN** 系统 SHALL 能给出聚合后的权限事实
- **AND** SHALL 保留子步骤权限事实的可追溯性
- **AND** 子步骤可追溯信息 SHALL 至少包含稳定子步骤标识、`permission_scope`、`requires_approval` 与 `decision_source`

### Requirement: Capability-backed tools SHALL preserve underlying permission truth

Pony Agent SHALL 要求 capability bridge 保真传递底层 capability 的真实权限与审批事实，而不是在桥接层弱化。

#### Scenario: A capability-backed tool is surfaced through runtime

- **WHEN** capability bridge 暴露一个 capability-backed tool
- **THEN** 系统 SHALL 保留底层 capability 的真实审批与 scope 信息
- **AND** SHALL NOT 在 bridge 层把更强权限压缩成更弱 summary

### Requirement: Permission facts SHALL be readable by trace and frontend surfaces

Pony Agent SHALL 让 trace、monitor 与前端读面能够直接消费统一权限事实与决策结果，而不是再次派生第二份权限真相。

#### Scenario: A tool call is shown in trace or frontend

- **WHEN** trace、monitor、前端或 session drilldown 展示某次工具活动
- **THEN** 它们 SHALL 能读取稳定权限字段
- **AND** SHALL NOT 依赖额外推导生成第二份权限真相

#### Scenario: Frontend reads both facts and outcomes

- **WHEN** 前端或 trace 展示一次权限相关工具结果
- **THEN** 它 SHALL 能同时读取 definition-time permission facts 与 decision-time outcome
- **AND** SHALL 能区分 `approval_required`、`permission_denied` 与 `out_of_scope`
