# 2026-06-04 Session 74

## 主题

- 收紧 `PA-033` 的 foundation 完成边界
- 新建 `PA-035`，把 runtime hook dispatch 从 foundation 中独立拆卡

## 本轮完成

1. 完成两轮只读子智能体审计：
   - 一轮聚焦 `PA-033` 的 spec / 任务卡 / OpenSpec tasks 边界
   - 一轮聚焦 runtime 真实 boundary 与 hooks foundation 的代码缺口
2. 审计共识收敛为：
   - `PA-033` 应收口为 foundation / no-op contract / binding / traceability
   - `PA-033` 不应再吸收 checkpoint runtime implementation 或正式 hook dispatch integration
   - runtime hook dispatch 应拆独立实现卡推进
3. 已更新 `PA-033` 任务卡：
   - 状态从 `In Progress` 调整为 `Review`
   - 明确其交付口径是不接 runtime dispatch 主链
4. 已新建 `PA-035` 任务卡与 OpenSpec change：
   - [PA-035](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-035-integrate-runtime-hook-dispatch-on-stable-boundaries.md)
   - [proposal.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/proposal.md)
   - [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/design.md)
   - [spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/specs/runtime-hook-dispatch-on-stable-boundaries/spec.md)
   - [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/tasks.md)
5. 已同步任务系统主视图：
   - `PA-033` 移到 `Review`
   - `PA-035` 进入 `In Progress`
   - dashboard 已更新 hooks 主线的阶段说明
6. 已完成 `PA-035` 独立 spec 审核并采纳修订：
   - 审核记录见 [2026-06-04-pa035-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa035-spec-review.md)
   - 已明确首轮 `trace-first integration`、`non-blocking` 默认口径，以及 `PA-034` 依赖边界
7. 已开始第一段 runtime 脚手架实现：
   - `hooks.rs` 已新增 `AgentHookExecutor: Send + Sync`
   - `runtime.rs` 已开始正式持有 hook registry / executor
   - 已新增 `dispatch_hook_trace_records(...)` helper
   - 已新增 runtime 定向测试，验证 priority 排序与未注册 boundary 的空结果
8. 已完成第二段 terminal-path integration：
   - 修平 `emit_stream_event(...)` 新签名带来的 runtime 中间态
   - no-tool completion 与 tool-follow-up completion 两条稳定完成路径已接入
     - `CheckpointPersistEnd`
     - `TurnFinalizeEnd`
   - `turn:checkpoint_persisted` 与 `turn:completed` 已开始承载 runtime-produced `hook_trace_records`
   - persisted `TurnTraceRecord` 已可写入 terminal hook trace evidence
9. 已新增 runtime 产物链回归测试：
   - `agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries`
   - 验证点覆盖：
     - `turn:checkpoint_persisted.hook_trace_records`
     - `turn:completed.hook_trace_records`
     - `load_session_snapshot(...).turn_trace_history.last().hook_trace_records`
10. 已完成第三段 stable-boundary realtime integration：
   - 初始 provider decision 前已真实发射 `ModelCallStart`
   - tool-follow-up provider call 前已真实发射 `ModelCallStart`
   - tool execution 前后已真实发射
     - `ToolCallStart`
     - `ToolCallEnd`
   - no-tool completion 路径的 post-response trace phase 已调整为 `response_ready`
     以避免把 response-ready 错误归类为重复的 `turn.model_call_started`
11. 已把多 hop 流式测试扩成 realtime hook trace evidence：
   - `agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream`
   - 现已验证：
     - `turn.model_call_started`
     - `turn.tool_call_started`
     - `turn.tool_call_completed`
     三类事件都可携带对应 boundary 的 `hook_trace_records`
12. 已补前端 read-plane/hydration 回归：
   - `tests/runtime-store.spec.ts`
   - 已新增两条读面验证：
     - backend snapshot reload 会保留 `hookTraceRecords`
     - streamed `turn:trace / turn:tool / turn:completed` 会把 `hookTraceRecords` 写入前端 `turnTraceHistory`
