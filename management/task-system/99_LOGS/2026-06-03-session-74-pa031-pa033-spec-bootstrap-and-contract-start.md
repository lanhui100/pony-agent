# 2026-06-03 Session 74

## 主题

- `PA-031` turn lifecycle / event contract 启动
- `PA-032` trace persistence / recovery contract 建档
- `PA-033` hooks foundation 建档与 spec 审核

## 本轮完成

1. 建立架构母文档：
   [turn-lifecycle-hooks-and-recovery.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/turn-lifecycle-hooks-and-recovery.md)
2. 新建 3 个 OpenSpec change：
   - [add-turn-lifecycle-event-contract](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-turn-lifecycle-event-contract)
   - [add-trace-persistence-and-recovery-contract](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-trace-persistence-and-recovery-contract)
   - [add-agent-hooks-pipeline-foundation](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-agent-hooks-pipeline-foundation)
3. 新建任务卡：
   - [PA-031](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-031-define-turn-lifecycle-and-event-contract.md>)
   - [PA-032](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-032-stabilize-trace-persistence-and-recovery-contract.md>)
   - [PA-033](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-033-build-agent-hooks-pipeline-foundation.md>)
4. 更新 dashboard / task board / docs index
5. 委派 2 个子智能体独立审核 spec，并采纳意见回写：
   [2026-06-03-pa031-pa032-pa033-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-03-pa031-pa032-pa033-spec-review.md)
6. 启动 `PA-031` 第一轮实现：
   - Rust `TurnStreamEvent` 新增 `eventType / eventVersion / sequence / emittedAtMs`
   - TS `TurnStreamEvent` 与 lifecycle/event type 开始承接 canonical 合同
   - SSE 测试补 canonical 事件元数据断言
7. 推进 `PA-031` 第二刀最小落地：
   - Rust `emit_event(...)` 统一生成 canonical `eventType / eventVersion / sequence / emittedAtMs`
   - terminal 事件后清理 turn 级 sequence registry
   - 前端 `runtime store` 开始用 canonical event metadata 做 phase 归一化
   - session restore / checkpoint restore 开始承接 canonical phase
8. 推进 `PA-031` 第三刀 contract 收口：
   - Rust / TS `TurnStreamEvent` 正式补齐 `eventId / sessionId`
   - SSE `id:` 从 turn 级别收口到 event 级别，优先使用 `eventId`
   - `cargo test --lib sse_adapter::tests::format_sse_event_uses_standard_event_id_and_data_lines -- --exact` 已拿到通过结果
9. 推进 `PA-031` 前端 fallback 收口：
   - `runtime store` 的 fallback trace timeline 已开始基于 canonical `eventType / phase` 识别 active hop
   - `turn:started / turn:delta / turn:trace / turn:tool / turn:completed` 的本地 trace 预测已减少对旧 phase 的直接猜测
   - 新增前端单测验证 canonical 事件元数据下的 timeline 状态流转
10. 启动 `PA-032` 第一刀：
   - `ExecutionCheckpoint` 读面已显式暴露 `checkpointKind / resumable / replayable`
   - 当前运行中 checkpoint 明确标记为 `runtime_control`
   - 前端 `ExecutionCheckpoint` 类型与测试工厂已同步新字段，避免前端继续猜测 recovery 能力

## 已采纳的审核意见

- 统一 `streaming_response` 为 canonical phase
- 明确 `PA-031` 只管 lifecycle truth，`PA-032` 只管 persisted trace/recovery truth
- 明确 `checkpointing` 不单独承诺 recovery checkpoint
- 给 `PA-031` 补一级事件名集合的 MUST 级约束
- 给 `PA-032` 收紧 fallback 的允许场景
- 明确 `PA-033` 与 `PA-022` 的范围关系
- 收紧 hooks 合同，显式禁止其演化成新调度层

## 验证

