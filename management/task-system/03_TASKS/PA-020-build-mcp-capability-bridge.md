# PA-020 建立 MCP capability bridge

## 状态
- Status: `Backlog`
- Priority: `P3`
- Owner: `Codex`

## 目标
在 core runtime、graph run、host control plane 与 memory/planner 边界稳定之后，把 MCP 作为能力接入层正式引入，让外部资源、工具与上下文可以通过统一 bridge 接进 Pony Agent，而不是直接侵入 runtime / graph 内部。

## 输出
- MCP bridge 第一版抽象
- MCP resource / tool / prompt-like capability 映射规则
- MCP 与内建 tools / skills / memory retrieval 的边界说明
- capability registry 中的 MCP 条目模型
- MCP 错误、权限与可观测性约束

## 验收标准
- MCP 被定义为能力接入层，而不是 graph 或 runtime 的调度层
- 内建 tools 与 MCP tools 可以被统一 capability registry 暴露
- planner / graph 只消费抽象 capability facts，不直接耦合 MCP 协议细节
- 宿主层不需要自己理解 MCP，只走统一 control plane / capability query
- 文档明确本卡不要求一次性做完所有外部协议或 marketplace 生态

## 当前进展
- 当前 codebase 已有本地 tools 抽象与 session/runtime 基础
- 还没有正式 MCP bridge、capability registry 扩展与权限治理
- `PA-015 / PA-019` 预计先稳定 control plane 与 planner 消费面

## 下一步动作
- 先定义 MCP 在 Pony Agent 中处于“能力接入层”的位置
- 再把 MCP resources / tools 映射到统一 capability registry
- 最后决定 planner / skills 如何消费 MCP 能力事实

## 当前卡点
- 如果在 planner、memory、control plane 未稳定前直接接 MCP，很容易让协议细节向上污染 graph、宿主和 UI

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-004-define-provider-and-tool-abstractions.md`
- `management/task-system/03_TASKS/PA-015-extract-host-control-plane.md`
- `management/task-system/03_TASKS/PA-019-build-graph-planner-and-decision-policy.md`
- `docs/architecture/overview.md`
