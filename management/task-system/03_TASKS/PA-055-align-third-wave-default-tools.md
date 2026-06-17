# PA-055 收口第三波默认工具对齐实现：Plan / Ask / MCP Resource / ToolSearch / Run

## 状态
- Status: `Archived`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 归档路径：
  [2026-06-17-add-third-wave-default-tool-alignment](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-17-add-third-wave-default-tool-alignment>)

## Delta Spec
- 归档路径：
  [third-wave-default-tool-alignment/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-17-add-third-wave-default-tool-alignment/specs/third-wave-default-tool-alignment/spec.md>)

## Canonical Spec
- 已同步：
  [third-wave-default-tool-alignment/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/third-wave-default-tool-alignment/spec.md>)

## Spec 状态
- Proposal: `reviewed`
- Spec: `reviewed`
- Design: `reviewed`
- Tasks: `reviewed`

## 背景
`PA-050 ~ PA-054` 已完成第二波工具面的规划与实现闭环，当前 Pony Agent 已具备：

- `Read / Search / List`
- `Edit / Write / Run`
- `Glob / WebFetch / WebSearch`
- `MCP Resource / ToolSearch`
- `Plan / Ask` 的前端默认工具展示

但从“任务卡默认工具合同”和“Codex / Claude Code 默认能力体验”双重视角看，仍存在一批没有完全收口的问题：

- `Plan` 与 `Ask` 仍偏展示层或协议层存在，缺少稳定的真实执行/中介语义
- `MCP Resource / ToolSearch` 虽已具备最小入口，但还没有被重新组织为“默认工具对齐波次”的统一实现任务
- `Run` 已具备真实命令执行 primitive，但仍需要在第三波中继续收口其与 `shell_command` 风格默认工具体验的对齐边界

因此需要把这 5 个工具作为一张新的实现任务统一收口，避免下一轮默认工具扩展再次碎片化。

## 目标
建立“第三波默认工具对齐实现”的正式任务与 spec，统一约束以下 5 个工具的实现边界、优先顺序、交互语义和与既有工具面的衔接方式：

- `Plan`
- `Ask`
- `MCP Resource`
- `ToolSearch`
- `Run`

## 输出
- `add-third-wave-default-tool-alignment` OpenSpec change
- 第三波 5 工具的正式 spec / design / tasks
- 至少一轮基于 `opencode / deepseek-v4-flash-free` 的三维只读审核记录
- 根据采纳意见调优后的 proposal / design / spec / tasks

## 范围边界
- 本卡只负责 5 个默认工具的“统一实现任务收口”和 spec 对齐
- 本卡不回退 `PA-051 ~ PA-054` 的已完成实现
- 本卡不把 Browser、Automation、Thread、Workflow、LSP 等更大能力面并入同一波
- `Run` 在本卡中只继续收口默认工具合同与 shell execution 边界，不重开一张独立权限体系重构卡
- `MCP Resource / ToolSearch` 在本卡中视为“默认工具对齐”的统一波次成员，而不是重新设计 capability bridge

## 验收标准
- spec SHALL 明确这 5 个工具为什么需要被收口为同一波次，而不是继续拆散推进
- spec SHALL 明确 `Plan / Ask` 的真实执行或宿主中介边界
- spec SHALL 明确 `MCP Resource / ToolSearch` 在默认工具面对齐中的角色，而不是只保留 deferred 口号
- spec SHALL 明确 `Run` 在第三波里是“默认工具合同升级”，不是另起产品名
- tasks SHALL 给出明确实现顺序、测试要求和非目标
- 至少完成一轮使用 `opencode / deepseek-v4-flash-free` 的独立只读 spec 审核，且覆盖至少 3 个维度

## 当前进展
- 已完成当前默认工具与 `codex-openai / claude-code-sourcemap` 的对照分析
- 已确认本轮统一收口的 5 个工具为：
  - `Plan`
  - `Ask`
  - `MCP Resource`
  - `ToolSearch`
  - `Run`
- 已完成 `PA-055` 任务卡与 `add-third-wave-default-tool-alignment` 的 proposal / design / spec / tasks 初稿
- 已使用 `opencode / deepseek-v4-flash-free` 完成一轮三维独立只读 spec 审核
- 已新增独立审核记录：
  [2026-06-16-pa055-spec-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-16-pa055-spec-review.md>)
- 已追加一轮实现阶段代码审核记录：
  [2026-06-16-pa055-code-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-16-pa055-code-review.md>)
- 已通过 `opencode export` 拿到两份并行代码审核的最终会话结论：
  - 实现向：`ses_12f7fb9a0ffeQe7fCZVTWKN1im`
  - 合同向：`ses_12f7fab08ffebGnjiUY0I3pWjU`
- 已采纳本轮高优先级意见并完成文档调优：
  - 补 `Why This Grouping`
  - 补 `Run` 与 `Plan / Ask` 的耦合理由
  - 补 `ToolSearch` 最小结构化候选字段契约
  - 补 `Ask` 在无 host mediation 场景下的 fallback 与副作用边界
  - 补 `Run -> RunShell` 的委托关系
- 已进入实现并完成一轮代码调优：
  - `Ask` 增加显式 `question` fallback，且完全缺参时恢复 error
  - `Ask` 正常 `text` 路径保持纯文本兼容输出，fallback 路径使用结构化 JSON
  - `ToolSearch` 返回结构化候选字段 `tool_name / source / confidence`
  - `Run` 保持外部 `workspace_run_command` 结果名兼容，并在 payload 中暴露 `delegateTool=run_shell`
  - 已撤回 `Plan` 在 runtime 中的硬编码权限特殊分支，避免绕过既有能力路径
  - 已完成 `pony-agent-core` 编译验证与相关目标测试验证
- 已完成 canonical spec 落盘到 `openspec/specs/third-wave-default-tool-alignment/spec.md`

## 下一步动作
1. 如需进一步收缩输出合同，可单开 cleanup 卡处理 `ToolSearch` flat / nested 字段冗余
2. 按需补一张后续维护卡，评估 `ToolSearch` 兼容层字段的长期去冗余策略
3. 本卡已完成归档，后续仅保留残余风险跟踪与回归观察

## 当前卡点
- 当前无功能性阻塞
- 验证过程中存在 Windows 增量编译目录 `os error 5` warning，但未阻止 `cargo check` 与目标测试通过
- 仅剩归档动作，没有未完成实现或验证项

## 断点续跑提示
继续前先看：

- [PA-050](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-050-build-second-wave-tool-surface.md>)
- [PA-051](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-051-implement-phase-a-tool-gaps.md>)
- [PA-054](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-054-implement-phase-d-mcp-resource-and-tool-search.md>)
- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
