# PA-045 Spec Review

## 审核对象

- [PA-045 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-045-tool-system-contract-and-exposure-boundary.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-system-contract-and-exposure-boundary/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-system-contract-and-exposure-boundary/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-system-contract-and-exposure-boundary/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/tool-system-contract-and-exposure-boundary/specs/tool-system-contract/spec.md>)

## 审核方式

- 独立智能体审查
- 审核结论：`通过，但需收紧关键合同点`

## 主要发现

1. `name / canonical_tool_name / execution primitive` 三者关系原本不够清晰，容易导致聚合主键漂移。
2. `deferred` 暴露策略原本只有概念，没有最小生命周期规则。
3. `ToolResult` 原本缺少对子步骤结果与附件对象的显式分层，`artifacts` 有万能袋风险。
4. `ToolResult.error` 原本未定义最小结构字段，后续审批/重试/前端提示会缺乏稳定依托。
5. 权限字段若直接冻结在母合同中，会过早把 `PA-047` 还未收口的语义固化。
6. 模型可见工具命名规则与最小映射示例原本不够明确。
7. `tasks.md` 与任务卡的进度表达需要同步。

## 已采纳修改

1. 在 proposal/design/spec 中补入三层标识模型：
   - `name`
   - `canonical_tool_name`
   - `execution_primitive`
2. 在 design/spec 中为 `deferred` 补最小生命周期规则，并明确默认只对当前 turn 生效。
3. 在 design/spec 中把 `ToolResult` 收口为：
   - `data`
   - `child_results`
   - `artifacts`
4. 在 design/spec 中为 `ToolResult.error` 补入最小结构字段：
   - `kind`
   - `message`
   - `details?`
   - `retryable?`
   - `source?`
5. 将审批/权限相关字段调整为 `policy_metadata` 占位，并明确正式语义由 `PA-047` 承接。
6. 补模型可见工具命名约束与 `Read / Search / List` 到 `workspace_*` 原语的最小映射示例。
7. 同步 `tasks.md` 与任务卡进度。

## 审核结论

本轮 spec 已从“方向正确但合同偏语义化”收紧到“可作为后续 `PA-046 ~ PA-049` 母合同继续推进”的状态。当前剩余动作主要是：

1. 运行 OpenSpec change 校验
2. 如校验通过，将本卡提升为可交付的 spec 阶段状态
3. 进入 `PA-046`
