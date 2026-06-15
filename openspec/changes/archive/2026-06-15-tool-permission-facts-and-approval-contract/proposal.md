# Proposal: Tool Permission Facts And Approval Contract

## Why

当前 Pony Agent 已经在多个位置暴露出工具权限相关事实：

- capability bridge 中已有 `requires_approval / permission_scope / permission_profile / host_mediated`
- runtime 与 telemetry 已能携带部分权限与失败信息
- 工具系统也已经确认后续必须支持 `permission_denied / approval_required / out_of_scope`

但这些事实还没有收口为统一合同，因此存在几个风险：

- builtin / capability / skill / composite tool 可能继续各自携带不同审批语义
- 工具定义层、权限决策层与前端读面之间还没有正式边界
- `PA-045` 中预留的 `policy_metadata` 若不尽快收口，就会重新变成松散字段袋
- trace、monitor、前端与后续 host approval 流程缺少统一权限真相源

因此需要把工具权限事实、审批语义与失败归一化正式写成一张独立 spec，而不是继续让它散落在实现和后续任务口头约定里。

## What Changes

- 建立 `tool permission facts and approval contract` 的正式 spec
- 定义统一 `ToolPermissionFacts` 或等价合同，并收口最小稳定字段
- 明确工具定义层与权限决策层的边界
- 定义 `permission_denied / approval_required / out_of_scope` 等失败语义与最小结构
- 规定 builtin / capability / skill / composite tool 必须复用同一套权限口径
- 定义 definition-time / decision-time / frontend-read 的交换面，避免三层各自推导权限真相

## Impact

- 后续审批 UI、host mediation、trace/monitor 展示都能建立在统一权限合同上
- `policy_metadata` 不再只是占位字段，而是进入正式语义模型
- 复合工具和 skill 不会再发明第二套弱化或隐藏底层权限差异的表达
- `out_of_scope` 与 workspace scope 的关系可以和 `PA-046` 正式闭环，而不是继续停留在实现约定

## Tracking

- Task card: `PA-047`
- OpenSpec Change: `tool-permission-facts-and-approval-contract`
