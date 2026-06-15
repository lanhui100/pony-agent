# tool-observability-contract Spec

## ADDED Requirements

### Requirement: Tool activity surfaces SHALL share a stable display field set

Pony Agent SHALL 为前端、trace、monitor 与 session drilldown 提供统一共享的工具展示字段集合，避免展示协议分叉。

#### Scenario: A tool call appears in frontend and trace

- **WHEN** 前端工具活动、trace、monitor 或 session drilldown 展示某次工具调用
- **THEN** 它们 SHALL 能读取一组稳定共享的展示字段
- **AND** SHALL NOT 各自发明第二套工具展示协议

#### Scenario: The shared display field set is explicit

- **WHEN** 系统暴露统一工具展示合同
- **THEN** 稳定共享字段 SHALL 至少包括：
  - `name`
  - `canonical_tool_name`
  - `display_name_zh`
  - `kind`
  - `status`
  - `summary`
  - `duration_ms`
  - `error`
  - `artifacts`
  - `child_results`

### Requirement: Frontend SHALL prefer localized display metadata

Pony Agent SHALL 将 `display_name_zh` 视为正式展示元数据，并在用户可见主标签中优先消费该字段。

#### Scenario: A product-level tool is shown to the user

- **WHEN** 前端展示某个产品级工具
- **THEN** 它 SHALL 优先使用 `display_name_zh` 作为主展示标签
- **AND** 在缺失时 SHALL 回退到稳定英文工具名

#### Scenario: Canonical names remain available for debugging and aggregation

- **WHEN** 前端或 trace 展示某个工具活动
- **THEN** 它们 SHALL 能读取 `canonical_tool_name`
- **AND** SHALL 以该字段作为聚合或调试锚点

### Requirement: Composite tools SHALL expose nested execution in a stable way

Pony Agent SHALL 让复合工具的父结果与子步骤结果都能通过稳定字段结构被展示和追踪。

#### Scenario: A composite tool has child results

- **WHEN** 某个复合工具带有多个子步骤结果
- **THEN** 前端与观测读面 SHALL 通过统一子结果字段展示
- **AND** SHALL NOT 依赖底层 primitive 名拼接自己的嵌套语义

#### Scenario: Parent and child statuses are both representable

- **WHEN** 某个复合工具主结果与子结果状态不一致
- **THEN** 系统 SHALL 能同时表达父结果状态与子步骤状态
- **AND** 若整体为部分成功，前端 SHALL 能稳定区分成功、失败、待审批或被拒绝的子步骤

### Requirement: Permission and failure states SHALL have stable presentation semantics

Pony Agent SHALL 为成功、部分成功、失败、被拒绝与待审批提供统一展示语义，不允许各读面各自解释状态。

#### Scenario: A tool call is denied or requires approval

- **WHEN** 某次工具调用被拒绝或待审批
- **THEN** 前端与观测读面 SHALL 能基于统一字段展示该状态

#### Scenario: Display statuses are explicit

- **WHEN** 前端或观测读面展示工具状态
- **THEN** 系统 SHALL 稳定区分：
  - 成功
  - 部分成功
  - 失败
  - 被拒绝
  - 待审批
- **AND** SHALL NOT 将 `permission_denied` 或 `approval_required` 吞并为普通失败

#### Scenario: A tool call partially succeeds

- **WHEN** 某个工具或复合工具部分成功
- **THEN** 前端与观测读面 SHALL 能稳定区分部分成功与完全失败

### Requirement: Artifact presentation SHALL remain distinct from child execution results

Pony Agent SHALL 将 `artifacts` 与 `child_results` 视为两类不同容器，分别承载附件对象与子步骤执行结果。

#### Scenario: A tool returns both artifacts and child results

- **WHEN** 某个工具同时返回附件对象与子步骤结果
- **THEN** 前端与观测读面 SHALL 区分这两类数据
- **AND** SHALL NOT 混为同一展示容器语义

#### Scenario: Artifact and child-result containers remain stable

- **WHEN** 系统返回 `artifacts` 或 `child_results`
- **THEN** `artifacts` SHALL 被视为附件/引用对象容器
- **AND** `child_results` SHALL 被视为子步骤执行结果容器
- **AND** 前端 SHALL NOT 将两者混用为同一列表语义

### Requirement: Migration compatibility SHALL preserve display stability

Pony Agent SHALL 在新旧工具迁移期保持展示层稳定，并为缺字段旧记录定义统一回退口径。

#### Scenario: A legacy primitive is still present during migration

- **WHEN** 某次工具活动仍由旧 primitive 或兼容别名触发
- **THEN** 前端主展示 SHALL 优先使用产品级工具名与 `display_name_zh`
- **AND** 旧 primitive 名 MAY 仅作为调试辅助信息存在

#### Scenario: A legacy activity lacks new display fields

- **WHEN** 某次旧工具活动暂时缺失部分新展示字段
- **THEN** 系统 SHALL 定义稳定回退口径
- **AND** SHALL NOT 允许各读面各自发明不同兼容逻辑
