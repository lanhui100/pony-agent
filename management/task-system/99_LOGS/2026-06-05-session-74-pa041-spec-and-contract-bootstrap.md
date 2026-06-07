# 2026-06-05 Session 74

## 主题

- 启动 `PA-041`：history-state hooks 与 restore-boundary contract

## 本轮完成

1. 完成新卡立项与任务系统接入
   - 新建 [PA-041 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-041-build-history-state-hooks-and-restore-boundaries.md)
   - 新建 OpenSpec change：`add-history-state-hooks-and-restore-boundaries`
   - 已补 proposal / design / spec / tasks
2. 完成独立 spec 审核与采纳修订
   - 子智能体 `Raman` 已完成只读 spec 审阅
   - 已回写 [2026-06-05-pa041-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa041-spec-review.md)
   - 采纳重点：
     - history-state evidence 只作为 audit chain，不得成为 restore / submission / cursor 仲裁输入
     - 缺少 hooks evidence 时，reload 不能据此重建 restore 结论
     - control-plane 与 runtime view 必须投影同一口径 evidence
     - 增加 source-of-truth non-regression test 任务
3. 启动第一段 contract/scaffolding 实现
   - `src-tauri/src/agent/hooks.rs` 已新增：
     - `HistoryStateHookPoint`
     - `HistoryStateCommandKind`
     - `HistoryStateCursorSummary`
     - `HistoryStateHookEnvelope`
     - `HistoryStateHookEvidence`
     - `HistoryStateHookExecutor / NoopHistoryStateHookExecutor`
   - 已新增最小测试：
     - `agent::hooks::tests::noop_history_state_executor_returns_empty_results`
4. 打通第一条 session-level history-state hook 闭环
   - `src-tauri/src/agent/session.rs` 已新增 `history_state_evidence` 的 state/snapshot 持久化字段
   - `SessionStore` 已接入 `HistoryStateHookExecutor`
   - `checkout_history_node(...)` 已在 `history.checkout.start / history.checkout.resolved` 调度 hooks 并持久化 evidence
   - blocked checkout 会只保留 start evidence，且不会改写既有 history cursor/live truth-source
5. 补齐第一组定向回归测试
   - `agent::session::tests::checkout_history_node_persists_history_state_hook_evidence`
   - `agent::session::tests::checkout_history_node_blocked_by_hook_persists_only_start_evidence`
   - `agent::session::tests::history_state_hook_evidence_roundtrip_through_file_backend`
6. 打通第一条 control-plane/runtime-view 读面投影
   - `HistoryCheckoutResponse` 已显式暴露 `history_state_evidence`
   - `SessionRuntimeView` 已显式暴露 `history_state_evidence`
   - 新增一致性测试：
     - `agent::control_plane::tests::history_checkout_response_and_runtime_view_share_same_history_state_evidence_projection`
7. 把同一套 history-state contract 扩到其余三条命令
   - `restore_branch_head(...)` 已接入 `history.branch_restore.start / resolved`
   - `fork_from_history_node(...)` 已接入 `history.branch_fork.start / resolved`
   - `switch_history_branch(...)` 已接入 `history.branch_switch.start / resolved`
   - 三条命令在被 guard 阻断时都保持既有 truth-source，不让 evidence 反向决定 cursor/restore 结果
8. 把其余 command response 读面投影接到 control plane
   - `RestoreBranchHeadResponse`
   - `ForkFromHistoryNodeResponse`
   - `SwitchHistoryBranchResponse`
   - 这些响应与 runtime view 一样，都直接投影 persisted audit chain
9. 补第二组定向回归测试
   - `agent::session::tests::restore_branch_head_persists_history_state_hook_evidence`
   - `agent::session::tests::fork_from_history_node_blocked_by_hook_persists_only_start_evidence`
   - `agent::session::tests::switch_history_branch_persists_history_state_hook_evidence`
   - `agent::control_plane::tests::history_restore_fork_switch_responses_project_history_state_evidence`
10. 补真相源 non-regression 与缺失 evidence 负向场景
   - `agent::session::tests::checkout_history_node_preserves_degraded_truth_source_with_hooks`
   - `agent::session::tests::missing_history_state_evidence_does_not_reconstruct_restore_conclusion_after_reload`
   - `agent::control_plane::tests::history_checkout_degrade_truth_source_does_not_depend_on_hooks_evidence`
   - 这组测试确认：
     - hooks 存在时 `degraded_to_transcript_only / workspace_rollback_applied / history cursor` 仍只由既有合同决定
     - 清空 hooks evidence 后 reload 只表现为“无 hooks evidence”，不会倒推出 restore/cursor/degrade 结论
11. 补前后端 history-state contract 对齐
   - `src/types/runtime.ts` 已补 `HistoryStateHookEvidence` 以及 `SessionSnapshot / SessionRuntimeView` 对应字段
   - `src/stores/runtime.ts` 已把 tauri 返回的 history-control 响应规范化为前端可消费形态，避免 `cursor` 嵌套结果把 `historyNodes/historyBranches` 清空
   - `src/components/HomeSessionSidebar.vue` 已优先消费新 contract，并兼容旧测试桩字段
   - 前端验证通过：
     - `npx vue-tsc --noEmit`
     - `npm run test:ui-guard`

## 验证

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::noop_history_state_executor_returns_empty_results -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::checkout_history_node_persists_history_state_hook_evidence -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::checkout_history_node_blocked_by_hook_persists_only_start_evidence -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::history_state_hook_evidence_roundtrip_through_file_backend -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::restore_branch_head_persists_history_state_hook_evidence -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::fork_from_history_node_blocked_by_hook_persists_only_start_evidence -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::switch_history_branch_persists_history_state_hook_evidence -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::checkout_history_node_preserves_degraded_truth_source_with_hooks -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::missing_history_state_evidence_does_not_reconstruct_restore_conclusion_after_reload -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::history_checkout_response_and_runtime_view_share_same_history_state_evidence_projection -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::history_restore_fork_switch_responses_project_history_state_evidence -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::history_checkout_degrade_truth_source_does_not_depend_on_hooks_evidence -- --exact --nocapture
npx vue-tsc --noEmit
npm run test:ui-guard
cargo check --manifest-path src-tauri/Cargo.toml --lib
```

结果：

- 新增 hooks 基线测试通过
- `checkout` history-state hooks 的 success / blocked / reload 三条最小回归路径通过
- `checkout` 的 control response / runtime view evidence 投影一致性通过
- `restore / fork / switch` 的 session command 路径与 control response evidence 投影通过
- `degraded_to_transcript_only` 真相源与“缺失 evidence 不重建 restore 结论”两条 guardrail 已通过定向回归
- 前端 runtime store / sidebar 已对齐新的 history-state contract，UI guard 与 TS 检查通过
- `cargo check --lib` 通过
- 仍有 Windows incremental finalize `os error 5` warning，但不影响通过判定

## 当前判断

- `PA-041` 的下一步不该直接把整条 history-state hooks 接到底，而应先在 `session / control_plane` 收敛最小 dispatch / evidence 闭环
- 当前最关键的 guardrail 已在 spec 层写清：history-state hooks evidence 不能长成第二 restore truth-source
- 当前 `checkout / restore / fork / switch` 四条命令都已经接入同一套 contract，且 degrade/non-regression guardrail 已有测试证据；剩余重点转向 host surface 收口与前端联调形态
- 当前前后端对 history-state evidence 的基础 contract 已收口，剩余重点转向是否需要单独 summary DTO 与更广 host surface 审查
