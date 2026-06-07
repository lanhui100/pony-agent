# PA-035 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-035-integrate-runtime-hook-dispatch-on-stable-boundaries.md`
- `openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/specs/runtime-hook-dispatch-on-stable-boundaries/spec.md`
- `openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/tasks.md`
- `src-tauri/src/agent/hooks.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/stores/runtime.ts`
- `tests/runtime-store.spec.ts`

## 审核口径

只按 `PA-035` 的完成边界判断：确认 hooks foundation 已被 runtime 接到一组真实稳定的 lifecycle boundary 上，且 runtime 产出的 hook trace evidence 能进入实时事件、persisted trace、reload/control-plane 与前端读面；不把 patch/side-effect 正式 applier、prepare/context build 提前接线或更大范围的 hooks 扩展重新算回本卡。

### 不在本审计内

- prepare/context build 早期 boundary 的真实 runtime 发射面
- patch / side-effect 结果的正式 contract applier
- memory / planner / skills / MCP 等 post-foundation hooks 扩展：`PA-022`

## 逐项结论

### A. stable-boundary runtime dispatch

状态：`达成`

证据：

- `runtime.rs` 已持有 `AgentHookRegistry + AgentHookExecutor`
- `dispatch_hook_trace_records(...)` 已按 registry priority 在稳定 boundary 上执行 hooks
- 首轮接线 boundary 已覆盖：
  - `ModelCallStart`
  - `ToolCallStart`
  - `ToolCallEnd`
  - `CheckpointPersistEnd`
  - `TurnFinalizeEnd`
- `agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries` 已证明 streamed stable boundary dispatch 真实发生

### B. trace evidence chain

状态：`达成`

证据：

- `turn_flow.rs` 事件发射链已支持携带 `hook_trace_records`
- `runtime.rs` 已把 runtime-produced `hook_trace_records` 写入：
  - `TurnStreamEvent`
  - persisted `TurnTraceRecord`
  - sync `TurnResult`
- streamed multi-hop 路径的 `turn:trace / turn:tool / turn:checkpoint_persisted / turn:completed / turn:failed` 已可带 hooks evidence
- sync completed / failed terminal path 也已补齐 hooks evidence 持久化

### C. ordering and controlled execution

状态：`达成`

证据：

- `dispatch_hook_trace_records(...)` 按 registry 顺序填充 `hook_order`
- `agent::runtime::tests::runtime_hook_dispatch_returns_trace_records_in_priority_order` 已覆盖稳定顺序
- hooks 仍只输出 trace evidence，不直接改 runtime 内部 store
- 本卡未引入新的 lifecycle phase，也未绕开 canonical model/tool path

### D. failure policy matrix

状态：`达成`

证据：

- 默认 executor failure 已按 evidence-first / non-blocking 记录到 `HookTraceRecord`
- `Degrade` 首轮仍保持 non-blocking
- `FailTurn` 已在 streamed 与 sync stable boundary 上具备最小闭环：
  - terminal `turn:failed`
  - failed `TurnResult`
  - persisted failed trace
- 任务卡中列出的 `FailTurn` streamed/sync exact 用例已形成完整矩阵

### E. reload / control-plane / frontend read-plane

状态：`达成`

证据：

- `agent::session::tests::file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces` 已证明 file-backend reload 保留 runtime-produced hook traces
- `agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics` 已证明 control-plane runtime view / hook aggregates 能读回 persisted hook evidence
- `tests/runtime-store.spec.ts` 已证明前端 hydration 与 streamed event 消费不会丢失 `hookTraceRecords`

### F. unstable-boundary guardrail

状态：`达成`

证据：

- `agent::runtime::tests::start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks` 已证明 runtime 不会为了接 hooks 人造 `TurnPrepare* / ContextBuild*` dispatch
- 这使 `PA-035` 继续保持“只接真实稳定 boundary”的实现边界

## 完成态验证

本轮重新执行的关键验证：

```powershell
$env:CARGO_TARGET_DIR='C:\Users\HUAWEI\Documents\pony-agent\src-tauri\target-codex-pa035'
cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries --lib -- --exact
cargo test --manifest-path src-tauri/Cargo.toml agent::session::tests::file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces --lib -- --exact
cargo test --manifest-path src-tauri/Cargo.toml agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics --lib -- --exact
npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1
```

结果：

- streamed stable-boundary persisted evidence exact：通过
- file-backend runtime-produced hook traces roundtrip exact：通过
- control-plane runtime view / hook metrics exact：通过
- 前端 `runtime-store` 定向回归：`55 passed`
- 后续复跑请改用 `npm run cargo:test:exact` / `npm run cargo:test:exact:b` 复用固定缓存槽位，避免再次生成任务号 target 目录

补充说明：

- Windows 环境仍会出现 incremental `os error 5` / file lock 警告
- 但本轮关键 exact 与前端回归都拿到了成功退出码，可作为正式验收凭证

## 最终裁定

`PA-035` 已满足任务卡与 delta spec 定义的完成边界，可以从 `Review` 更新为 `Done`。

关闭理由：

1. runtime stable-boundary hook dispatch、trace evidence、persisted roundtrip 与 control-plane/frontend read-plane 已形成完整闭环。
2. ordering、non-blocking/degrade、`FailTurn`、reload 与 unstable-boundary guardrail 均已有验证矩阵支撑。
3. 本卡严格维持 trace-first integration 范围，没有演化成新的隐式调度层。
4. OpenSpec `tasks.md` 已全部完成，后续更大 hooks 扩展应单独立项，不再阻塞本卡关闭。
