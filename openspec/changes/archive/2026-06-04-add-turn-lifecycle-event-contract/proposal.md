## Why

当前 runtime、turn stream、trace persistence、frontend runtime store 都已经具备部分 turn 生命周期信号，但缺少统一的 canonical lifecycle contract。结果是：

- 后端执行阶段与前端展示阶段并不一一对应
- trace、checkpoint、history、UI 都在各自推导“当前 turn 到底发生了什么”
- 后续若直接引入 hooks，会把执行主线进一步打散

要让 trace 成为稳定观测面、让恢复能力可验证、让 hooks 可拓展，必须先把 `turn lifecycle + event contract` 正式收口。

## What Changes

- 定义 Pony Agent 的 canonical turn phase 状态机
- 定义 turn 事件流的命名、顺序、终态与 payload 基线
- 明确多 hop model/tool 执行在生命周期中的表达方式
- 定义前后端共享的最小 contract，与现有 runtime/session/stream 出口对齐
- 为 `PA-031` 建立 spec-first 实施基线，并作为 `PA-032`、`PA-033` 的前置依赖

## Capabilities

### New Capabilities

- `turn-lifecycle-event-contract`

### Modified Capabilities

- 无

## Impact

- Rust runtime：`src-tauri/src/agent/runtime.rs`
- turn 事件出口：`src-tauri/src/agent/turn_flow.rs`
- 宿主读面：`src-tauri/src/agent/control_plane.rs`
- 共享类型与前端消费：`src/types/runtime.ts`、`src/stores/runtime.ts`
- 文档与任务系统：`docs/architecture/`、`management/task-system/`
