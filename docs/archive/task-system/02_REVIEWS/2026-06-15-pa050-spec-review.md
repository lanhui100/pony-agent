# PA-050 Spec Review

## 审核对象

- [PA-050 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-050-build-second-wave-tool-surface.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)

## 审核方式

- 使用 `opencode`
- 模型：`deepseek/deepseek-v4-flash`
- 方式：两轮独立只读 spec 审核，不改文件

## 第一轮审核结论

- 总体结论：`需收紧后可通过`

### 高优先级问题

1. `RunShell / Run` 缺少安全沙箱边界与高风险命令约束。
2. `Edit` 缺少零匹配、多匹配和非法 patch 的失败语义。
3. `Grep` 与 `Search` 的产品层/实现层关系没有正式闭环。

### 建议采纳修改

1. 在 spec 中为 `Run` 增加高风险命令默认拒绝或更高等级审批要求。
2. 在 spec/design 中补 `Edit` 的匹配失败与歧义失败规则。
3. 在 spec/design 中明确产品层继续保留 `Search`，`Grep` 作为实现层 primitive。
4. 在 spec 中补 `Glob` 与 `List` 的职责分界。
5. 在 tasks 中把 `ToolSearch` 明确为 deferred 范围，而不是与其他候选能力并列推进。

## 第二轮审核结论

- 总体结论：`方向正确，但还需收紧执行顺序与 scope control`

### 高优先级问题

1. `tasks.md` 只有平铺 checklist，缺少 Phase 依赖标记，允许被并行解读。
2. `spec.md` 没有显式引用分阶段模型，优先级锚点主要留在 `design.md`。
3. `RunShell` 与产品层 `Run` 的关系还需要更明确的命名约束。

### 建议采纳修改

1. 在 `tasks.md` 增加 `Phase A -> B -> C -> D` 执行依赖说明。
2. 在 `spec.md` 的 requirement 级别补 `Phase` 标签。
3. 在任务卡范围边界中明确第二批候选能力只产出边界与进入条件，不产出代码或原型。
4. 在 `design.md` 中明确产品层 canonical name 继续保持 `Run`，`RunShell` 仅作为内部实现层名称。

## 已采纳修改

1. 在 `design.md` 与 `spec.md` 中补入 `Edit` 的零匹配、多匹配与非法输入失败语义。
2. 在 `design.md` 与 `spec.md` 中补入 `Run` 的高风险命令边界，以及 `RunShell` 仅作为内部实现能力名的约束。
3. 在 `design.md` 与 `spec.md` 中补入 `Search` 对外稳定、`Grep` 作为实现层 primitive 的关系说明。
4. 在 `spec.md` 中补入 `Glob` 与 `List` 的职责分界 requirement。
5. 在 `tasks.md` 中新增阶段依赖说明，并将 `ToolSearch` 改为 deferred 范围。
6. 在任务卡范围边界中明确第二批候选能力只产出边界说明和进入条件，不产出代码、原型或实现验证。

## 未采纳项

- 暂无。本轮两次审核提出的高优先级意见均已采纳到文档。

## 审核结论

本轮收紧后，`PA-050` 已从“工具候选清单”提升为“具备阶段顺序、scope control 和安全边界约束的正式第二批工具规划 spec”。当前这套文档已经可以作为后续拆实现卡的基线，下一步应在不扩 scope 的前提下，将 `Phase A` 的 `Edit / Write / Run` 缺口进一步拆成可实现任务。
