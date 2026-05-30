# PA-021 建立 skills registry 与 bridge

## 状态
- Status: `Backlog`
- Priority: `P3`
- Owner: `Codex`

## 目标
在 planner、memory 与 MCP capability bridge 具备最小稳定边界后，引入 skills 体系作为可组合的能力封装层，让 skills 成为 planner 可选择、graph 可审计、runtime 可执行的高层能力单元，而不是零散 prompt 模板或宿主私有脚本。

## 输出
- skills registry 第一版模型
- skill manifest / capability fact / invocation boundary
- skills 与 tools / MCP / memory retrieval 的组合规则
- planner 消费 skills 的最小策略
- skills 可观测性与审计字段

## 验收标准
- skills 被定义为能力封装层，而不是 turn runtime 或宿主控制层
- planner 能读取 skill capability facts，但不依赖 skill 的实现细节
- skill invocation 与普通 tool / MCP invocation 的边界清楚
- skill 不直接持有宿主私有状态，而是通过统一 control plane / capability bridge 接入
- 文档明确本卡不要求完整 marketplace、分发平台或复杂沙箱

## 当前进展
- 当前项目已经有“skills”概念来源，但 Pony Agent 本体还没有正式的 skills registry
- `PA-020` 预计先把 MCP 能力接入层抽象出来
- `PA-019` 预计先建立 planner 的上层决策消费面

## 下一步动作
- 先决定 skill 在 capability registry 中的表示
- 再定义 skill invocation 的最小 lifecycle
- 为 hooks 与审计补充 skill 级生命周期事件

## 当前卡点
- 如果在 planner 和 capability bridge 未建立前直接做 skills，只会得到一层新的 prompt 包装，难以形成真正可编排能力

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-019-build-graph-planner-and-decision-policy.md`
- `management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md`
- `docs/architecture/overview.md`
