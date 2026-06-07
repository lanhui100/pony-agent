# PA-035 落地 runtime hook dispatch stable-boundary integration

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- [add-runtime-hook-dispatch-on-stable-boundaries](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries)

## Delta Spec
- [runtime-hook-dispatch-on-stable-boundaries/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/specs/runtime-hook-dispatch-on-stable-boundaries/spec.md)

## Canonical Spec
- 交付后沉淀到 `openspec/specs/` 的 runtime hook dispatch canonical spec

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把 `PA-033` 已完成的 hooks foundation 真正接到 runtime 的稳定 lifecycle boundary 上，让 hooks 至少在一组已真实发射的 canonical boundary 上可被调度、排序、执行并进入实时 trace / persisted trace，而不把这张卡扩成新的隐式调度层。

## 输出
- runtime 对稳定 hook boundary 的最小 dispatch 闭环
- `AgentHookRegistry + NoopHookExecutor` 到 `runtime` 的正式接线
- `HookExecutionResult -> HookTraceRecord -> TurnStreamEvent / TurnTraceRecord / SessionSnapshot` 的真实产物链
- 稳定 boundary 上的 hook ordering / failure policy / trace evidence 验证矩阵
- 与 `PA-033 / PA-022` 的范围分工说明

## 验收标准
- runtime 只在已真实发射的 canonical lifecycle boundary 上执行 hooks
- hook dispatch 不要求宿主、前端或 provider 了解 runtime 内部细节
- 本卡至少覆盖一组稳定 boundary：`ModelCallStart / ToolCallStart / ToolCallEnd / CheckpointPersistEnd / TurnFinalizeEnd`
- hook 执行结果会进入 `TurnStreamEvent` 与 persisted `TurnTraceRecord`
- dispatch 仍保持受控：不直接让 hook 任意改内部 store，不创建新 lifecycle phase，不绕开 canonical model/tool path
- 首轮 runtime dispatch 以 trace-first integration 为验收目标，不要求本轮完成 patch / side-effect 的正式 contract applier
- 测试覆盖 ordering、failure policy、trace evidence、reload roundtrip 与“仅稳定 boundary 接线”的边界约束

## 当前进展
- `PA-033` 已交付 foundation/no-op contract、binding、traceability 与 persisted roundtrip 基础
- `PA-034` 已把 checkpoint boundary 做成 runtime 真实事实源，为 `CheckpointPersistEnd` 提供稳定锚点
- `PA-035` 只消费 `PA-034` 提供的 checkpoint stable boundary 事实，不重复定义 checkpoint / recovery contract
- 已完成独立 spec 审核并采纳修订，见：
  [2026-06-04-pa035-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa035-spec-review.md)
- 子智能体只读审计已确认：
  - 当前 runtime 已真实发射的 boundary 足以支撑一张“stable-boundary dispatch”实现卡
  - prepare/context build 等早期 boundary 仍缺真实事件面，不宜在本卡首轮强接
  - 真正的 runtime hook dispatch 已超出 `PA-033`，应拆独立实现卡推进
- 已完成第一段 runtime 脚手架实现：
  - `hooks.rs` 已新增 `AgentHookExecutor: Send + Sync`
  - `runtime.rs` 已开始正式持有 `AgentHookRegistry + hook executor`
  - 已新增 `dispatch_hook_trace_records(...)` helper，按 registry priority 产出 `HookTraceRecord`
  - 已新增 runtime 定向测试，验证 priority 排序与未注册 boundary 的空结果
- 已完成第二段 terminal-path integration：
  - `turn_flow.rs` 的 `emit_stream_event(...)` 已补齐 `hook_trace_records + session_id` 新签名
  - no-tool completion 路径已接入 `CheckpointPersistEnd / TurnFinalizeEnd`
  - tool-follow-up completion 路径已接入 `CheckpointPersistEnd / TurnFinalizeEnd`
  - `turn:checkpoint_persisted` 与 `turn:completed` 已可携带 runtime-produced `hook_trace_records`
  - persisted `TurnTraceRecord` 已可写入 terminal hook trace records
- 已完成第三段 stable-boundary realtime integration：
  - 初始 provider decision 与 tool-follow-up provider call 前，runtime 已真实发射 `ModelCallStart`
  - tool execution 前后，runtime 已真实发射 `ToolCallStart / ToolCallEnd`
  - 已修正 no-tool completion 路径中的 post-response trace phase，避免把 response-ready 误报为重复的 `ModelCallStart`
  - multi-hop 流式事件已可在 `turn:trace / turn:tool` 上携带对应 boundary 的 runtime-produced `hook_trace_records`
- 已补 runtime 产物级回归测试：
  - `agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries`
  - 该测试已验证 `event -> persisted trace` 的 terminal hook trace roundtrip
- 已补 stable-boundary realtime trace evidence：
  - `agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream`
  - 该测试现已验证多 hop 流式路径上的 `ModelCallStart / ToolCallStart / ToolCallEnd` hook trace evidence
- 已补 frontend hydration / streamed-event read-plane 回归：
  - `tests/runtime-store.spec.ts`
  - 已验证 backend snapshot reload 可保留 `hookTraceRecords`
  - 已验证 streamed `turn:trace / turn:tool / turn:completed` 事件可把 runtime-produced `hookTraceRecords` 写入前端 `turnTraceHistory`
  - 已修复前端 store 在 `turn:tool` 事件落库时漏传 `hookTraceRecords` 的真实缺口
- 已补 backend reload / control-plane read-plane 回归：
  - `agent::session::tests::file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces`
  - `agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics`
  - 已验证 runtime-produced 多 boundary `hookTraceRecords` 可经 file backend roundtrip 保留
  - 已验证 control-plane runtime view / session drilldown 能读回 persisted hook trace records 与 hook aggregates
