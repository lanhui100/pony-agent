# PA-047 Spec Review

## 审核对象

- [PA-047 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-047-tool-permission-facts-and-approval-contract.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-permission-facts-and-approval-contract/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-permission-facts-and-approval-contract/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-permission-facts-and-approval-contract/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-permission-facts-and-approval-contract/specs/tool-permission-contract/spec.md>)

## 审核方式

- 独立智能体审查
- 审核智能体：`Mendel`
- 审核结论：`不通过，需补齐权限合同的结构化闭环`

## 主要发现

1. `ToolPermissionFacts` 在 proposal/design 中被视为核心合同，但 spec 没有把最小字段与稳定语义写成正式要求。
2. `approval_required / permission_denied / out_of_scope` 原本只被收口为失败名称，没有最小结构、决策来源与 scope 说明。
3. builtin / capability / skill / composite 的权限聚合规则原本缺乏正式口径，尤其缺少子步骤权限可追溯的最小读面。
4. 工具定义层、权限决策层与前端读面的边界原本只有“不要做什么”，没有“必须交换什么”的共享 envelope。
5. `out_of_scope` 与 `PA-046` 的 workspace scope 语义衔接不够正式。
6. `tasks.md` 与任务卡状态没有反映 artifacts 已起草、已进入 review 的真实进度。

## 已采纳修改

1. 在 design/spec 中补入统一 permission envelope，并区分 definition-time / decision-time / frontend-read 三层交换面。
2. 在 design/spec 中收口 `ToolPermissionFacts` 最小稳定字段：
   - `permission_scope`
   - `permission_profile`
   - `approval_mode`
   - `host_mediated`
   - `requires_approval`
   - `decision_source`
3. 在 spec 中把 `permission_denied / approval_required / out_of_scope` 收紧为结构化失败语义，至少要求带 `decision_source` 与 `scope`。
4. 在 design/spec 中补入 skill/composite 的保守聚合规则，明确 scope 并集、审批与 host mediation 的向上冒泡，以及子步骤最小可追溯字段。
5. 在 design/spec 中补入 `out_of_scope` 与 `PA-046` workspace scope 的复用关系。
6. 同步 `tasks.md` 与任务卡状态，避免任务系统与 OpenSpec 实际进度失真。

## 审核结论

本轮收紧后，`PA-047` 已从“方向正确但结构偏松”提升为“可供工具面、前端读面与审批流程复用的正式权限合同”。当前剩余动作主要是：

1. 运行 OpenSpec change 校验
2. 在 `PA-048` 与 `PA-049` 中复用统一 permission envelope 与结构化失败语义