13. 已修复前端 store 的真实缺口：
   - `src/stores/runtime.ts` 之前在 `turn:tool` 事件落库时未把 `payload.hookTraceRecords` 传给 `commitTurnTraceTimeline(...)`
   - 该缺口会导致 tool boundary 的 hook evidence 在前端历史里被上一条 trace 覆盖
   - 现已补齐 `trace / phase_changed / checkpoint_persisted / tool / completed / failed / cancelled` 等事件写面
14. 已补 backend reload / control-plane 读面回归：
   - `agent::session::tests::file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces`
   - `agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics`
   - 已验证 runtime 真实产出的多 boundary hook trace records 可经 file backend roundtrip 保留
   - 已验证 control-plane runtime view 与 monitor drilldown 能读回 hook traces 与 hook aggregates
15. 已把首轮 hook failure 语义收紧到 non-blocking evidence：
   - `dispatch_hook_trace_records(...)` 不再把 hook executor error 直接上抛为 turn failure
   - 默认改为把 executor failure 归一化写成 hook trace evidence
   - 新增测试 `agent::runtime::tests::runtime_hook_dispatch_records_executor_failure_without_stopping_turn_by_default`
   - 当前语义与 spec 首轮 `observe / non-blocking` 口径对齐，但 `FailTurn / Degrade` 的更强矩阵仍未单独实现完毕
16. 已补 `Degrade` 首轮语义与 unstable-boundary guardrail：
   - 新增测试 `agent::runtime::tests::runtime_hook_dispatch_records_degrade_failure_evidence_without_stopping_turn`
   - 已验证 `Degrade` 在本卡首轮仍保持 evidence-first / non-blocking
   - 新增测试 `agent::runtime::tests::start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks`
   - 已验证 runtime 不会为了 hooks 接线人造 `TurnPrepare* / ContextBuild*` boundary dispatch
