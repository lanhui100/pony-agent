# 2026-06-04 Session 74

## 主题

- 收紧 `PA-036 / PA-037` 的 spec 边界
- 把 `PA-036` 的 sync terminal envelope 实现进度正式回写到任务系统

## 本轮完成

1. 复用子智能体 `Planck` 完成 `PA-036 / PA-037` 只读 spec 审核
2. 审核共识收敛为：
   - `PA-036` 只做 terminal envelope parity 与 persisted truth-source verification
   - `PA-036` 不扩 hooks evidence model，不新增 lifecycle event contract
   - `PA-037` 只消费既有 runtime/control-plane/history graph 合同
   - `PA-037` 不新增 replay backend command，不重做前端私有状态机
3. 已新增审核记录：
   - [2026-06-04-pa036-pa037-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa036-pa037-spec-review.md)
4. 已回写 `PA-036` 任务卡与 OpenSpec：
   - 收紧输出与验收口径，改为“evidence 保真验证”而非“扩读面”
   - 明确 terminal envelope `eventType` 只复用 `PA-031` canonical terminal event
   - 明确前端遇到缺失 envelope 的 persisted trace 时不得发明 canonical terminal metrics
5. 已回写 `PA-037` 任务卡与 OpenSpec：
   - 收紧 CTA 触发条件到 paused run、recovery checkpoint、non-default submission plan
   - 明确 `historical / historical_dirty / paused / recovery-capable` 的真相源绑定约束
   - 明确 `replay` 只作为文案或入口，不新增后端命令
   - 按 `HomeSessionSidebar` 与 `HomeWorkspace / runtime-store` 拆分验证面
6. 已把 `PA-036` 当前实现进度登记为已完成项：
   - `turn_flow.rs` 新增 `TurnEventEnvelope`
   - `turn_flow.rs` 新增 `build_terminal_turn_event_envelope(...)`
   - `runtime.rs` 已为 sync completed / failed terminal path 注回 canonical terminal envelope
   - 3 条 sync terminal exact 测试已开始直接断言 `eventId / eventType / eventVersion / sequence / emittedAtMs`
7. 已继续把 `PA-036` 的 monitor / frontend 读面收紧到 terminal truth-source：
   - `control_plane.rs` 新增 canonical terminal envelope 判定
   - monitor summary / session metrics / provider-model-tool-hook 聚合现仅统计带 canonical terminal envelope 的 persisted trace
   - raw trace 仍保留在 session drilldown 的 `turn_trace_history` 中，避免丢失原始证据
   - `ModelMonitorPage.vue` 已为缺失 terminal envelope 的 trace 增加 raw-evidence 提示
8. 已补新的验证面：
   - `agent::control_plane::tests::load_model_monitor_summary_aggregates_capability_usage_dimensions`
     - 结果：通过
   - `agent::control_plane::tests::monitor_summary_excludes_raw_traces_without_terminal_envelope_from_canonical_metrics`
     - 结果：通过
   - `npx vitest run tests/ModelMonitorPage.spec.ts --pool=forks --maxWorkers=1`
     - 结果：通过
9. 已补 `session` file-backend failed / cancelled reload 保真回归：
   - `file_backend_roundtrip_restores_failed_terminal_envelope_and_existing_evidence`
   - `file_backend_roundtrip_restores_cancelled_terminal_envelope_and_existing_evidence`
   - 两条回归都直接断言 terminal envelope 与既有 evidence 字段不会在 reload 后丢失
10. 已补 mixed sync / stream terminal truth-source 组合验证：
   - `monitor_summary_aggregates_mixed_sync_and_stream_terminal_truth_from_persisted_traces`
   - 已验证 monitor summary 对 sync-like 与 stream-like persisted trace 使用同一套 canonical aggregation 口径
   - 已验证不会因来源类型不同而分叉 terminal metadata 统计逻辑
11. 已补 failed / cancelled session drilldown evidence 保真验证：
   - `model_monitor_session_drilldown_preserves_failed_and_cancelled_terminal_evidence`
   - 已验证 failed / cancelled trace 在 drilldown 中都能保留 terminal envelope
   - 已验证 provider call record、tool activity、hook trace evidence 不会在 drilldown 读面被截断或丢失
12. 已补前端 runtime-store / monitor terminal envelope 消费回归：
   - `runtime-store` 新增 raw failed trace 与 canonical failed trace 两条对照回归
   - 已验证缺失 terminal envelope 的 raw trace 不会被恢复成 canonical failed runtime phase
   - 已验证带 canonical terminal envelope 的 failed trace 仍会恢复成 failed runtime phase
   - `ModelMonitorPage.spec.ts` 已重新通过，raw trace warning 仍与 store 语义一致

## 验证

- 既有验证结果沿用本轮前状态：
  - `cargo test --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target-codex-pa036 --lib --no-run`
    - 结果：通过
  - `agent::runtime::tests::run_turn_persists_terminal_hook_traces_on_completed_sync_turn`
    - 结果：通过
  - `agent::runtime::tests::run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace`
    - 结果：通过
  - `agent::runtime::tests::run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence`
    - 结果：通过
- 本轮新增验证：
  - `agent::control_plane::tests::load_model_monitor_summary_aggregates_capability_usage_dimensions`
    - 结果：通过
  - `agent::control_plane::tests::monitor_summary_excludes_raw_traces_without_terminal_envelope_from_canonical_metrics`
    - 结果：通过
  - `npx vitest run tests/ModelMonitorPage.spec.ts --pool=forks --maxWorkers=1`
    - 结果：通过
  - `agent::session::tests::file_backend_roundtrip_restores_failed_terminal_envelope_and_existing_evidence`
    - 结果：通过
  - `agent::session::tests::file_backend_roundtrip_restores_cancelled_terminal_envelope_and_existing_evidence`
    - 结果：通过
  - `agent::control_plane::tests::monitor_summary_aggregates_mixed_sync_and_stream_terminal_truth_from_persisted_traces`
    - 结果：通过
  - `agent::control_plane::tests::model_monitor_session_drilldown_preserves_failed_and_cancelled_terminal_evidence`
    - 结果：通过
  - `npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1`
    - 结果：通过
  - `npx vitest run tests/ModelMonitorPage.spec.ts --pool=forks --maxWorkers=1`
    - 结果：通过

## 当前判断

- `PA-036` 的近线实现已经从“只有 streamed 路径有 canonical terminal annotation”推进到“sync terminal path 也有最小 envelope 闭环”
- `PA-036` closeout 结论已明确：
  - sync `run_turn()` 路径只存在 `completed / failed`
  - canonical `cancelled` 只存在于 streamed turn + execution control
  - 因此 `2.2` 已收窄为“sync failed 与 streamed cancelled persisted trace”，属于文案对齐，不是遗漏实现
- `PA-036` 已具备 acceptance audit，可以转入 `Review`
- `PA-037` 现阶段已成为下一条实现主线，适合直接进入前端 session 控制交互与反馈闭环开发
