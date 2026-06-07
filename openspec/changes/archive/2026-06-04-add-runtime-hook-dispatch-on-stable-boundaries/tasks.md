# Tasks: Add Runtime Hook Dispatch On Stable Boundaries

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-035` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-runtime-hook-dispatch-on-stable-boundaries` 的 proposal / design / spec 文档

## 2. Runtime Dispatch Foundation

- [x] 2.1 为 `AgentRuntime` 引入 hook registry / executor 持有与 stable-boundary dispatch helper
- [x] 2.2 在 `ModelCallStart / ToolCallStart / ToolCallEnd / TurnFinalizeEnd` 接入最小 runtime hook dispatch
- [x] 2.3 在 `CheckpointPersistEnd` 接入最小 runtime hook dispatch，并只消费现有 checkpoint stable boundary 事实
- [x] 2.4 让 runtime 产出的 `hookTraceRecords` 进入实时 event 与 persisted turn trace

## 3. Verification and Closeout

- [x] 3.1 补 ordering / failure policy / trace evidence 的 Rust 定向测试
- [x] 3.2 补 reload/persisted roundtrip 对 runtime-produced hook trace records 的回归测试
- [x] 3.3 补 frontend hydration/clone 对 runtime-produced hook trace records 的回归测试
- [x] 3.4 补 stable-boundary failure-policy matrix 与“首轮 non-blocking 为主”的负例测试
- [x] 3.5 验证 prepare/context build 未被 runtime 人造接线
- [x] 3.6 完成独立 spec 审核并采纳必要修订
- [x] 3.7 回写任务卡、review 文档、日志与验收证据

## 进度备注

- 已完成 terminal-path trace-first integration：
  - `turn:checkpoint_persisted`
  - `turn:completed`
  - persisted `TurnTraceRecord`
- `2.4 / 3.1 / 3.4` 当前为部分完成：
  - 已有 runtime-produced realtime + terminal hook trace evidence
  - 已补默认 non-blocking executor failure evidence
  - 已补 `Degrade` 在首轮仍保持 non-blocking 的定向 evidence 测试
  - 已补 streamed stable-boundary 全覆盖：
    `ModelCallStart / ToolCallStart / ToolCallEnd / CheckpointPersistEnd / TurnFinalizeEnd`
    均已具备 `FailTurn -> turn:failed/persisted failed trace` 最小闭环
  - 已补 sync `ModelCallStart / ToolCallStart / ToolCallEnd` 的 `FailTurn -> failed TurnResult(hook evidence)` 最小闭环
  - 已补 sync `CheckpointPersistEnd / TurnFinalizeEnd` 的
    `completed TurnResult + persisted trace` terminal hook evidence
  - 已补 sync `CheckpointPersistEnd / TurnFinalizeEnd` 的
    `FailTurn -> failed TurnResult(hook evidence) + persisted failed trace` 最小闭环
- `3.2` 已完成：
  - 已补 session file backend roundtrip 对多 boundary runtime-produced hook traces 的回归
  - 已补 control-plane runtime view / monitor drilldown 对 persisted hook trace records 与 hook aggregates 的读面回归
- `3.5` 已完成：
  - 已补 `start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks`
  - 已验证 prepare/context build hooks 不会被 runtime 为了接线而人造发射
