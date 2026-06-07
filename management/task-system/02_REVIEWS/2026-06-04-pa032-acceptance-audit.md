# PA-032 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-032-stabilize-trace-persistence-and-recovery-contract.md`
- `openspec/changes/add-trace-persistence-and-recovery-contract/specs/trace-persistence-and-recovery-contract/spec.md`
- `openspec/changes/add-trace-persistence-and-recovery-contract/tasks.md`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/stores/runtime.ts`
- `tests/runtime-store.spec.ts`

## 审核口径

只按 `PA-032` 任务卡与 delta spec 的完成边界判断，不把后续 hooks runtime dispatch、workflow mode 或更大范围的 agent extensibility 工作重新算回本卡。

### 不在本审计内

- hooks runtime 正式执行链：`PA-033 / PA-022`
- checkpoint lifecycle boundary 的进一步 runtime completion exact coverage：`PA-034`
- skills / capability composition 扩展：`PA-021`

## 逐项结论

### A. persisted trace truth

状态：`达成`

证据：

- `session.rs` 已把 `TurnTraceRecord` 持久化为 session snapshot 的正式组成部分
- `TurnTraceRecord` 已显式承接 `sessionId / eventId / eventType / eventVersion / sequence / emittedAtMs`
- `control_plane -> runtime -> session store` 已支持 terminal lifecycle event metadata 回写到已持久化 trace
- `agent::session::tests::file_backend_roundtrip_restores_checkpoint_persist_evidence` exact 通过，证明 reload 后仍保留 checkpoint persist evidence

### B. runtime checkpoint / recovery checkpoint distinction

状态：`达成`

证据：

- `execution_control.rs` 已定义统一 `ExecutionCheckpoint` 投影，并通过 `refresh_execution_checkpoint_projection(...)` 计算 `projectedRuntimePhase / submissionCommand`
- `control_plane.rs` 已把运行中 turn 投影为 `checkpointKind=runtime_control`
- `control_plane.rs` 已把 graph run recovery 投影为 `checkpointKind=recovery`
- `recoveryMode` 已显式区分 `replay_required / persisted_effect`
- `agent::control_plane::tests::submission_plan_starts_fresh_run_when_recovery_contract_requires_replay` exact 通过
- `agent::control_plane::tests::submission_plan_switches_with_session_checkpoint_boundary` exact 通过

### C. session-level checkpoint switching and execution-plan arbitration

状态：`达成`

证据：

- `load_execution_checkpoint(session_id=...)` 已按优先级仲裁：
  - runtime control
  - graph projected recovery
  - persisted trace lifecycle boundary
- `resolve_graph_run_submission_plan(...)` 已成为后端统一执行入口仲裁面
- `agent::control_plane::tests::session_checkpoint_query_switches_from_runtime_control_to_recovery_after_turn_boundary` 已覆盖 session 级 checkpoint 切换
- `agent::control_plane::tests::submission_plan_falls_back_to_graph_run_when_checkpoint_is_absent` 已覆盖 graph-run fallback
- `agent::control_plane::tests::submission_plan_prefers_checkpoint_projection_when_available` 已覆盖 recovery checkpoint 优先
- `agent::control_plane::tests::submission_plan_switches_with_session_checkpoint_boundary` exact 通过，直接证明同一 session 在 boundary 前后 plan 会从 `continue` 切到 `resume`

### D. history restore and degrade contract

状态：`达成`

证据：

- `HistoryCheckoutResult / HistoryRestoreResult` 已显式暴露 transcript/workspace 双维结果与 degrade reason
- `control_plane.rs` 历史 checkout / restore 测试已验证 transcript restore applied、workspace rollback capable/applied 与 degrade 字段
- 前端 `runtime store` 已对齐同一套结果字段，不再自行猜测 restore 是否完成

### E. hydration and frontend consumption

状态：`达成`

证据：

- `runtime.ts` 已保存并应用 `latestExecutionCheckpoint`
- `loadSessionState()` 已从 `runtimeView.checkpoint` 与 `runtimeView.submissionPlan` 一起 hydrate
- 前端回归已覆盖：
  - runState 缺失但 recovery checkpoint 存在时仍正确 resume
  - stale local runState 与后端 submission plan 冲突时，以后端 plan 为准
  - hydration `submissionPlan` 并复用其 `runId`
  - hydration `start_graph_run_stream` default plan 时不错误复活旧 `runId`

### F. verification

状态：`达成`

本轮完成态证据：

```powershell
npx vitest run tests/runtime-store.spec.ts
npx vue-tsc --noEmit
cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet
$env:CARGO_TARGET_DIR='C:\Users\HUAWEI\Documents\pony-agent\src-tauri\target-codex-pa032'; cargo test --manifest-path src-tauri/Cargo.toml agent::control_plane::tests::submission_plan_starts_fresh_run_when_recovery_contract_requires_replay --lib -- --exact
$env:CARGO_TARGET_DIR='C:\Users\HUAWEI\Documents\pony-agent\src-tauri\target-codex-pa032'; cargo test --manifest-path src-tauri/Cargo.toml agent::control_plane::tests::submission_plan_switches_with_session_checkpoint_boundary --lib -- --exact
$env:CARGO_TARGET_DIR='C:\Users\HUAWEI\Documents\pony-agent\src-tauri\target-codex-pa032'; cargo test --manifest-path src-tauri/Cargo.toml agent::control_plane::tests::file_backed_reload_restores_lifecycle_boundary_projection --lib -- --exact
```

结果：

- 前端 `runtime-store` 定向回归通过：`51 passed`
- `vue-tsc --noEmit` 通过
- `cargo check --tests` 通过
- replay-required 仲裁 exact Rust 单测通过
- 后续如需复跑，请改用固定槽位命令，例如 `npm run cargo:test:exact -- agent::control_plane::tests::submission_plan_starts_fresh_run_when_recovery_contract_requires_replay --lib -- --exact`，不要再新建 `target-codex-*` 目录
- session boundary -> submission plan 切换 exact Rust 单测通过
- reload/lifecycle-boundary roundtrip exact Rust 单测通过

补充说明：

- 默认 `target` 目录在本机 Windows 环境下仍可能出现 `LNK1104` 或 incremental `os error 5` 干扰
- 本轮通过独立 `CARGO_TARGET_DIR` 获得稳定 exact 结果，因此完成态以这些 exact 单测退出码为准

## 最终裁定

`PA-032` 已满足任务卡定义的完成边界，可以从 `Review` 更新为 `Done`。

关闭理由：

1. trace persistence、submission plan 仲裁、history degrade 合同与 hydration 消费边界已经形成完整闭环。
2. reload/hydration、replay-required 仲裁与 session boundary 切换已有后端 exact Rust 证据和前端回归支撑。
3. OpenSpec `tasks.md` 已全部勾选完成，且 `PA-034` 后续工作不再反向阻塞本卡关闭。
4. 当前 recovery contract 已成为后续 session control / lifecycle boundary 任务的稳定输入面。
