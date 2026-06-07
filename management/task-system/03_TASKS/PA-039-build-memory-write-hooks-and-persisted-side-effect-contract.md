# PA-039 构建 memory-write hooks 与 persisted side-effect contract

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## OpenSpec Change
- [add-memory-write-hooks-and-persisted-side-effect-contract](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-memory-write-hooks-and-persisted-side-effect-contract)

## Delta Spec
- [memory-write-hooks-and-persisted-side-effect-contract/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-memory-write-hooks-and-persisted-side-effect-contract/specs/memory-write-hooks-and-persisted-side-effect-contract/spec.md)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把 hooks 扩展到 memory write 与 persisted side-effect 这条高风险路径，建立“写入意图 -> hook 决策 -> 持久化证据 -> recovery/replay 口径”的正式合同，让后续记忆写入和副作用扩展都不再依赖隐式 store mutation。

## 输出
- memory-write hook point 与 normalized write intent
- persisted-effect / replay-required side-effect contract
- hook trace / persistence evidence / recovery 读面闭环
- 与 `PA-018 / PA-032 / PA-033` 的边界说明
- 针对 write guard、write transform、persisted effect 的验收矩阵

## 验收标准
- hooks 只能消费规范化 memory-write intent，而不是直接操作 session store
- `persisted_effect / replay_required` 的最小证据要求明确且可验证
- 当持久化证据不完整时，系统必须稳定降级到 `replay_required`，不得由实现或前端猜测恢复结论
- 当前阶段至少要求 `long-term memory write` 路径的 hook 结果能进入 session truth-source，并以 `source_history_node_id` 向 recovery 判定链投影 `persisted_effect / replay_required` evidence
- reload 后仍能读回 hook 证据、写入结果与 replay 决策依据
- 测试覆盖 `long-term memory write` 路径上的 allow/deny、transform、persisted-effect、reload 与 replay-required 五类路径；通用 side-effect 扩展不作为本卡关闭前置

## 当前进展
- `PA-018` 已稳定 retrieval / long-term memory 边界
- `PA-032` 已稳定 recovery / replay contract
- `PA-033` 已给出 hooks structured result 与 persistence requirements 基线
- `PA-039` 负责把这些底座接到 memory write 与 side-effect 合同上
- 已完成一轮独立 spec 审核并采纳修订，见：
  [2026-06-04-pa038-pa039-pa040-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa038-pa039-pa040-spec-review.md)
- 已完成一轮只读实现入口勘探：
  - 首个真实 truth-source 入口优先落在 `session.rs` 的 `update_long_term_memory_from_user_message(...)`
  - persisted effect / reload / recovery 投影优先沿 `SessionSnapshot.long_term_memory_entries -> FileSessionBackend roundtrip -> control_plane recovery/read-plane` 这条链路收口
  - hooks 契约层建议先补 `MemoryWriteIntent / MemoryWriteHookPoint / PersistedEffectEvidence`，而不是直接改 `SessionStore` 内部结构
- 已完成第一轮真实接线：
  - `hooks.rs` 已新增 `MemoryWriteHookPoint / MemoryWriteTarget / MemoryWriteOperation / MemoryWriteIntentRecord / MemoryWriteHookEnvelope / PersistedEffectEvidence`
  - `session.rs` 已在 `update_long_term_memory_from_user_message(...)` 接入 `planned write -> hook envelope -> persisted evidence -> long_term_memory_entries mutation` 最小闭环
  - `SessionState / SessionSnapshot / HistoryNode` 已新增 `memory_write_evidence`，并完成 snapshot / hydrate / roundtrip 串联
- memory-write hook 控制链已接入真实写入路径：
  - `SessionStore` 已新增可插拔 `MemoryWriteHookExecutor`，默认无 hooks，测试中可注入 executor
  - `MemoryWriteIntentRecord` 已补 `kind / content` 真值字段，`HookPatchOperation` 已补 `value_text`，transform 不再只能停留在 summary
  - guard `deny` 现在可以真实阻止 memory write 与 evidence 持久化；transform `patch` 现在可以真实改写待持久化 memory intent
- memory-write hook traceability / reload / read-plane 已接入 session truth-source：
  - `SessionState / SessionSnapshot / HistoryNode` 已新增 `memory_write_hook_trace_records`
  - `update_long_term_memory_from_user_message(...)` 现在会把 `MemoryWriteHookExecutor` 返回的结果投影为 `HookTraceRecord` 并持久化
  - deny / transform 的 hook 决策不再只存在于运行时，而是能随 session snapshot、history checkout、FileSessionBackend roundtrip 一起读回
- control-plane 读面已完成第一轮投影：
  - `ExecutionCheckpoint` 已新增 `persisted_effect_evidence`
  - `load_execution_checkpoint(...)` 会把 session 内的 `memory_write_evidence` 附带投影到 checkpoint / runtime view
  - `PersistedEffectEvidence` 已新增 `source_history_node_id`，memory-write evidence 现在具备已持久化历史节点锚点
  - lifecycle boundary checkpoint 现在会按 `source_history_node_id` 过滤与当前恢复边界相关的 evidence，并据此收敛到 `persisted_effect` 或 `replay_required`
  - 当前仍未做到 per-turn 精确锚定，现阶段已从 session 级可见性推进到 history-node 级恢复判定
- 已拿到第一批后端验证证据：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::append_turn_persists_memory_write_evidence_for_explicit_note -- --exact --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_evidence_roundtrip_through_store -- --exact --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_guard_deny_blocks_persistence_and_memory_mutation -- --exact --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_transform_patch_can_rewrite_persisted_memory_intent -- --exact --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::memory_write_hook_trace_records_roundtrip_through_store -- --exact --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::history_checkout_restores_memory_write_hook_trace_records_from_selected_node -- --exact --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_projects_memory_write_evidence_from_session_snapshot -- --exact --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_keeps_replay_required_when_latest_node_has_no_memory_write_evidence -- --exact --nocapture`
  - `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet`
  - `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - 当前新增验证已确认：memory-write hook trace records 能在 deny / transform / reload / history checkout 路径上保持可追踪与可读回；checkpoint 侧仍主要消费 `memory_write_evidence`
- 已完成 acceptance audit 与 closeout，见：
  [2026-06-05-pa039-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa039-acceptance-audit.md)
- 当前卡的实现缺口已全部关闭：
  - `long-term memory write` 的 allow / deny / transform / persisted_effect / replay_required 合同已形成真实闭环
  - `memory_write_evidence` 与 `memory_write_hook_trace_records` 均可随 session snapshot / history checkout / file roundtrip 读回
  - recovery / checkpoint 判定链已能基于 `source_history_node_id` 投影 persisted-effect 与 replay-required evidence

## 下一步动作
- 本卡已完成 closeout；后续若要扩到通用 side-effect family、`replace_long_term_memory(...)` 或更细粒度 runtime-view 投影，应另开新卡承接

## 当前卡点
- 无；本卡已完成

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`
- `management/task-system/03_TASKS/PA-032-stabilize-trace-persistence-and-recovery-contract.md`
- `management/task-system/03_TASKS/PA-033-build-agent-hooks-pipeline-foundation.md`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/context.rs`
- `src-tauri/src/agent/control_plane.rs`
