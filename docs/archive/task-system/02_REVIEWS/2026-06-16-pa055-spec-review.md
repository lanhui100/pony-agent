# PA-055 Spec Review

## 审核对象

- [PA-055 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-055-align-third-wave-default-tools.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/specs/third-wave-default-tool-alignment/spec.md>)

## 审核方式

- 使用 `opencode`
- 模型：`opencode/deepseek-v4-flash-free`
- 方式：三维独立只读 spec 审核，不改文件

## 审核维度

1. 范围与分组合理性
2. 工具边界与安全约束
3. task card / proposal / design / tasks / spec 一致性

## 模型结论

- 总体结论：`需修改后通过`

## 高优先级问题

1. `Run` 的波次归入理由不足，文档没有说明它为什么必须和 `Plan / Ask / MCP Resource / ToolSearch` 同波收口。
2. `ToolSearch` 的“结构化候选结果”没有最小字段契约，导致 requirement 不可测试。
3. `Ask` 在无 host mediation 模式下的 fallback 行为未定义，存在安全与可用性缺口。

## 中优先级问题

1. `Run` 与 `RunShell` 的关系不够明确，容易出现“别名 / 包装层 / 编排层”三种不同实现理解。
2. `Ask` 的副作用边界没有被明确写死，容易与 `Plan / Run` 的职责交叉。
3. canonical spec 归档前缺少长期可读性提醒，当前 `ADDED Requirements` 风格偏 change-log 化。
4. `proposal.md` 缺少一个显式的 “Why This Grouping” 论证段落。

## 采纳与调优

本轮采纳以下修改：

1. 在 [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/proposal.md>) 中新增 `Why This Grouping`，解释这 5 个工具为何应同波收口。
2. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/design.md>) 中新增 `Run Coupling`，明确 `Run` 与 `Plan / Ask` 的编排与审批联动。
3. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/design.md>) 中补 `Ask` 的副作用边界与 host mediation 不可用时的 fallback 约束。
4. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/design.md>) 中补 `ToolSearch` 的最小结构化候选字段。
5. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/design.md>) 中明确 `Run SHALL delegate to RunShell with contract validation`。
6. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/specs/third-wave-default-tool-alignment/spec.md>) 中补 `Ask` 的 headless fallback requirement 和 side-effect constraint。
7. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/specs/third-wave-default-tool-alignment/spec.md>) 中补 `ToolSearch` 结构化候选字段 requirement。
8. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/specs/third-wave-default-tool-alignment/spec.md>) 中补 `Run` 与 `RunShell` 的委托关系 requirement，以及 `Run` 同波理由说明。
9. 在 [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-third-wave-default-tool-alignment/tasks.md>) 中补 canonical spec 归档前的长期可读性检查任务。

## 未采纳项

- 暂不在本轮 spec 中引入 `Plan` 步骤数上限或递归深度限制。
  原因：当前 change 的重点是默认工具面对齐，而不是 `ToolPlan` 复杂度治理。

## 结果

- 当前版本已从“方向正确但关键契约偏松”收紧为“可以指导实现拆解”的 spec 包。
- 下一步可以进入实现任务拆解，或继续做一轮偏工程可测性的二审。

## 审核证据

- 原始输出：
  [.tmp/pa055-opencode-review-2.txt](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa055-opencode-review-2.txt)