- `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - 结果：通过，`42 passed`
- `cargo check --manifest-path src-tauri/Cargo.toml --tests`
  - 结果：通过
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml format_sse_event_uses_standard_event_id_and_data_lines -- --exact`
  - 结果：最初超时；后续改用 `cargo test --manifest-path src-tauri/Cargo.toml --lib sse_adapter::tests::format_sse_event_uses_standard_event_id_and_data_lines -- --exact`，已通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib stop_turn_and_checkpoint_queries_share_same_registry_surface`
  - 结果：通过

## 下一步

1. 继续 `PA-031`：
   - 继续压缩前端 fallback timeline 对旧 `calling_model / calling_tool` 推导的依赖
   - 决定前端是否开始消费 `eventId / sessionId / sequence` 做 timeline 去重、排序与重放保护
   - 决定 `RuntimePhase` 是否继续保留为 UI 投影层，还是进一步向 canonical turn phase 收敛
   - 让 trace timeline / session restore 更稳定地消费 canonical event metadata
2. 在 `PA-031` 稳定后启动 `PA-032` 的 persistence / recovery 实装
   - 当前已进入第一刀：先收 checkpoint 语义显式化
3. 暂不启动 `PA-033` 代码实现，先等待 lifecycle 与 recovery 语义更稳定

## 风险与提醒

- 当前 Rust 精确单测执行较重，单独 `cargo test ... --exact` 在本机上可能超时；短期内以 `cargo check --tests` + 前端定向测试作为第一轮保护
- 虽然 `sequence` 已有正式生成逻辑，但前端目前还没有把它用作 timeline 去重 / 排序 / 重放保护；这仍是后续重点
- `eventId / sessionId` 已进入事件合同，但 trace persistence / recovery 读面还没有完全把它们作为 roundtrip 验证锚点；这会进入 `PA-032`
- 当前 `PA-032` 还只完成了 runtime checkpoint 显式化，真正的 recovery-capable checkpoint 与 reload roundtrip 还没落地

## 2026-06-03 后续追记

11. 推进 `PA-032` 第二刀：
   - persisted `TurnTraceRecord` 已显式补齐 `sessionId / eventId / eventType / eventVersion / sequence / emittedAtMs`
   - terminal lifecycle event metadata 已通过 `control_plane -> runtime -> session store` 回写到已持久化 trace
   - Rust roundtrip 测试已覆盖“先持久化 trace，再回写 terminal event 锚点，reload 后仍完整保留”
12. 推进 `PA-032` 第三刀：
   - 统一 `load_execution_checkpoint(...)` 已能在缺少 runtime turn checkpoint 时回退投影 graph run recovery checkpoint
   - `checkpointKind = recovery` 与 `resumable / replayable` 已进入统一 checkpoint 读面
13. 推进 `PA-032` 第四刀：
   - 前端 `runtime store` 已开始区分 `runtime_control` 与 `recovery`
   - `recovery checkpoint` 初始化后不再被误判为正在提交中的 turn，而是恢复为可继续的 ready-state / run-state
14. 推进 `PA-032` 第五刀：
   - 前端 `runtime store` 已显式保存最近一次 `ExecutionCheckpoint`，作为 session hydration 后的恢复决策兜底输入
   - `submitTurn()` 在 `retrievedContext.runState` 缺失时，已可基于 `recovery checkpoint + activeRunId` 正确选择 `resume_graph_run_stream`
   - 新增前端回归测试，覆盖“runState 缺失但 recovery checkpoint 仍可触发 resume”的路径
15. 推进 `PA-032` 第六刀：
   - `HistoryCheckoutResult / HistoryRestoreResult` 已统一显式暴露 transcript/workspace 双维恢复结果字段
   - 后端 `checkout_history_node / restore_branch_head` 已返回 `transcriptRestoreApplied / workspaceRestoreCapable / workspaceRestoreApplied / degradedToTranscriptOnly / degradationReason`
   - 前端 `runtime store` fallback 与测试已对齐同一套结果合同，避免 restore UI 继续猜测 workspace rollback 是否生效
16. 推进 `PA-032` 第七刀：
   - 前端 `runtime store` 在消费 turn lifecycle 事件时，已将 canonical `eventId / eventType / eventVersion / sequence / emittedAtMs` 回写到 `turnTraceHistory`
   - 运行中 trace 与 reload 后 persisted trace 现在共享同一组 terminal lifecycle recovery 锚点，不再只在后端 roundtrip 路径中保真
   - 新增前端回归测试，覆盖 canonical event metadata 会随 terminal event 落进 persisted turn trace
17. 推进 `PA-032` 第八刀：
   - 前端 `runtime store` 已基于 `eventId / sequence / emittedAtMs` 为单 turn 建立事件游标
   - 重复事件与低序号乱序事件不再覆盖较新的 canonical trace / tool state / assistant message
   - 新增前端回归测试，覆盖“先接收较新事件，再收到旧序号 delta/重复 tool 事件时保持 trace 不回退”
18. 推进 `PA-032` 第九刀：
   - `ExecutionCheckpoint` 已显式暴露 `recoveryMode`
   - `runtime_control` checkpoint 现在明确标记为 `replay_required`
   - graph run 投影出的 `recovery` checkpoint 现在明确标记为 `persisted_effect`
19. 推进 `PA-032` 第十刀：
   - 前端 `runtime store` 已将 `recoveryMode` 接入 submission 决策
   - `persisted_effect` recovery checkpoint 允许自动走 `resume_graph_run_stream`
   - `replay_required` recovery checkpoint 不再误走 resume，而是回退到新开 graph run
20. 推进 `PA-032` 第十一刀：
   - 当前端 `retrievedContext.runState` 与 recovery checkpoint 合同冲突时，submission 决策现在以 checkpoint 的 `recoveryMode` 为准
   - `paused` runState 不再能覆盖 `replay_required` checkpoint 去误触发 resume
   - 新增前端回归测试，覆盖“runState 要求 resume，但 checkpoint 要求 replay 时，最终走 start_graph_run_stream”
21. 推进 `PA-032` 第十二刀：
   - graph run 投影出的 `recovery` checkpoint 不再把 `replayable` 错误绑定到 `resumable`
   - `replay_required` recovery checkpoint 现在仍会显式暴露 `replayable=true`
   - 新增后端单测，覆盖“non-resumable recovery checkpoint 仍必须暴露 replayable contract”
22. 推进 `PA-032` 第十三刀：
   - `load_execution_checkpoint(session_id=...)` 已补齐 session 级查询优先级仲裁
   - 运行中的 turn 现在优先暴露 `runtime_control`；当 graph run 进入可恢复边界后，session 级查询会切换为 graph-projected `recovery`
   - 新增后端单测，覆盖“同一 session 查询从 runtime_control 切到 recovery”的合同
23. 推进 `PA-031 / PA-032` 交界面的 recovery/lifecycle 消费合同后端化：
   - 后端 `ExecutionCheckpoint` 已显式补齐 `contractVersion / runId / projectedRuntimePhase / submissionCommand`
   - 前端 `runtime store` 现在优先消费后端投影出的 phase 与 submission command，仅在缺字段时才回退到本地兼容推断
   - recovery checkpoint 在 `runState` 缺失场景下不再必须依赖前端 `activeRunId + recoveryMode + phase` 手工拼装恢复命令
   - 新增后端单测，覆盖 recovery checkpoint projection 会稳定产出 `submissionCommand / projectedRuntimePhase`
24. 推进 `PA-032` 的 submission plan 第二刀：
   - 后端已新增 `resolve_graph_run_submission_plan`，在 checkpoint 缺席或不足时统一仲裁 `start_graph_run_stream / resume_graph_run_stream / continue_graph_run_stream`
   - 前端 `submitTurn()` 现在已改为后端 submission plan 优先；仅当后端计划不可用时，才回退到本地 `runState / checkpoint` 兼容路径
   - 新增后端单测，分别覆盖“缺 checkpoint 时回退 graph_run source”与“存在 recovery checkpoint 时优先采用 checkpoint source”
   - 新增前端回归测试，覆盖“本地 stale runState 与后端 plan 冲突时，以后端 plan 为准”
25. 启动 `PA-033` 第一轮 no-op skeleton：
   - 新增 `src-tauri/src/agent/hooks.rs`
   - 已落第一版 hook contract types、descriptor 校验、registry 与 noop executor
   - 当前仍未把 hooks 接入 runtime 执行链，保持 foundation-only 范围
   - 已新增 3 条精确 Rust 单测，覆盖重复注册拒绝、failure policy 校验与 noop observe 执行归一化
26. 推进 `PA-033` 第二轮 foundation 收口：
   - 已将易与 context/history/task 语义冲突的 `HookBoundary` 收敛为 `TurnHookPoint`
   - 已新增 `HookTraceRecord`，把 no-op foundation 的 traceability 读面明确成正式结构
   - 已新增精确 Rust 单测，覆盖执行结果可稳定投影到 trace record
27. 推进 `PA-033` 第三轮 foundation 收口：
   - `hooks.rs` 已新增 `HookStructuredResult`，把 no-op foundation 的结果从“枚举 + 文本摘要”升级为结构化合同
   - 已新增 `HookDenyDecision / HookPatchOperation / HookSideEffectRequest`，分别承接 guard deny、transform patch、side-effect request 的最小语义
   - descriptor 校验现在会拒绝与 `HookClass` 不兼容的 `allowed_result_kinds`，避免 hooks 合同向万能回调漂移
   - hooks foundation 仍未接入 runtime 主执行链，继续保持 contract-first / skeleton-only 边界
28. 推进 `PA-033` 第四轮 foundation 收口：
   - `AgentHookDescriptor` 已补 `timeoutMs / replayRequirements / sideEffectPersistenceRequirements`
   - `HookExecutionResult / HookTraceRecord` 已补 `hookOrder / inputSummary / persistenceEvidenceRef`，为后续 recovery 与 trace 持久化留出最小证据位
   - descriptor 校验已新增 `timeoutMs > 0` 与 `persisted_effect 必须要求 persistence evidence` 两条硬约束
   - 已新增稳定顺序与 patch conflict handling 的最小 foundation：`priority`、`list_for_hook_point(...)`、`merge_patch_results(...)`
29. 新增子智能体只读审计：
   - 审计目标：识别 `PA-033` 离安全接入 runtime 还差哪些 contract gap
   - 已采纳结论：优先补 `timeout/recovery evidence`；后续再补 `guard deny` 与 hook trace 持久化读面
30. 推进 `PA-033` 第五轮 foundation 收口：
   - 已新增 `HookExecutionResult::guard_decision(...)`，让 guard deny 不再只是 spec 概念，而是正式的结构化结果构造入口
   - descriptor 校验已新增“`deny` 能力必须 `canBlock=true`”硬约束
   - deny reason 现在可稳定投影到 `HookTraceRecord`，为后续 runtime 阻断与 trace/audit 对齐铺路
31. 推进 `PA-033` 第六轮 foundation 收口：
   - `TurnTraceRecord` 已正式新增 `hook_trace_records`
   - `TurnStreamEvent / TurnResult / RecordingTurnEventSink / SessionSnapshot / 前端 TurnTraceRecord` 已统一承接同一字段合同
   - `runtime store` 已新增 hook trace clone/hydration 路径，为后续前端 trace 读面展示 hooks 留出正式承载位
   - 当前 runtime 仍不会主动发 hook trace events，本轮目标仅是打通 persisted trace roundtrip 通道
32. 推进 `PA-033` 第七轮 foundation 收口：
   - `hooks.rs` 已新增 `CanonicalTurnPhase / CanonicalTurnEventType / HookLifecycleBinding`
   - `TurnHookPoint` 现在已正式绑定 canonical `eventType + phase`，不再只是独立枚举名
   - 已新增 `hook_point_matches_canonical_boundary(...)`，为后续 runtime 接 hooks 时统一判断 boundary 对齐关系
   - 已补防漂移单测，覆盖 `ModelResponseEnd -> streaming_response`、`ToolCallEnd -> tool_result_integrating`、`TurnFinalizeEnd -> terminal events/phases`
33. 新增子智能体只读审计：
   - 审计目标：收敛 `TurnHookPoint -> canonical lifecycle/event` 的最小绑定表
   - 已采纳主结论：hook point 必须绑定 canonical vocabulary，不再让 runtime 通过 payload 形状猜语义
   - 未采纳之处：`ToolCallEnd` 仍保持对齐 `tool_result_integrating`，因为这与母文档里的整合边界更一致
34. 推进 `PA-033` 第八轮 foundation 收口：
   - `turn_flow.rs` 已新增交叉测试，直接校验当前 canonical event 发射逻辑与 hooks binding 是否一致
   - 已覆盖 `ContextBuildEnd / ModelCallStart / ModelResponseEnd / ToolCallEnd / TurnFinalizeEnd` 五类边界
   - 这一步让 hooks foundation 不再只依赖静态映射表，而是开始与真实事件发射面建立验证关系
35. 推进 `PA-033 / PA-031` 交界面的 runtime 对齐：
   - `runtime.rs` 的 `turn:completed` event payload phase 已从历史 `ready` 收敛为 canonical `completed`
   - runtime failed / cancelled 路径已新增 hooks binding 精确断言
   - runtime tool-hop 路径已新增 hooks binding 断言
36. 修复测试宿主回归：
   - `src-tauri/tests/session_regression.rs` 的简化 `agent` 模块已补 `hooks.rs`
   - `cargo check --tests` 现已重新通过，说明 `session.rs` 新增 `hook_trace_records` 不再打穿外部测试编译目标

## 追加验证

- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::file_backend_roundtrip_restores_terminal_event_annotation_after_trace_update -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::file_backend_roundtrip_restores_turn_trace_history -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::recording_turn_event_sink_uses_fallback_summary_when_terminal_event_has_none -- --exact`
  - 结果：通过
