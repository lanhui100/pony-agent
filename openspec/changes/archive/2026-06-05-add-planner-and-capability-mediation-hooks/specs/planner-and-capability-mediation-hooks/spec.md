## ADDED Requirements

### Requirement: Planner hooks SHALL consume normalized planner facts only
planner hooks SHALL 只消费规范化 planner facts，而不是 provider raw protocol 或 UI 私有状态。

#### Scenario: A hook observes graph decision formation
- **WHEN** graph planner 形成 `continue / wait_user` 决策
- **THEN** hook SHALL 读取 normalized planner facts
- **AND** hook SHALL NOT 依赖 provider 原始流响应

#### Scenario: A planner hook transforms allowed fields only
- **WHEN** planner hook 返回 transform 结果
- **THEN** 该结果 SHALL 只允许修改预先声明的 planner envelope 白名单字段
- **AND** 只读字段与决策真相源字段 SHALL 保持不可改写

### Requirement: Capability hooks SHALL mediate through the existing registry boundary
capability hooks SHALL 建立在 capability bridge / skill ingress 的中介面上，不得绕开 registry。

#### Scenario: A hook guards skill ingress
- **WHEN** 系统接收 skill source snapshot 或 capability mediation 请求
- **THEN** hook SHALL 读取 normalized mediation envelope
- **AND** hook SHALL NOT 直接创建新的 registry truth-source

#### Scenario: A capability hook transforms allowed mediation fields only
- **WHEN** capability hook 返回 transform 结果
- **THEN** 该结果 SHALL 只允许修改预先声明的 mediation envelope 白名单字段
- **AND** capability identity、source truth 与 registry ownership SHALL 保持只读

### Requirement: Planner and capability hooks SHALL remain controlled extensions
planner / capability hooks SHALL 不得演化成第二 scheduler 或第二 capability registry。

#### Scenario: A hook transforms a planner suggestion
- **WHEN** hook 返回 transform 结果
- **THEN** 该结果 SHALL 只作用于规范化 planner/capability envelope
- **AND** hook SHALL NOT 直接调度新的 turn、model hop 或 registry mutation

### Requirement: Planner and capability hook evidence SHALL enter read-plane surfaces
planner / capability hook evidence SHALL 可进入 trace、monitor 与 control-plane drilldown。

#### Scenario: A user inspects a mediated planner/capability path
- **WHEN** 某次 planner 或 capability mediation 路径触发 hooks
- **THEN** 系统 SHALL 能展示 hook 的 boundary、结果、耗时与是否阻断

#### Scenario: Planner evidence is persisted through the turn truth-source
- **WHEN** planner preflight、tool selection 或 graph decision 形成了规范化 evidence
- **THEN** 系统 SHALL 把该 evidence 写入 `hook_trace_records -> turn trace -> session snapshot`
- **AND** control-plane drilldown SHALL 能读回这些 planner boundary

#### Scenario: Capability evidence is readable without declaring monitor closeout
- **WHEN** capability resolve 或 skill mediation 路径触发 hooks
- **THEN** 系统 SHALL 至少保证 control-plane / session snapshot 能读回对应 evidence
- **AND** 若 monitor 投影尚未完整落地，系统 SHALL NOT 把 monitor closeout 误报为本 change 已完成

#### Scenario: Capability ingress is readable from source drilldown without becoming turn trace noise
- **WHEN** control-plane 接收 mcp source snapshot 或 skill source snapshot
- **THEN** 系统 SHALL 在对应 source drilldown / runtime registry 中记录最近一次 ingress observation
- **AND** 该 ingress observation SHALL 保持附着在 source truth 上，而不是伪装成新的 turn-level hook trace

#### Scenario: Capability ingress survives file-backed reload through the existing runtime boundary
- **WHEN** mcp source snapshot 或 skill source snapshot 已写入既有 file-backed store，随后 runtime / control-plane 被重建
- **THEN** 系统 SHALL 能在重建后的 source drilldown 中继续读回对应 ingress observation
- **AND** 该恢复过程 SHALL 通过既有 runtime / registry 边界回填，而不是额外引入第二份 capability truth-source

#### Scenario: Source-level ingress can be inspected in monitor without being misreported as session trace aggregation
- **WHEN** monitor 页面展示 capability source 调试读面
- **THEN** 系统 SHALL 能展示 source-level ingress observation 的 summary / boundary / candidate ids
- **AND** 该展示 SHALL 保持属于 source inspect，而不是被误计入 session-level canonical trace aggregation

#### Scenario: Canonical monitor summary ignores source ingress without trace activity
- **WHEN** source ingress 已进入 capability / skill source drilldown，但对应会话 trace 中没有 capability activity
- **THEN** `load_model_monitor_summary()` SHALL NOT 因为 source ingress 存在而新增 capability / skill canonical 聚合计数
- **AND** source ingress 仍 SHALL 可通过 source inspect 读回
