# PA-041 构建 history-state hooks 与 restore-boundary contract

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## OpenSpec Change
- [add-history-state-hooks-and-restore-boundaries](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-history-state-hooks-and-restore-boundaries>)

## Delta Spec
- [history-state-hooks-and-restore-boundaries/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-history-state-hooks-and-restore-boundaries/specs/history-state-hooks-and-restore-boundaries/spec.md>)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把 hooks 扩展到 `history checkout / restore branch head / fork from history node / switch branch` 这条会话状态恢复路径，让系统能在不绕开 history graph、history cursor 与 workspace rollback 真相源的前提下，统一观察、阻断和审计历史态切换与恢复边界。

## 输出
- history-state hook point 第一版
- normalized history-control envelope 与 rollback/degrade 合同
- history-state evidence 的 persistence / reload / read-plane 最小闭环
- 与 `PA-028 / PA-032 / PA-037 / PA-033 / PA-035` 的职责边界
- history-state hooks 的验收矩阵

## 验收标准
- hooks 只允许挂在稳定的 `history checkout / branch restore / branch fork / branch switch` boundary 上
- hooks 只消费 normalized history-control envelope，不得直接改写 `SessionStore`、history graph、history cursor 或 workspace rollback 结果
- `TranscriptOnly / TranscriptAndWorkspace / degraded_to_transcript_only` 语义不得因 hooks 退化为前端补偿逻辑或隐式状态猜测
- hooks 不得伪造 `workspace_rollback_capable / workspace_rollback_applied / degradation_reason`，也不得绕开既有 history truth-source
- history-state evidence 至少覆盖 `history_checkout / branch_restore / branch_fork / branch_switch` 四类 canonical boundary，并能在 reload 后被 control-plane / runtime view 读回 boundary、result kind、duration 与 degraded 相关摘要
- history-state evidence 只作为 persisted audit chain，不得成为 restore、submission 或 history cursor 的仲裁输入
- 测试至少覆盖 checkout/restore/fork/switch 四类 canonical boundary、一次 transcript+workspace degrade 路径、一次“缺少 hooks evidence 不能重建 restore 结论”的负向场景，以及 file-backed reload / control-plane / runtime-view roundtrip 一致性断言

## 当前进展
- `PA-028` 已稳定 history graph、branch、cursor 与 `checkout/restore/fork/switch` 真边界
- `PA-032` 已稳定 recovery / degrade contract，并显式暴露 transcript/workspace 双维恢复结果
- `PA-037` 已完成 session control 对 history degrade feedback 的统一前端反馈
- `PA-033 / PA-035` 已完成 hooks foundation 与 stable-boundary runtime dispatch 基线
- `PA-041` 负责把这些已稳定的 history-state boundary 接入 hooks，而不是回到 UI 或 runtime 私有分支补丁式扩展
- 已完成一轮独立 spec 审核并采纳修订，见：
  [2026-06-05-pa041-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa041-spec-review.md)
- 已完成一轮只读实现入口勘探：
  - `SessionStore::checkout_history_node(...)`
  - `SessionStore::restore_branch_head(...)`
  - `SessionStore::fork_from_history_node(...)`
  - `SessionStore::switch_history_branch(...)`
  - `HostControlPlane` 对应的四个控制命令入口
