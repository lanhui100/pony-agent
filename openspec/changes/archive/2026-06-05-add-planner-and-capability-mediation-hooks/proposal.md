## Why

现在 `planner`、`capability bridge`、`skills ingress` 都已经有稳定边界，但 hooks 还停在 turn/runtime 与即将扩展的 run/memory 层：

- `PA-019` 已稳定 planner facts 与 graph decision boundary
- `PA-020 / PA-021` 已稳定 capability registry / skill ingress 边界
- 后续如果不在这些层建立 hooks mediation，扩展就只能继续靠特判分支或直接改主逻辑

因此需要单独拆一张“planner + capability mediation hooks”卡，把 hooks 扩展到高层决策与能力中介面，但严禁其演化成第二调度层或第二 registry。

## What Changes

- 定义 planner facts hook point 与 capability mediation hook point
- 规范化 planner / capability mediation envelope
- 让 hook evidence 进入 trace / monitor / control-plane
- 明确本卡不扩展到 run-level 或 memory-write hooks

## Capabilities

### New Capabilities

- `planner-and-capability-mediation-hooks`

### Modified Capabilities

- `agent-hooks-pipeline-foundation`
- `mcp-capability-bridge`
- `skills-registry-bridge`