- `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - 结果：通过，最新为 `45 passed`
- `cargo check --manifest-path src-tauri/Cargo.toml --tests`
  - 结果：通过
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests`
  - 结果：通过，最新为 `19 passed`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::turn_flow::tests -- --nocapture`
  - 结果：通过，`5 passed`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests -- --nocapture`
  - 结果：通过，`19 passed`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_uses_sink_for_empty_input_failure -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan -- --exact`
  - 结果：通过
- `cargo check --manifest-path src-tauri/Cargo.toml --tests`
  - 结果：通过
  - 备注：本机仍有多次 incremental `os error 5` warning
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_measures_first_token_latency_from_turn_start_across_tool_hops -- --exact`
  - 结果：未拿到逻辑结论；连续受 Windows `link.exe` `LNK1104` 文件锁影响
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_uses_compat_sync_for_deepseek_tool_followup -- --exact`
  - 结果：未拿到逻辑结论；连续受 Windows `link.exe` `LNK1104` 文件锁影响
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::non_resumable_recovery_checkpoint_still_exposes_replayable_contract -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::history_commands_and_runtime_view_follow_persisted_history_graph -- --exact`
  - 结果：通过
- `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - 结果：通过，最新为 `46 passed`
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::stop_turn_and_checkpoint_queries_share_same_registry_surface -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::graph_run_can_stop_resume_and_expose_checkpoint -- --exact`
  - 结果：通过
