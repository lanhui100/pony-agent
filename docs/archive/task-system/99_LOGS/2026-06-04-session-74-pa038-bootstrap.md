# 2026-06-04 Session 74

## 主题

- 启动 `PA-038`，完成 run hooks / execution-control boundary 的第一段 contract/scaffolding

## 本轮完成

1. 将 `PA-022` 的 post-foundation hooks 范围拆成三张可执行卡
   - `PA-038`：run hooks 与 execution-control boundary
   - `PA-039`：memory-write hooks 与 persisted side-effect contract
   - `PA-040`：planner 与 capability-mediation hooks
2. 为三张卡补齐 OpenSpec 变更骨架
   - `proposal / design / tasks / spec`
3. 完成独立 spec review 并采纳修订
   - 见 [2026-06-04-pa038-pa039-pa040-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa038-pa039-pa040-spec-review.md)
   - 重点采纳：
     - `PA-038` 不得成为第二 arbitration source
     - `PA-039` 明确 persisted evidence 最小字段与 replay 优先级
     - `PA-040` 分开 planner / capability 的 transform 白名单
4. 正式启动 `PA-038`
   - 状态已切到 `In Progress`
   - `hooks.rs` 已新增：
     - `RunHookPoint`
     - `CanonicalGraphRunPhase`
     - `CanonicalGraphRunEventType`
     - `RunHookLifecycleBinding`
     - `ExecutionControlCommandKind`
     - `RunControlHookEnvelope`
     - `canonical_graph_run_binding_for_hook_point(...)`
     - `run_hook_point_matches_canonical_boundary(...)`
5. 将 `submission_plan` 接到真实 boundary 词汇表
   - `control_plane.resolve_graph_run_submission_plan(...)` 现在返回：
     - `hook_point`
     - `canonical_event_type`
     - `canonical_phase`
     - `hook_envelope`
   - `continue_graph_run_stream` 在没有 checkpoint、仅从 graph-run fallback 推导时，会被标准化到 `ready` boundary，避免 hooks 看到内部 `running` phase
   - hooks 契约层新增 `RunControlCheckpointContext`，避免直接依赖 `ExecutionCheckpoint` 具体实现，保证测试宿主和未来扩展点都能复用同一抽象
6. 将 `GraphRunEvent` 本身接到 canonical run-hook boundary
   - `graph.rs` 现在会在创建 `GraphRunEvent` 时直接附带可选：
     - `hook_point`
     - `canonical_event_type`
     - `canonical_phase`
   - 已覆盖的真实 boundary：
     - `run_start`
     - `wait_user`
     - `run_paused`
     - `run_completed`
     - `run_failed`
     - `run_cancelled`
   - 对 `Updated + Ready/Running` 这类存在歧义的 graph event 暂不强行归类，避免把 hooks 词汇表退化成“猜测内部调度语义”
7. 已补 PA-038 当前最贴近改动面的定向测试
   - `agent::graph::tests::graph_runner_can_start_run_and_record_waiting_user_turn`
   - `agent::control_plane::tests::graph_run_can_stop_resume_and_expose_checkpoint`
   - 既有 submission-plan 相关测试已补 canonical boundary 断言

## 验证

- `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet`
  - 结果：通过
  - 备注：Windows 增量编译仍有 `os error 5` finalize warning，未见新的逻辑失败
- 精确 `cargo test --lib ... -- --exact`
  - 结果：冷编译超时，暂未形成新的逻辑失败证据
   - 已补对应 run-level binding 测试骨架
- `cargo test --manifest-path src-tauri/Cargo.toml --lib graph_runner_can_start_run_and_record_waiting_user_turn -- --nocapture`
  - 结果：通过（`1 passed`）
- `cargo test --manifest-path src-tauri/Cargo.toml --lib graph_run_can_stop_resume_and_expose_checkpoint -- --nocapture`
  - 结果：通过（`1 passed`）

## 验证

```powershell
cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet
$env:CARGO_TARGET_DIR='C:\Users\HUAWEI\Documents\pony-agent\src-tauri\target-codex-pa038'; cargo test --manifest-path src-tauri/Cargo.toml --lib run_hook_points_map_only_to_canonical_graph_vocabulary -- --exact
```

结果：

- `cargo check --tests --quiet`：通过
- `run_hook_points_map_only_to_canonical_graph_vocabulary` exact：
  - 本轮在 Windows 环境下仍因冷编译/链接时延超时
  - 未形成新的逻辑失败证据
- `graph_runner_can_start_run_and_record_waiting_user_turn`：
  - 通过，确认 `run_start -> wait_user` boundary 已有真实事件证据
- `graph_run_can_stop_resume_and_expose_checkpoint`：
  - 通过，确认 `run_paused` boundary 已进入 control-plane 响应与 checkpoint 路径

## 当前判断

- `PA-038` 已从“Ready spec”进入“开始实现”的真实状态
- 当前实现已从纯 contract/scaffolding 推进到“submission-plan + graph-run event”两段真实 boundary 对齐
- 下一步应继续补 `stop_requested / run_resume` 的命令边界显式化，以及 persisted trace / runtime view 的统一审计链

## 回写

- [PA-038](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-038-build-run-hooks-and-execution-control-boundaries.md)
- [Task Board](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [Dashboard](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