17. 已补 `FailTurn` 的最小 streamed runtime 闭环：
   - `dispatch_hook_trace_records(...)` 现在会把 `FailTurn` executor failure 标记为 blocked trace evidence
   - `turn:failed` terminal payload 已支持携带 `hookTraceRecords`
   - streamed `ModelCallStart / ToolCallStart / ToolCallEnd / CheckpointPersistEnd / TurnFinalizeEnd` 已接入：
     `FailTurn -> boundary realtime event(hook evidence) -> turn:failed(hook evidence) -> persisted failed trace(hook evidence)`
   - 新增测试 `agent::runtime::tests::start_turn_stream_fail_turn_policy_emits_failed_terminal_with_hook_evidence`
   - 新增测试 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_start_stops_before_tool_execution`
   - 新增测试 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_end_stops_before_followup_model_call`
   - 新增测试 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_checkpoint_boundary_emits_failed_instead_of_completed`
   - 新增测试 `agent::runtime::tests::start_turn_stream_fail_turn_policy_on_finalize_boundary_emits_failed_with_terminal_hook_evidence`
   - 当前 streamed stable-boundary 已全覆盖，非 streamed / sync path 的 `FailTurn` 矩阵待继续铺开
18. 已补 sync `ModelCallStart / ToolCallStart / ToolCallEnd` 的最小 `FailTurn` evidence：
   - `run_turn()` 现在会在 `ModelCallStart` 的 `FailTurn` 下返回带 `hookTraceRecords` 的 failed `TurnResult`
   - `handle_sync_tool_turn()` 现在会在 `ToolCallStart / ToolCallEnd` 的 `FailTurn` 下返回带 `hookTraceRecords` 的 failed `TurnResult`
   - 新增测试 `agent::runtime::tests::run_turn_fail_turn_policy_on_model_call_start_returns_failed_result_with_hook_evidence`
   - 新增测试 `agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_start_returns_failed_before_tool_execution`
   - 新增测试 `agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_end_returns_failed_before_followup_model_call`
19. 已补 sync terminal-boundary completion / failure 闭环：
   - `handle_sync_tool_turn()` 已把累计的 `hook_trace_records` 返回给 `run_turn()`
   - `run_turn()` completed path 已接入
     - `CheckpointPersistEnd`
     - `TurnFinalizeEnd`
   - sync completed `TurnResult` 与 persisted `TurnTraceRecord` 现在都会保留 terminal hook evidence
   - sync attachments failure 已改为持久化带 hooks 的 failed trace，不再只落纯 trace/provider 数据
   - sync `CheckpointPersistEnd / TurnFinalizeEnd` 现已具备
     `FailTurn -> failed TurnResult(hook evidence) + persisted failed trace` 最小闭环
   - 新增测试 `agent::runtime::tests::run_turn_persists_terminal_hook_traces_on_completed_sync_turn`
   - 新增测试 `agent::runtime::tests::run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace`
   - 新增测试 `agent::runtime::tests::run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence`

## 验证

- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib --no-run`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_returns_trace_records_in_priority_order -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_returns_empty_for_unregistered_boundary -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_records_executor_failure_without_stopping_turn_by_default -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::runtime_hook_dispatch_records_degrade_failure_evidence_without_stopping_turn -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_emits_failed_terminal_with_hook_evidence -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_start_stops_before_tool_execution -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_tool_call_end_stops_before_followup_model_call -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_checkpoint_boundary_emits_failed_instead_of_completed -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fail_turn_policy_on_finalize_boundary_emits_failed_with_terminal_hook_evidence -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_fail_turn_policy_on_model_call_start_returns_failed_result_with_hook_evidence -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_start_returns_failed_before_tool_execution -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_fail_turn_policy_on_tool_call_end_returns_failed_before_followup_model_call -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::run_turn_persists_terminal_hook_traces_on_completed_sync_turn -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::session_runtime_view_reads_runtime_generated_hook_traces_and_metrics -- --exact`
  - 结果：通过
- `$env:CARGO_TARGET_DIR='C:\\Users\\HUAWEI\\Documents\\pony-agent\\src-tauri\\target-codex-pa035'; cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::registry_rejects_persisted_effect_without_persistence_requirement -- --exact`
  - 结果：通过
- `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - 结果：通过

## 当前判断

- `PA-033` 的核心工作已经站稳，剩余主要是最终关卡流转
- 下一段真实开发主线应切到 `PA-035`
- `PA-035` 首轮只接稳定 boundary：
  - `ModelCallStart`
  - `ToolCallStart`
  - `ToolCallEnd`
  - `CheckpointPersistEnd`
  - `TurnFinalizeEnd`
- 当前已真实落地并验证到产物链的 boundary：
  - `ModelCallStart`
  - `ToolCallStart`
  - `ToolCallEnd`
  - `CheckpointPersistEnd`
  - `TurnFinalizeEnd`
- 前端 streamed event 与 session reload 读面已能保住 runtime-produced hook trace evidence
- backend file roundtrip 与 control-plane runtime view / monitor drilldown 读面已能保住 runtime-produced hook trace evidence
- hook dispatch 首轮失败语义已改为 evidence-first / non-blocking，避免把 hooks 观测层做成新的 turn 故障源
- `Degrade` 在首轮已被明确验证为 non-blocking evidence 路径
- `FailTurn` 的 streamed stable-boundary 最小闭环已全覆盖，failed terminal path 不再丢 hook evidence
- `FailTurn` 的 sync model/tool/terminal path 最小 evidence 已全部落地，non-streamed 不再只剩 terminal 空白
- prepare/context build 等早期 boundary 已验证不会被 runtime 人造接线，仍保持 contract-only 状态

## 下一步

1. 评估 terminal event metadata 是否需要进一步反写到 persisted trace terminal envelope
2. 评估 control-plane/frontend 是否需要对 failed-trace hook evidence 增补专门聚合断言
3. 评估是否把 `SyncToolTurnOutcome.trace_timeline` 一并收敛，避免继续保留未使用字段
