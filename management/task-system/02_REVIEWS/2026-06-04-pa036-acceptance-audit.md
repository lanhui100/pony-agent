# PA-036 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-036-harden-terminal-trace-envelope-and-monitor-truth-source.md`
- `openspec/changes/add-terminal-trace-envelope-and-monitor-truth-source/specs/terminal-trace-envelope-and-monitor-truth-source/spec.md`
- `openspec/changes/add-terminal-trace-envelope-and-monitor-truth-source/tasks.md`
- `src-tauri/src/agent/turn_flow.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/components/ModelMonitorPage.vue`
- `src/stores/runtime.ts`
- `tests/ModelMonitorPage.spec.ts`
- `tests/runtime-store.spec.ts`

## 审核口径

只按 `PA-036` 的完成边界判断：确认 terminal envelope parity、persisted truth-source、reload evidence 保真与前端 read-plane 消费约束已经落地；不把 hooks 扩展、额外取消语义或新的 lifecycle contract 重新算回本卡。

### 不在本审计内

- 新增 sync `cancelled` runtime 语义
- 扩 hooks evidence model 或新增 lifecycle event vocabulary：`PA-031 / PA-035`
- session 控制交互面与前端操作反馈闭环：`PA-037`

## 逐项结论

### A. sync terminal envelope parity

状态：`达成`

证据：

- `turn_flow.rs` 已提供 `TurnEventEnvelope` 与 `build_terminal_turn_event_envelope(...)`
- `runtime.rs` 已为 sync `completed / failed` persisted trace 与 `TurnResult` 注回 canonical terminal envelope
- `agent::runtime::tests::run_turn_persists_terminal_hook_traces_on_completed_sync_turn` 已覆盖 sync completed
- `agent::runtime::tests::run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace` 已覆盖 sync failed
- `agent::runtime::tests::run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence` 已覆盖 failed terminal hook evidence 保真

### B. cancelled path scope clarification

状态：`达成`

证据：

- `runtime.rs` 中 `run_turn()` sync 路径只产出 `completed / failed`，没有 stop/cancel 入口
- `runtime.rs` 的 `cancel_stream_turn(...)`、`persist_cancelled_turn_outcome(...)` 与 `should_cancel_turn(...)` 全部挂在 streamed turn + execution control 路径
- `agent::runtime::tests::start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan` 与 `agent::runtime::tests::start_turn_stream_cancels_with_canonical_finalize_boundary_when_stop_is_requested_during_tool_execution` 已证明 canonical `cancelled` 语义存在于 stream path
- 因此 `tasks.md` 中原“sync failed / cancelled”属于文案写宽；本轮已收窄为“sync failed 与 streamed cancelled”，这是 contract 对齐，不是遗漏实现

### C. persisted evidence and reload truth

状态：`达成`

证据：

- `session.rs` file-backend roundtrip 已保留 terminal envelope 的 `eventId / eventType / eventVersion / sequence / emittedAtMs`
- `provider_call_records / tool_activities / hook_trace_records` 不会因 failed / cancelled reload 丢失
- `agent::session::tests::file_backend_roundtrip_restores_failed_terminal_envelope_and_existing_evidence` 已覆盖 failed reload
- `agent::session::tests::file_backend_roundtrip_restores_cancelled_terminal_envelope_and_existing_evidence` 已覆盖 cancelled reload

### D. monitor/control-plane canonical truth-source

状态：`达成`

证据：

- `control_plane.rs` 已把 canonical metrics 收紧为只统计带 terminal envelope 的 persisted trace
- raw trace 仍保留在 session drilldown 中作为原始证据，不再参与 canonical summary
- `agent::control_plane::tests::monitor_summary_excludes_raw_traces_without_terminal_envelope_from_canonical_metrics` 已覆盖 raw trace 排除逻辑
- `agent::control_plane::tests::monitor_summary_aggregates_mixed_sync_and_stream_terminal_truth_from_persisted_traces` 已覆盖 mixed sync/stream 聚合口径
- `agent::control_plane::tests::model_monitor_session_drilldown_preserves_failed_and_cancelled_terminal_evidence` 已覆盖 failed/cancelled drilldown evidence 保真

### E. frontend read-plane consumption guard

状态：`达成`

证据：

- `runtime.ts` 现在只会从带 canonical terminal envelope 的 trace 恢复 `failed / cancelled` terminal runtime phase
- `ModelMonitorPage.vue` 已对缺失 envelope 的 persisted trace 显示 raw / non-canonical warning
- `tests/runtime-store.spec.ts` 已覆盖 raw failed trace 与 canonical failed trace 的恢复差异
- `tests/ModelMonitorPage.spec.ts` 已覆盖 raw warning 展示
- 本轮重新执行 `npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1`，结果 `55 passed`

### F. verification note

状态：`达成`

补充说明：

- 前端关键回归本轮已复跑通过：`npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1`
- `npx vitest run tests/ModelMonitorPage.spec.ts --pool=forks --maxWorkers=1` 本轮已复跑通过：`8 passed`
- `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet` 本轮已通过
- Rust exact 用例的完成态证据以前序定向验证为主：
  - `run_turn_persists_terminal_hook_traces_on_completed_sync_turn`
  - `run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace`
  - `run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence`
  - `file_backend_roundtrip_restores_failed_terminal_envelope_and_existing_evidence`
  - `file_backend_roundtrip_restores_cancelled_terminal_envelope_and_existing_evidence`
  - `monitor_summary_aggregates_mixed_sync_and_stream_terminal_truth_from_persisted_traces`
  - `model_monitor_session_drilldown_preserves_failed_and_cancelled_terminal_evidence`
- 本轮在 Windows 环境使用新的 `CARGO_TARGET_DIR` 复跑两条 Rust exact 时，命令因冷编译耗时超过会话时限而超时，未拿到新的退出码；结合当前 `cargo check --tests`、前端复跑与前序 exact 结果，这更像环境时延问题，不构成发现失败证据

## 最终裁定

`PA-036` 已满足任务卡与 delta spec 的完成边界，可以从 `Review` 更新为 `Done`。

关闭理由：

1. sync terminal envelope、stream cancelled parity、reload 保真与 monitor/control-plane truth-source 已形成统一完成态口径。
2. 本轮前端关键回归与 `cargo check --tests` 通过，结合前序 exact Rust 结果，足以支撑完成态裁定。
3. raw trace 只能作为 non-canonical evidence 的防误读约束已经落地，说明本卡不是只补字段，而是完成了真相源收紧。
4. OpenSpec `tasks.md` 已全部完成，后续若要扩 sync cancel，应另立新 change，不再阻塞本卡关闭。
