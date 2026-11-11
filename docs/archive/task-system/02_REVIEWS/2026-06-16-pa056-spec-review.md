# PA-056 Spec Review

## 审核对象

- [PA-056 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-056-redesign-context-assembly-and-cache-strategy.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/specs/context-assembly-and-cache-strategy/spec.md>)

## 审核方式

- 使用 `opencode`
- 模型：`opencode/deepseek-v4-flash-free`
- 方式：三维独立只读 spec 审核，不改文件

## 审核维度

1. 分层合理性与任务拆分
2. 缓存边界与前缀稳定性
3. `system prompt / AGENT.md / workspace / memory hooks` 一致性

## 模型结论

- 总体结论：`Conditionally Pass`

## 高优先级问题

1. 缺少从当前 `TurnContextBuilder` 扁平组装迁移到目标分层结构的实现路径。
2. `Runtime Facts` 的稳定性边界不完整，尤其是 workspace roots 变化是否触发 cache reset 未明确。
3. `AGENT.md` 的深层覆盖规则没有明确是“文件级替换”还是“声明级合并”。
4. provider continuation 失败时的回退策略未定义。
5. `session summary / truncation note / planner skills summary` 等动态说明项没有明确分配到具体层。

## 中优先级问题

1. `MemoryProvider / MemorySelectionPolicy / MemoryInjectionMode` 只有名字，没有签名示意。
2. 单轮用户临时指令缺少明确层归属。
3. `context_refresh_reason / instruction_scope_sources / conversation_carry_mode` 的观测要求没有任务覆盖。
4. `low-frequency cache reset` 缺少定量目标。
5. `Memory Injection` 排在 `Conversation Carry` 前面但缺少排序理由。

## 采纳与调优

本轮采纳以下修改：

1. 在 [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/proposal.md>) 中新增 `Why This Grouping`。
2. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/design.md>) 中新增 `Migration Path`。
3. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/design.md>) 中补 `Memory Injection` 与 `Conversation Carry` 的排序理由。
4. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/design.md>) 中补 provider continuation 失败后的默认回退策略。
5. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/design.md>) 中补动态说明项的显式层分配。
6. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/design.md>) 中补 `MemoryProvider` 的示意 trait 签名。
7. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/specs/context-assembly-and-cache-strategy/spec.md>) 中明确 `AGENT.md` 采用文件级覆盖优先。
8. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/specs/context-assembly-and-cache-strategy/spec.md>) 中补 continuation 失败回退 requirement。
9. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/specs/context-assembly-and-cache-strategy/spec.md>) 中补 `low-frequency` 的定量目标。
10. 在 [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/redesign-context-assembly-and-cache-strategy/tasks.md>) 中补观测任务与实现桥接任务。

## 未采纳项

- 暂不要求所有 provider 在本轮立即切到 continuation-first 实现。
  原因：本轮目标是统一上下文构建架构和缓存边界，不是一次性完成所有 transport 迁移。

## 结果

- 当前版本已从“分层方向正确但实现桥接偏弱”收紧为“可以指导后续实现拆解”的 spec 包。
- 下一步可以进入实现任务拆解，或继续做一轮偏实现可测性的二审。

## 审核证据

- 原始输出：
  [.tmp/pa056-opencode-spec-review.txt](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa056-opencode-spec-review.txt)
