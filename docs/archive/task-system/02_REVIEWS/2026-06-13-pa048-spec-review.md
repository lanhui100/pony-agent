# PA-048 Spec Review

## 审核对象

- [PA-048 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-048-first-wave-tool-surface-and-legacy-mapping.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/first-wave-tool-surface-and-legacy-mapping/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/first-wave-tool-surface-and-legacy-mapping/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/first-wave-tool-surface-and-legacy-mapping/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/first-wave-tool-surface-and-legacy-mapping/specs/first-wave-tool-surface/spec.md>)

## 审核方式

- 独立智能体审查
- 审核智能体：`Kierkegaard`
- 审核结论：`不通过，需补齐首批工具面、命名层次与兼容合同`

## 主要发现

1. 首批 8 个产品级工具名原本停留在 design 清单，没有进入 spec 成为正式工具合同。
2. 模型可见名、canonical 产品名与 execution primitive 三层命名关系原本没有正式闭环。
3. `Edit / Write / Run / Ask` 等工具原本缺少合同级边界或兼容状态说明。
4. 旧 `workspace_*` 到新工具面的兼容策略原本只有原则，没有生命周期、退出条件与前端展示规则。
5. `Plan` 与 `Ask` 原本缺少与 `PA-045 / PA-047` 结果合同、权限合同的闭环说明。
6. `tasks.md` 与任务卡没有反映 artifacts 已形成且已进入独立 review 的真实进度。

## 已采纳修改

1. 在 spec 中正式冻结首批工具清单：
   - `Plan`
   - `Read`
   - `Search`
   - `List`
   - `Edit`
   - `Write`
   - `Run`
   - `Ask`
2. 在 design/spec 中补入 `model_visible_name / canonical_tool_name / execution_primitive` 三层命名模型。
3. 在 design/spec 中补入 8 个首批工具的稳定边界，以及 primitive 未完全落地时的结构化兼容状态。
4. 在 design/spec 中补入旧别名的迁移展示、退出条件与兼容层生命周期规则。
5. 在 design/spec 中补入 `Plan` 与 `Ask` 的专项合同，并明确复用 `PA-047` 的权限与 host mediation 语义。
6. 同步 `tasks.md` 与任务卡进度表达。

## 审核结论

本轮收紧后，`PA-048` 已从“工具名清单草案”提升为“首批产品级工具面与旧原语迁移策略的正式合同”。当前剩余动作主要是：

1. 运行 OpenSpec change 校验
2. 将冻结后的工具面与命名模型继续输入 `PA-049` 的前端展示合同
