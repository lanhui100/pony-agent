# Design: Add Terminal Trace Envelope And Monitor Truth Source

## Context

当前系统已经建立了两层事实：

1. canonical lifecycle event metadata
2. persisted `TurnTraceRecord`

但这两层事实并非在所有 terminal 路径上都完全同构：

- stream path 通过 `RecordingTurnEventSink -> annotate_turn_trace_terminal_event(...)` 回写 terminal envelope
- sync path 直接持久化 trace 时，通常仍写入 `event_id/type/version/sequence/emitted_at_ms = None`

这意味着 monitor / control-plane 虽然能读 trace、算 metrics，但并不能证明“所有 terminal persisted trace 的 envelope 质量一致”。

## Design Decisions

### 1. terminal envelope 的真相源仍然是 persisted trace

- monitor / control-plane 继续以 persisted `TurnTraceRecord` 为聚合输入
- 不允许为 sync trace 额外引入一套“monitor-only fallback event metadata”
- 若 terminal envelope 不完整，应补 runtime / persistence，而不是补 monitor 猜测

### 2. sync / stream 只允许实现路径不同，不允许读面口径不同

- stream path 可以保留“先 emit 再 annotate persisted trace”的结构
- sync path 可以在持久化完成时直接构造 terminal envelope
- 但落盘后的 `TurnTraceRecord` 必须对读面呈现同样的 canonical terminal metadata

### 3. failed / cancelled evidence 必须跟 terminal envelope 一起保真

- 对 failed / cancelled 来说，terminal metadata 不是单独价值
- 若 provider call records、tool activities、hook trace records 在 terminal path 上继续漂移，monitor / drilldown 仍然不可完全信任
- 因此这张卡需要把“terminal envelope + terminal evidence”一起验证，而不是只补 event id

### 4. 本卡不扩 recovery 语义

- `checkpointKind / recoveryMode / resumable / replayable` 仍由 `PA-032 / PA-034` 负责
- 本卡只确保 terminal trace envelope 和 monitor/read-plane 聚合质量

## Implementation Sketch

1. 在 `src-tauri/src/agent/runtime.rs` 收紧 sync completed / failed / cancelled persisted trace 的 terminal envelope 写入
2. 在 `src-tauri/src/agent/session.rs` 保持 terminal envelope roundtrip 与历史兼容
3. 在 `src-tauri/src/agent/control_plane.rs` 为 sync/failed/cancelled drilldown 聚合补 exact evidence 断言
4. 在 `src/components/ModelMonitorPage.vue` 与相关测试中验证 failed/cancelled read-plane 不依赖隐式补偿

## Verification

- Rust exact tests 覆盖：
  - sync completed terminal envelope
  - sync failed terminal envelope
  - sync cancelled terminal envelope
  - reload roundtrip 后 terminal envelope 不丢
  - monitor/control-plane drilldown 对 failed/cancelled evidence 的读面一致性
- 前端 tests 覆盖：
  - runtime-store 对 terminal metadata 的消费不因 sync/stream 来源而分叉
  - monitor drilldown 对 failed/cancelled evidence 的展示不依赖前端造假

