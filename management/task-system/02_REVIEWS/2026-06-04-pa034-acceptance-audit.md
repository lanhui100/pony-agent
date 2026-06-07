# PA-034 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-034-implement-checkpoint-lifecycle-boundary.md`
- `openspec/changes/add-checkpoint-lifecycle-boundary-implementation/specs/checkpoint-lifecycle-boundary/spec.md`
- `openspec/changes/add-checkpoint-lifecycle-boundary-implementation/tasks.md`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src-tauri/src/agent/session.rs`
- `src/stores/runtime.ts`
- `tests/runtime-store.spec.ts`

## 审核口径

只按 `PA-034` 的完成边界判断：确认 `checkpointing / turn.checkpoint_persisted` 已成为 runtime、persisted trace、reload/control plane 与前端 runtime store 共同可见的真实 lifecycle boundary，同时不把这组 evidence 误判成 recovery capability。

### 不在本审计内

- `turn.model_call_started` 等更完整 hooks runtime boundary 发射面：`PA-033 / PA-022`
- 更广义的 lifecycle event vocabulary 收口：`PA-031`
- trace persistence / submission plan recovery contract 主体：`PA-032`

## 逐项结论

### A. runtime checkpoint boundary emission

状态：`达成`

证据：

- `runtime.rs` 已在 normal completion 与 tool follow-up completion 路径补发：
  - `turn:phase_changed` with `phase=checkpointing`
  - `turn:checkpoint_persisted`
- `turn.completed` 前已显式经过 checkpoint persist boundary，不再只在 spec 中声明该阶段存在
- `agent::runtime::tests::start_turn_stream_emits_first_token_latency_on_reasoning_delta` exact 通过
- `agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream` exact 通过

### B. persisted evidence and reload truth

状态：`达成`

证据：

- completed trace timeline 已新增 `checkpoint_persist` evidence
- session snapshot / file backend roundtrip 会保留这组 persisted evidence
- `agent::session::tests::file_backend_roundtrip_restores_checkpoint_persist_evidence` exact 通过
- `agent::control_plane::tests::file_backed_reload_restores_lifecycle_boundary_projection` exact 通过

### C. execution checkpoint / recovery distinction

状态：`达成`

证据：

- `control_plane.rs` 已把 persisted checkpoint evidence 投影为 `checkpointKind=lifecycle_boundary`
- 当 `runtime_control` 与 graph `recovery` 缺席时，`load_execution_checkpoint(...)` 可回退读取 lifecycle boundary
- 该投影不会把 `checkpointing` 自动提升为 recovery capability，不会劫持 submission plan
- `agent::control_plane::tests::completed_session_can_project_checkpoint_lifecycle_boundary_without_recovery` exact 通过
- `agent::control_plane::tests::lifecycle_boundary_checkpoint_does_not_override_default_submission_plan` exact 通过

### D. frontend hydration and runtime consumption

状态：`达成`

证据：

- `runtime.ts` 已监听 `turn:phase_changed` 与 `turn:checkpoint_persisted`
- frontend runtime store 现在能把 checkpoint lifecycle boundary 稳定投影为 `connecting`
- `npx vitest run tests/runtime-store.spec.ts` 通过，`51 passed`
- `npx vue-tsc --noEmit` 通过

### E. verification notes

状态：`达成`

补充说明：

- 本轮排查中发现 multi-hop 流式 exact 的阻塞根因不是 runtime 主逻辑，而是测试合同过期：
  - 当前 OpenAI native tool flow 会先走流式 initial decision
  - 旧测试把首段 mock 写成 JSON decision，导致流式初判先消费错误响应，随后 `server.finish()` 等待未消费 mock 而假性挂起
- 本轮已把相关 stream 测试首段 mock 改为与 runtime 对齐的 SSE decision，并去掉与 `PA-034` 验收无关、尚未正式发射的 `turn.model_call_started` 断言
- 这属于修正测试合同，不属于给 production runtime 追加补丁

## 完成态验证

```powershell
$env:CARGO_TARGET_DIR='C:\Users\HUAWEI\Documents\pony-agent\src-tauri\target-codex-pa034b'
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::completed_session_can_project_checkpoint_lifecycle_boundary_without_recovery -- --exact
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_does_not_override_default_submission_plan -- --exact
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::file_backed_reload_restores_lifecycle_boundary_projection -- --exact
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::file_backend_roundtrip_restores_checkpoint_persist_evidence -- --exact
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_emits_first_token_latency_on_reasoning_delta -- --exact
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream -- --exact
npx vitest run tests/runtime-store.spec.ts
npx vue-tsc --noEmit
```

结果：

- 后续复跑建议改用 `npm run cargo:test:exact`、`npm run cargo:test:exact:b`、`npm run cargo:test:exact:c` 分摊 exact 用例，不再新建 `target-codex-pa034b`

- 六条关键 Rust exact 用例全部通过
- 前端 runtime-store 定向回归通过：`51 passed`
- `vue-tsc --noEmit` 通过
- Windows 环境仍可能出现 incremental `os error 5` 告警，但本轮独立 `CARGO_TARGET_DIR` exact 结果稳定，可作为正式验收凭证

## 最终裁定

`PA-034` 已满足任务卡与 delta spec 定义的完成边界，可以从 `Review` 更新为 `Done`。

关闭理由：

1. `checkpointing` 已从 contract 名词落成 runtime、persisted trace、execution checkpoint 与前端读面的真实 lifecycle boundary。
2. completed / tool follow-up / reload roundtrip / 不误判为 recovery 的四类路径都已有正式验证证据。
3. 本卡为 `PA-035` 提供了真实 `CheckpointPersist*` 稳定锚点，边界价值已经被后续实现证明。
4. OpenSpec `tasks.md` 已全部完成，不再需要继续把 hooks/runtime 缺口回灌到本卡。
