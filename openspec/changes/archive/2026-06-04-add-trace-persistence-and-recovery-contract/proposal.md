## Why

当前 trace、checkpoint、history、rollback 能力都已有局部实现，但职责边界尚未清楚：

- turn trace 已后端落盘，但前端仍大量二次推导
- execution checkpoint 仍以进程内状态为主，重启后恢复语义不稳定
- history checkout 与 workspace rollback 的成功/降级口径需要正式化

如果不先把 persistence 与 recovery contract 收口，继续修 trace/session 只会持续打补丁。

## What Changes

- 定义 trace persistence 的 canonical source-of-truth 边界
- 区分 ephemeral runtime checkpoint 与 persisted recovery checkpoint
- 定义 transcript-only / transcript+workspace 两类恢复口径
- 明确 frontend 对 trace / checkpoint / history 的消费约束
- 为 `PA-032` 建立严格验收标准

## Capabilities

### New Capabilities

- `trace-persistence-and-recovery-contract`

### Modified Capabilities

- 无

## Impact

- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/stores/runtime.ts`
- `management/task-system/`
