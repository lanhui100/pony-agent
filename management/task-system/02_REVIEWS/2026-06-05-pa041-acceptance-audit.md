# PA-041 Acceptance Audit

## 审核范围

- [management/task-system/03_TASKS/PA-041-build-history-state-hooks-and-restore-boundaries.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-041-build-history-state-hooks-and-restore-boundaries.md)
- [openspec/changes/archive/2026-06-05-add-history-state-hooks-and-restore-boundaries/specs/history-state-hooks-and-restore-boundaries/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-history-state-hooks-and-restore-boundaries/specs/history-state-hooks-and-restore-boundaries/spec.md>)
- [openspec/changes/archive/2026-06-05-add-history-state-hooks-and-restore-boundaries/tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-05-add-history-state-hooks-and-restore-boundaries/tasks.md>)
- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- [src/types/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)
- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [src/components/HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)

## 审核口径

只按 `PA-041` 当前任务卡与 delta spec 的完成边界判断：确认 history-state hooks 是否已经在 `history checkout / branch restore / branch fork / branch switch` 四类稳定 boundary 上形成真实 dispatch、persisted audit chain、reload/read-plane 闭环，以及 degrade / restore truth-source 不被 hooks evidence 反向仲裁。

### 不在本审计内

- 新的 session-control 视觉设计或交互信息架构升级
- 是否引入单独的 history-state evidence summary DTO
- `PA-028 / PA-032 / PA-037` 之外的新 history scheduler / workflow 能力

## 逐项结论

### A. 四类 history-state boundary 的真实 hook dispatch 闭环

状态：`达成`

发现：

- `hooks.rs` 已定义 `HistoryCheckout* / BranchRestore* / BranchFork* / BranchSwitch*` 八个 hook point，以及统一的 `HistoryStateCommandKind / HistoryStateHookEnvelope / HistoryStateHookEvidence`。
- `session.rs` 中的 `checkout_history_node(...) / restore_branch_head(...) / fork_from_history_node(...) / switch_history_branch(...)` 都已经在各自 `start / resolved` boundary 执行 hook dispatch，并把结果持久化到 `history_state_evidence`。
- guard 阻断路径已显式保持既有 truth-source，不会因为 hook 结果而提前生成 resolved evidence 或改写 live cursor。

证据：

- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)

判断：

history-state hooks 已不再只是 contract 占位或 trace 旁路，而是真正接入四类稳定 session-history control boundary。

### B. persisted audit chain / reload / read-plane 闭环

状态：`达成`

发现：

- `SessionState / SessionSnapshot` 已持久化 `history_state_evidence`。
- file-backed roundtrip 已验证 evidence 可跨 reload 保留。
- `HistoryCheckoutResponse / RestoreBranchHeadResponse / ForkFromHistoryNodeResponse / SwitchHistoryBranchResponse / SessionRuntimeView` 都已直接投影同一份 persisted audit chain。
- 前端 `runtime.ts / runtime store / HomeSessionSidebar` 已对齐新 contract，不再停留在旧的 history-control 响应字段名上。

证据：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [src/types/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)
- [src/stores/runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [src/components/HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)

判断：

history-state evidence 已具备“session truth-source -> control-plane / runtime view -> frontend consumer”的最小工业化闭环，不再依赖前端私有推断。

### C. degrade / rollback / cursor 真相源 non-regression

状态：`达成`

发现：

- `TranscriptAndWorkspace -> degraded_to_transcript_only` 路径已经有显式测试证明：hooks 存在时，`workspace_rollback_applied / degradation_reason / history cursor` 仍只由既有合同决定。
- blocked checkout / fork 路径已经证明：start boundary 被阻断时不会偷偷生成 resolved evidence。
- “清空 hooks evidence 再 reload” 的负向场景已经验证：系统只表现为“无 hooks evidence”，不会据此重建 restore/cursor/degrade 结论。

证据：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)

判断：

`PA-041` 最关键的 guardrail 已经被代码和测试同时压实：history-state evidence 没有长成第二套 restore truth-source。

### D. 验收矩阵覆盖度

状态：`达成`

已覆盖：

- `checkout / restore / fork / switch` 四类 canonical boundary
- `blocked` 路径
- `TranscriptAndWorkspace` degrade 路径
- file-backed reload
- control-plane/runtime-view 一致性
- 缺失 evidence 的负向场景
- 前端类型与 UI guard 对齐

验证命令：

```powershell
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

判断：

当前验证矩阵已经满足任务卡与 delta spec 对 `PA-041` 的完成要求。

## 最终裁定

`PA-041` 已满足任务卡与 delta spec 的完成边界，可以从 `In Progress` 更新为 `Done`。

关闭理由：

1. 四类 history-state control command 都已经接入稳定 hook dispatch，并产出 persisted audit chain。
2. reload / control-plane / runtime-view / frontend 基础消费面都已对齐同一份 evidence contract。
3. `degraded_to_transcript_only` 与“缺失 evidence 不重建 restore 结论”两条高风险 guardrail 已通过显式 non-regression 测试。
4. 本卡剩余讨论点只涉及是否新增更轻量 DTO 或进一步扩展前端展示，不再构成 `PA-041` 当前 scope 的完成阻断。