- `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - 结果：通过，最新为 `48 passed`
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::session_checkpoint_query_switches_from_runtime_control_to_recovery_after_turn_boundary -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::non_resumable_recovery_checkpoint_still_exposes_replayable_contract -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::execution_control::tests::recovery_checkpoint_projection_exposes_submission_command_and_projected_phase -- --exact`
  - 结果：通过
- `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - 结果：通过，最新为 `49 passed`
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::submission_plan_falls_back_to_graph_run_when_checkpoint_is_absent -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::submission_plan_prefers_checkpoint_projection_when_available -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::registry_rejects_duplicate_hook_names -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::registry_rejects_descriptor_without_allowed_failure_policy_match -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::noop_executor_normalizes_observe_hook_result -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::execution_result_can_be_projected_to_trace_record -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::registry_rejects_result_kind_incompatible_with_hook_class -- --exact`
  - 结果：因并行 `cargo test` 争抢产物目录而超时；后续改为串行模块测试统一覆盖
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::noop_executor_normalizes_transform_hook_to_structured_patch -- --exact`
  - 结果：因并行 `cargo test` 争抢产物目录而超时；后续改为串行模块测试统一覆盖
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::noop_executor_normalizes_side_effect_hook_to_persisted_request -- --exact`
  - 结果：因并行 `cargo test` 争抢产物目录而超时；后续改为串行模块测试统一覆盖
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests`
  - 结果：第一次通过，`7 passed`
  - 备注：测试结束后仍有本机常见的 incremental `os error 5` warning，但不影响本轮 hooks 单测结论
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests`
  - 结果：第二次通过，`10 passed`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests`
  - 结果：第三次通过，`12 passed`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests`
  - 结果：第四次通过，`15 passed`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::file_backend_roundtrip_restores_turn_trace_history -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::file_backend_roundtrip_restores_terminal_event_annotation_after_trace_update -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib sse_adapter::tests::format_sse_event_uses_standard_event_id_and_data_lines -- --exact`
  - 结果：通过
- `npx vue-tsc --noEmit`
  - 结果：通过
