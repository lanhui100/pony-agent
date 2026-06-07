# PA-031 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-031-define-turn-lifecycle-and-event-contract.md`
- `openspec/changes/add-turn-lifecycle-event-contract/specs/turn-lifecycle-event-contract/spec.md`
- `openspec/changes/add-turn-lifecycle-event-contract/tasks.md`
- `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
- `src-tauri/src/agent/turn_flow.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/sse_adapter.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/types/runtime.ts`
- `src/stores/runtime.ts`
- `tests/runtime-store.spec.ts`

## 审核口径

只按 `PA-031` 的完成边界判断：确认 Pony Agent 已形成统一的 canonical turn lifecycle / event contract，并被 runtime、stream、checkpoint/control-plane 与前端消费共同使用；不把 persisted trace/recovery truth、stable-boundary hook dispatch 或 session control UX 的后续实现重新算回本卡。

### 不在本审计内

- persisted trace / recovery contract 主体：`PA-032`
- checkpoint lifecycle boundary 的运行时实现细节：`PA-034`
- stable-boundary runtime hook dispatch：`PA-035`
- terminal truth-source / session control UX：`PA-036 / PA-037`

## 逐项结论

### A. canonical lifecycle truth

状态：`达成`

证据：

- `docs/architecture/turn-lifecycle-hooks-and-recovery.md` 已沉淀 turn lifecycle 作为底座的母文档
- `turn_flow.rs` / `hooks.rs` 已明确 canonical event vocabulary 与 hook-boundary 对齐关系
- `execution_control.rs` / `runtime.ts` 都已存在 canonical lifecycle -> runtime phase 的受控映射
- `checkpointing` 现在只作为 lifecycle boundary，不再单独承诺 recovery capability

### B. canonical event vocabulary and envelope

状态：`达成`

证据：

- `hooks.rs` 已定义 `CanonicalTurnEventType`
- `turn_flow.rs` 已统一 canonical `eventType / eventVersion / sequence / emittedAtMs`
- `TurnStreamEvent` / SSE 事件帧已具备 `eventId / sessionId / turnId`
- `sse_adapter::tests::format_sse_event_uses_standard_event_id_and_data_lines` 已验证 SSE 采用标准 `id/event/data` 帧格式

### C. multi-hop / failed / cancelled semantics

状态：`达成`

证据：

- `runtime.rs` 已稳定发射：
  - `turn.model_call_started`
  - `turn.tool_call_started`
  - `turn.tool_call_completed`
  - `turn.checkpoint_persisted`
  - `turn.completed / turn.failed / turn.cancelled`
- 多 hop 模型调用与工具 hop 已能被显式表达，而不是依赖前端回推
- 失败与取消终态已具备唯一 terminal semantics，不再和 completed 混淆

### D. frontend consumption prefers canonical lifecycle data

状态：`达成`

证据：

- `src/types/runtime.ts` 已对齐 canonical lifecycle 字段与 event metadata
- `src/stores/runtime.ts` 已优先消费 canonical `eventType / phase / submissionPlan / checkpoint`
- `restorePhaseFromTurnHistory(...)`、streamed event hydration、checkpoint/runtime view hydrate 都已以 canonical metadata 为先
- `tests/runtime-store.spec.ts` 已覆盖“prefers canonical event metadata over legacy phase guessing while streaming”

### E. verification coverage

状态：`达成`

本轮重新执行的关键验证：

```powershell
$env:CARGO_TARGET_DIR='C:\Users\HUAWEI\Documents\pony-agent\src-tauri\target-codex-pa035'
cargo test --manifest-path src-tauri/Cargo.toml sse_adapter::tests::format_sse_event_uses_standard_event_id_and_data_lines --lib -- --exact
cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream --lib -- --exact
cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan --lib -- --exact
cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::start_turn_stream_fail_turn_policy_emits_failed_terminal_with_hook_evidence --lib -- --exact
npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1
```

结果：

- canonical SSE event envelope exact：通过
- multi-hop streamed lifecycle exact：通过
- cancelled terminal exact：通过
- failed terminal exact：通过
- 前端 `runtime-store` 回归：`55 passed`

补充说明：

- Windows 环境下使用新的 `target` 目录冷编译会显著超时，因此本轮复跑复用了已热身的 `target-codex-pa035`
- 后续复跑请改用 `npm run cargo:test:exact` / `npm run cargo:test:exact:b` 这类固定槽位命令，不再继续扩散 `target-codex-*`
- 这不影响 `PA-031` 的验收结论，因为本轮关键 exact 均拿到了成功退出码

## 最终裁定

`PA-031` 已满足任务卡与 delta spec 定义的完成边界，可以从 `Review` 更新为 `Done`。

关闭理由：

1. canonical turn lifecycle / event vocabulary 已成为 runtime、stream、checkpoint 与前端共享的正式真相源。
2. 多 hop / failed / cancelled 终态语义与 SSE envelope 已有 exact Rust 证据和前端回归支撑。
3. 本卡输出的 contract 已被后续 `PA-032 / PA-034 / PA-035 / PA-037` 真实消费，证明它不是停留在文档层的空壳。
4. OpenSpec `tasks.md` 已全部完成，当前不存在阻塞本卡关闭的遗留实现项。
