## Why

`PA-033` 已经把 hooks foundation 的 contract、binding、traceability 与 persisted roundtrip 说清楚，但 runtime 里这套 foundation 仍未真正接线：

- `AgentHookRegistry / NoopHookExecutor` 还停留在独立骨架
- `TurnStreamEvent / TurnTraceRecord` 已有 `hookTraceRecords` 字段，但 runtime 产物基本仍是空值
- 已真实发射的 canonical lifecycle boundary 还没有被 hooks runtime 使用

这会让 hooks 继续停留在“可描述、不可运行”的状态，也会让后续所有扩展都缺少第一条真实的 runtime evidence 链。

因此需要单独拆一张实现卡，只把 hooks foundation 接到已经稳定的 turn lifecycle boundary 上，先拿到最小真实 dispatch 闭环。
本卡依赖已有 checkpoint stable boundary 事实源，但不重新定义 checkpoint / recovery contract。

## What Changes

- 为一组稳定 canonical boundary 增加 runtime hook dispatch
- 将 `AgentHookRegistry + NoopHookExecutor` 正式接入 `AgentRuntime`
- 让 `HookExecutionResult -> HookTraceRecord` 进入实时 turn event 与 persisted turn trace
- 明确本卡不吸收 prepare/context build 早期 boundary，也不扩展到 `run / memory write / planner / skills / MCP`

## Capabilities

### New Capabilities

- `runtime-hook-dispatch-on-stable-boundaries`

### Modified Capabilities

- `agent-hooks-pipeline-foundation`
- `turn-lifecycle-event-contract`
