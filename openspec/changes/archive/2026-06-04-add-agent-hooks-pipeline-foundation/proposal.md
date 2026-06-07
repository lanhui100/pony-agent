## Why

hooks 是 Pony Agent 未来不可回避的工业化扩展面，但 hooks 同时与 turn lifecycle、trace、checkpoint 和 recovery 紧密相连。若在 lifecycle contract 未定义清楚时直接扩 hooks，只会让主执行链进一步失控。

因此需要先建立一版“受控 hooks pipeline foundation”，让 hooks 挂在稳定的 lifecycle boundary 上，而不是成为新的隐式调度层。

## What Changes

- 定义 hooks 的分类、权限边界与失败语义
- 定义 hooks 可挂接的 lifecycle boundary
- 定义 hooks 与 trace、checkpoint、recovery 的关系
- 定义 hook registry / executor / trace adapter 的最小骨架目标
- 为 `PA-033` 提供 spec-first 实施边界

## Capabilities

### New Capabilities

- `agent-hooks-pipeline-foundation`

### Modified Capabilities

- 无

## Impact

- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/session.rs`
- `src/stores/runtime.ts`
- `management/task-system/`