- 已启动第一段 contract/scaffolding 实现：
  - `hooks.rs` 已新增 `HistoryStateHookPoint / HistoryStateCommandKind / HistoryStateHookEnvelope / HistoryStateCursorSummary / HistoryStateHookEvidence`
  - 已新增 `HistoryStateHookExecutor / NoopHistoryStateHookExecutor`
  - 已新增最小 hooks 基线测试 `agent::hooks::tests::noop_history_state_executor_returns_empty_results`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::noop_history_state_executor_returns_empty_results -- --exact --nocapture` 通过
  - `cargo check --manifest-path src-tauri/Cargo.toml --lib` 通过
- 已把第一条 session 闭环接到 `checkout_history_node(...)`：
  - `SessionState / SessionSnapshot` 已纳入 `history_state_evidence`
  - `SessionStore` 已接入 `HistoryStateHookExecutor`
  - `checkout_history_node(...)` 已在 `history.checkout.start / history.checkout.resolved` 两个 boundary 调度 hooks，并持久化 audit evidence
  - blocked checkout 会保留既有 truth-source，不写入 resolved evidence，不改写 live cursor 结论
- 已把同一套 session-level dispatch/evidence contract 扩到其余三条 history-state 命令：
  - `restore_branch_head(...)` 已在 `history.branch_restore.start / history.branch_restore.resolved` 调度 hooks 并持久化 evidence
  - `fork_from_history_node(...)` 已在 `history.branch_fork.start / history.branch_fork.resolved` 调度 hooks 并持久化 evidence
  - `switch_history_branch(...)` 已在 `history.branch_switch.start / history.branch_switch.resolved` 调度 hooks 并持久化 evidence
  - `fork` / `restore` / `switch` 被 guard 阻断时同样保持既有 truth-source，不把 hooks evidence 变成 restore 仲裁输入
- 已把同口径读面最小投影接到 host control plane：
  - `HistoryCheckoutResponse` 已显式暴露 `history_state_evidence`
  - `SessionRuntimeView` 已显式暴露 `history_state_evidence`
  - 当前两者都直接投影 `SessionSnapshot.history_state_evidence`，不在读面重算 restore 结论
- 已把其余 history 命令响应也接入同一份 persisted audit chain 投影：
  - `RestoreBranchHeadResponse`
  - `ForkFromHistoryNodeResponse`
  - `SwitchHistoryBranchResponse`
- 已完成一轮前后端合同对齐：
  - `src/types/runtime.ts` 已补 `HistoryStateHookEvidence / SessionSnapshot.historyStateEvidence / SessionRuntimeView.historyStateEvidence`
  - `src/stores/runtime.ts` 已把 tauri 返回的 `checkout / restore / fork / switch` 响应规范化为前端可消费形态，避免 `cursor` 嵌套结构与旧前端扁平结果错位
  - `HomeSessionSidebar.vue` 已兼容新旧 history-control 响应字段，优先消费新 contract，同时不打断现有 UI 测试桩
- 已补第一组定向回归测试：
  - `agent::session::tests::checkout_history_node_persists_history_state_hook_evidence`
  - `agent::session::tests::checkout_history_node_blocked_by_hook_persists_only_start_evidence`
  - `agent::session::tests::history_state_hook_evidence_roundtrip_through_file_backend`
  - `agent::session::tests::restore_branch_head_persists_history_state_hook_evidence`
  - `agent::session::tests::fork_from_history_node_blocked_by_hook_persists_only_start_evidence`
  - `agent::session::tests::switch_history_branch_persists_history_state_hook_evidence`
  - `agent::session::tests::checkout_history_node_preserves_degraded_truth_source_with_hooks`
  - `agent::session::tests::missing_history_state_evidence_does_not_reconstruct_restore_conclusion_after_reload`
  - `agent::control_plane::tests::history_checkout_response_and_runtime_view_share_same_history_state_evidence_projection`
  - `agent::control_plane::tests::history_restore_fork_switch_responses_project_history_state_evidence`
  - `agent::control_plane::tests::history_checkout_degrade_truth_source_does_not_depend_on_hooks_evidence`
  - `npx vue-tsc --noEmit`
  - `npm run test:ui-guard`
  - `cargo check --manifest-path src-tauri/Cargo.toml --lib` 继续通过
- 已完成正式 acceptance audit，可按完成态关闭：
  - [2026-06-05-pa041-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa041-acceptance-audit.md)
- 当前判断：
  - 这条线已经具备独立拆卡条件
  - 当前不建议再新增“第四张模糊 hooks 大卡”，而应继续沿稳定 boundary 拆独立 change

## 下一步动作
- 本卡完成后，后续如果继续推进 history-state evidence 展示精炼，应以新卡承接：
  - host surface 进一步审查
  - evidence summary DTO
  - session control UI 深化联调

## 当前卡点
- 无

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-028-build-history-node-management-and-branching.md`
- `management/task-system/03_TASKS/PA-032-stabilize-trace-persistence-and-recovery-contract.md`
- `management/task-system/03_TASKS/PA-037-build-session-control-surface-and-feedback-loop.md`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/control_plane.rs`
- `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
