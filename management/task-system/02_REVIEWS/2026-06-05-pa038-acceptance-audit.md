# PA-038 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-038-build-run-hooks-and-execution-control-boundaries.md`
- `openspec/changes/add-run-hooks-and-execution-control-boundaries/specs/run-hooks-and-execution-control-boundaries/spec.md`
- `openspec/changes/add-run-hooks-and-execution-control-boundaries/tasks.md`
- `src-tauri/src/agent/hooks.rs`
- `src-tauri/src/agent/graph.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/components/HomeWorkspace.vue`
- `src/components/HomeSessionSidebar.vue`
- `tests/runtime-store.spec.ts`
- `tests/HomeWorkspace.spec.ts`
- `tests/HomeSessionSidebar.spec.ts`

## 审核口径

只按 `PA-038` 当前任务卡与 delta spec 的完成边界判断：确认 run hooks / execution-control boundary 是否已经在 `submission_plan / wait_user / stop_requested / run_resume` 四类 canonical boundary 上形成真实 persisted evidence、reload/read-plane roundtrip 与 session control 最小可见反馈；不把未来“更正式 persisted trace”当作本卡关闭前置。

### 不在本审计内

- 通用 run-hook executor 的进一步扩展
- 比 `GraphRun / GraphRunCheckpoint` 更高一层的 persisted trace 升格
- `PA-039 / PA-040` 的 memory-write / planner / capability hooks 范围

## 逐项结论

### A. stable graph-run / execution-control boundary 已收口到承诺的最小集合

状态：`达成`

代码参考：

- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:347)
- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:377)
- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:398)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:1892)
- [src-tauri/src/agent/graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs:119)

判断：

本卡承诺的稳定 boundary 已收敛到 `submission_plan / wait_user / stop_requested / run_resume`，没有把 hooks 扩展成第二调度层或回到 graph 内部临时步骤。

### B. command-boundary evidence 已进入 persisted graph truth-source

状态：`达成`

代码参考：

- [src-tauri/src/agent/graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs:119)
- [src-tauri/src/agent/graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs:198)
- [src-tauri/src/agent/graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs:650)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:983)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:1316)

判断：

`GraphRun / GraphRunCheckpoint` 已稳定持有 `control_boundary_evidence`，`stop_requested` 与 `run_resume` 不再隐没在结果态事件里。

### C. reload / runtime-view / session control 读面闭环成立

状态：`达成`

代码参考：

- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:2006)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:2288)
- [src/components/HomeWorkspace.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeWorkspace.vue)
- [src/components/HomeSessionSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)

验证：

- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::graph::tests::graph_runner_can_start_run_and_record_waiting_user_turn -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::graph_run_can_stop_resume_and_expose_checkpoint -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::graph::tests::persistent_graph_run_store_roundtrips_checkpointable_state -- --exact --nocapture`
- `npx vitest run tests/runtime-store.spec.ts -t "hydrates submissionPlan from session runtime view and reuses its runId|hydrates default submissionPlan without reviving an active run id"`
- `npx vitest run tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts --reporter=dot`

判断：

`GraphRun / GraphRunCheckpoint` roundtrip、runtime view hydrate 和 session control 前端反馈已经形成最小验收闭环。

### D. run-level hooks 仍是受控扩展，不是第二 arbitration source

状态：`达成`

规范参考：

- [management/task-system/03_TASKS/PA-038-build-run-hooks-and-execution-control-boundaries.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-038-build-run-hooks-and-execution-control-boundaries.md:20)
- [openspec/changes/add-run-hooks-and-execution-control-boundaries/specs/run-hooks-and-execution-control-boundaries/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-run-hooks-and-execution-control-boundaries/specs/run-hooks-and-execution-control-boundaries/spec.md:10)

判断：

submission-plan 仍是 execution command 仲裁真源；本卡没有为了可观测性把 run hooks 扩大成新的 scheduler 或新的 command source。

## 验证备注

- 本轮重新执行的定向测试全部通过。
- `cargo check --manifest-path src-tauri/Cargo.toml --tests` 与 `cargo test --manifest-path src-tauri/Cargo.toml --test session_regression -- --nocapture` 现已通过；此前 `session_regression` 缺失 `capability_bridge` fixture module 的编译断点已一并修复。
- Windows 环境仍有 incremental finalize `os error 5` warning，但未影响通过判定。

## 最终裁定

`PA-038` 已满足任务卡与 delta spec 的完成边界，可以从 `In Progress` 更新为 `Done`。

关闭理由：

1. `submission_plan / wait_user / stop_requested / run_resume` 四类 canonical boundary 已形成真实 persisted evidence。
2. `GraphRun / GraphRunCheckpoint`、runtime view、session control 前端三层读面已完成 roundtrip 闭环。
3. run-level hooks 仍保持受控扩展边界，没有越界成为第二 arbitration source。