- 已补首轮 non-blocking failure evidence：
  - `dispatch_hook_trace_records(...)` 不再把 executor error 直接升级成 turn 失败
  - 默认会把 executor failure 归一化写成 hook trace evidence，继续保持 stable-boundary 首轮 non-blocking 口径
  - 已新增 `agent::runtime::tests::runtime_hook_dispatch_records_executor_failure_without_stopping_turn_by_default`
  - 已新增 `agent::runtime::tests::runtime_hook_dispatch_records_degrade_failure_evidence_without_stopping_turn`
  - 已验证 `Degrade` 在首轮仍走 evidence-first / non-blocking 口径
- 已补 `FailTurn` 最小 runtime 闭环：
  - `dispatch_hook_trace_records(...)` 现会保留 `FailTurn` executor failure 的 blocked trace evidence
  - `turn:failed` terminal payload 已支持携带 `hookTraceRecords`
  - 已在 streamed `ModelCallStart / ToolCallStart / ToolCallEnd / CheckpointPersistEnd / TurnFinalizeEnd` 路径接入 `FailTurn -> turn:failed + persisted failed trace`
  - 已新增 `agent::runtime::tests::start_turn_stream_fail_turn_policy_emits_failed_terminal_with_hook_evidence`
  - 已新增 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_start_stops_before_tool_execution`
  - 已新增 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_end_stops_before_followup_model_call`
  - 已新增 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_checkpoint_boundary_emits_failed_instead_of_completed`
  - 已新增 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_finalize_boundary_emits_failed_with_terminal_hook_evidence`
- 已补 sync path 的最小 `FailTurn` evidence：
  - `run_turn()` 已接入 `ModelCallStart -> FailTurn -> failed TurnResult(hook evidence)`
  - `handle_sync_tool_turn()` 已接入 `ToolCallStart / ToolCallEnd -> FailTurn -> failed TurnResult(hook evidence)`
  - 已新增 `agent::runtime::tests::run_turn_fail_turn_policy_on_model_call_start_returns_failed_result_with_hook_evidence`
  - 已新增 `agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_start_returns_failed_before_tool_execution`
  - 已新增 `agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_end_returns_failed_before_followup_model_call`
- 已补 sync terminal-boundary completion / failure 闭环：
  - `run_turn()` completed path 已接入 `CheckpointPersistEnd / TurnFinalizeEnd`
  - sync completed `TurnResult.hook_trace_records` 与 persisted `TurnTraceRecord.hook_trace_records`
    已不再只停留在 initial model hook evidence
  - sync attachments failure 已改为持久化带 hooks 的 failed trace
  - sync `CheckpointPersistEnd / TurnFinalizeEnd` 已具备
    `FailTurn -> failed TurnResult(hook evidence) + persisted failed trace` 最小闭环
  - 已新增 `agent::runtime::tests::run_turn_persists_terminal_hook_traces_on_completed_sync_turn`
  - 已新增 `agent::runtime::tests::run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace`
  - 已新增 `agent::runtime::tests::run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence`
- 已补 unstable boundary guardrail：
  - 已新增 `agent::runtime::tests::start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks`
  - 已验证即使注册了 `TurnPrepareStart / TurnPrepareEnd / ContextBuildStart / ContextBuildEnd` hooks，runtime 也不会为了接线人造这些 boundary 的 dispatch
- 已完成本轮定向验证：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib --no-run`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_returns_trace_records_in_priority_order -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_returns_empty_for_unregistered_boundary -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_records_executor_failure_without_stopping_turn_by_default -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_records_degrade_failure_evidence_without_stopping_turn -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_emits_failed_terminal_with_hook_evidence -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_start_stops_before_tool_execution -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_end_stops_before_followup_model_call -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_checkpoint_boundary_emits_failed_instead_of_completed -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_finalize_boundary_emits_failed_with_terminal_hook_evidence -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_fail_turn_policy_on_model_call_start_returns_failed_result_with_hook_evidence -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_start_returns_failed_before_tool_execution -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_end_returns_failed_before_followup_model_call -- --exact`
  - `npm run cargo:test:exact -- agent::runtime::tests::run_turn_persists_terminal_hook_traces_on_completed_sync_turn -- --exact`
  - `npm run cargo:test:exact -- agent::runtime::tests::run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace -- --exact`
  - `npm run cargo:test:exact -- agent::runtime::tests::run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence -- --exact`
  - `npm run cargo:test:exact -- agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries -- --exact`
  - `npm run cargo:test:exact:b -- agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics -- --exact`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::registry_rejects_persisted_effect_without_persistence_requirement -- --exact`
  - `npm run test:unit -- --run tests/runtime-store.spec.ts`

## 下一步动作
- 后续若要扩展 prepare/context build、patch applier 或 post-foundation hooks，应按新 change 单独推进
- 将 hooks 主线继续聚焦到剩余未关账的 lifecycle / extensibility 范围，而不是继续在本卡上扩 scope

## 当前卡点
- 暂无；本卡已完成关闭

## 与 PA-033 / PA-022 的关系
- `PA-033` 负责 foundation、binding、traceability 合同，不负责正式 runtime dispatch integration
- `PA-035` 负责把 foundation 接到稳定 turn boundary 的 runtime 执行链
- `PA-022` 继续保留 `run / memory write / planner / skills / MCP` hooks 等 post-foundation 扩展范围

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-033-build-agent-hooks-pipeline-foundation.md`
- `management/task-system/03_TASKS/PA-022-build-lifecycle-hooks-pipeline.md`
- `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
- `src-tauri/src/agent/hooks.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
