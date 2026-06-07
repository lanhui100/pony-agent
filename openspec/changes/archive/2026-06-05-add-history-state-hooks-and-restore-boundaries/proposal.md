## Why

`history checkout / restore / fork / switch branch` 已经是 Pony Agent 当前 session 可控能力里的稳定真边界，也已经具备明确的 degrade / rollback 合同与前端反馈面。但这条路径尚未纳入 hooks 体系，意味着：

- 无法在 history-state 切换前后挂统一 guard / observe / patch 合同
- 历史恢复相关的控制策略容易退化为 UI 私有逻辑或 `SessionStore` 内部特判
- reload 后缺少统一的 history-state audit evidence，难以把“恢复了什么、为何 degraded、是否允许切换”持续读回

本次 change 的关键约束是：history-state hooks evidence 只能成为审计证据载体，不能演化成新的 restore / submission / cursor 仲裁真相源。

在 `PA-033 / PA-035 / PA-038 / PA-039 / PA-040` 已经把 turn/run/memory/planner/capability 边界都接入 hooks 之后，history-state control 是当前最自然且尚未覆盖的稳定 lifecycle boundary。

## What Changes

- 为 `history checkout / branch restore / branch fork / branch switch` 定义 history-state hook point
- 定义 normalized history-control envelope，包括：
  - 请求类型、node/branch 标识
  - transcript/workspace 恢复模式
  - rollback capability / degrade contract 摘要
  - history cursor / resolved node 的最小只读事实
- 定义 history-state evidence 的 persistence / reload / control-plane read-plane 合同
- 在 `session / runtime / control_plane` 落最小 dispatch / evidence 闭环
- 为 transcript+workspace degrade 路径补 hooks 约束，明确 hooks 不得伪造 rollback 结果

## Impact

- 让 session 状态切换成为 hooks 正式覆盖的受控扩展面
- 为未来的历史态策略、审计和企业化 guardrail 提供稳定合同
- 避免把 history-state 控制逻辑继续散落在前端状态文案或 `SessionStore` 私有分支中
