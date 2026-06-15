# PA-050 补齐第二批基础工具能力并形成实现顺序

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 归档路径：
  [2026-06-15-add-second-wave-tool-surface](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-15-add-second-wave-tool-surface>)

## Delta Spec
- 已归档于：
  [second-wave-tool-surface/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-15-add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)

## Canonical Spec
- 已同步到：
  `openspec/specs/second-wave-tool-surface/spec.md`

## Spec 状态
- Proposal: `validated`
- Spec: `validated`
- Design: `validated`
- Tasks: `validated`

## 背景
`PA-045 ~ PA-049` 已完成工具协议、workspace 合同、权限合同、首批工具面与前端展示合同的第一轮闭环，Pony Agent 当前已经正式冻结首批产品级工具面：

- `Plan`
- `Read`
- `Search`
- `List`
- `Edit`
- `Write`
- `Run`
- `Ask`

但当前代码真实落地的内建工具仍主要集中在：

- `Run`
- `Ask`
- `Read`
- `List`
- `Search`
- `Plan`

也就是说，首批合同中的 `Edit / Write` 还没有对应的真实内建 primitive；同时，和当前项目内对照用的 Codex / Claude Code 基线相比，Pony Agent 仍缺少更完整的执行、检索、外部读取、MCP 资源读取与工具发现能力。

## 目标
建立第二批基础工具能力的正式 spec 与实现顺序，明确 Pony Agent 下一步应该优先补哪些工具、每类工具的边界是什么、与现有首批工具面如何衔接，以及哪些能力属于后续阶段而不是当前 v1。

## 输出
- `add-second-wave-tool-surface` OpenSpec change
- 第二批工具能力的正式 spec
- 第二批工具实现优先级与分阶段范围
- 当前首批工具合同与真实实现缺口的对齐说明
- 至少一轮独立 spec review 与采纳修订记录

## 范围边界
- 本卡聚焦“下一步应该实现哪些工具”与“这些工具的稳定边界”，不直接完成所有工具代码实现
- 本卡不回灌已归档的 `PA-045 ~ PA-049`
- 本卡不要求一次把 Browser、Workflow、Thread 管理、Automation 等所有扩展能力纳入内建工具面
- 本卡不重做现有工具协议、权限合同或展示合同，只在其基础上扩展第二批实现范围
- `Glob / Grep / WebFetch / WebSearch / MCP Resource / ToolSearch` 在本卡中只产出进入条件、边界说明与阶段顺序，不产出代码、原型或实现验证

## 验收标准
- spec SHALL 明确区分“合同里已冻结但尚未真实实现的首批工具缺口”和“新增第二批工具能力”
- spec SHALL 给出至少一组稳定的第二批候选工具清单与优先级
- spec SHALL 明确 `Edit / Write / RunShell` 的 v1 边界
- spec SHALL 明确 `Glob / Grep / WebFetch / WebSearch / MCP Resource / ToolSearch` 中哪些进入近线范围
- design/tasks SHALL 明确哪些工具先做、哪些后做、哪些不在当前范围
- 至少完成一轮使用 `opencode / deepseek-v4-flash` 的独立只读 spec 审核，并根据采纳意见调优文档

## 当前进展
- 已完成现状调研、proposal / design / spec / tasks 初稿与 OpenSpec strict validate
- 已完成两轮 `opencode / deepseek-v4-flash` 独立只读 spec 审核，并已采纳高优先级意见回写到文档
- 已新增独立审核记录：
  [2026-06-15-pa050-spec-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-15-pa050-spec-review.md>)
- 已完成 `PA-051 ~ PA-054` 全部实现与收口，当前真实产品级 builtin 工具面已包含：
  - `Run`
  - `Ask`
  - `Read`
  - `List`
  - `Search`
  - `Glob`
  - `WebFetch`
  - `WebSearch`
  - `MCPResource`
  - `ToolSearch`
  - `Write`
  - `Edit`
  - `Plan`
- 已完成 canonical spec 同步与 OpenSpec 归档收口

## 下一步动作
1. 后续如继续扩工具面，优先基于 `PA-050` 已冻结的第二波产品边界继续拆新卡
2. 若进入第三波工具能力，优先补 `workspace_read_file` / `workspace_read_file_segment` / `mcp_resource_list` 等更强读面与发现能力

## 当前卡点
- 暂无。当前卡已完成态收口。

## 断点续跑提示
继续前先看：

- [01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [2026-06-15-pa050-spec-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-15-pa050-spec-review.md>)
- [PA-048](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-048-first-wave-tool-surface-and-legacy-mapping.md>)
- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [first-wave-tool-surface spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/first-wave-tool-surface/spec.md>)
- [codex-hermes-claude-code-对比说明.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/codex-hermes-claude-code-对比说明.md)
