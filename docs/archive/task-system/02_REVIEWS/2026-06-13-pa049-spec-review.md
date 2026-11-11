# PA-049 Spec Review

## 审核对象

- [PA-049 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-049-tool-observability-and-frontend-presentation-contract.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-observability-and-frontend-presentation-contract/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-observability-and-frontend-presentation-contract/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-observability-and-frontend-presentation-contract/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-observability-and-frontend-presentation-contract/specs/tool-observability-contract/spec.md>)

## 审核方式

- 独立智能体审查
- 审核智能体：`Mendel`
- 审核结论：`不通过，需补齐展示字段、状态语义与迁移兼容合同`

## 主要发现

1. 统一展示字段集合原本只在 design 中出现，没有进入 spec 成为正式共享字段合同。
2. `display_name_zh`、`canonical_tool_name`、`child_results`、`artifacts` 与权限失败态原本没有以字段级正式要求进入 spec。
3. 复合工具、部分成功、待审批与被拒绝原本只有概念性描述，没有闭合到可验收的状态语义。
4. 新旧工具迁移兼容原本缺少正式 requirement，验收矩阵也没有规范锚点。
5. `tasks.md` 与任务卡状态没有反映 artifacts 已形成并进入独立 review 的真实进度。

## 已采纳修改

1. 在 spec 中正式写入共享展示字段集合：
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
2. 将 `display_name_zh` 收紧为主展示标签的正式字段，并明确英文回退规则。
3. 在 design/spec 中补入父结果与子步骤状态并存、部分成功与权限失败态不可被普通失败吞并的展示语义。
4. 在 design/spec 中补入 `artifacts` 与 `child_results` 的稳定容器语义。
5. 在 spec 中新增迁移兼容 requirement，明确旧 primitive 调用时的主展示优先级与缺字段回退口径。
6. 同步 `tasks.md` 与任务卡状态。

## 审核结论

本轮收紧后，`PA-049` 已从“展示方向说明”提升为“前端、trace、monitor 与迁移兼容可共同消费的正式展示合同”。当前剩余动作主要是：

1. 运行 OpenSpec change 校验
2. 将展示字段与状态语义继续落到前端实现与验收矩阵中
