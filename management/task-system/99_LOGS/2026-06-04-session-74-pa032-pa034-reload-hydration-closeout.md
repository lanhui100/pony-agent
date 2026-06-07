# 2026-06-04 Session 74

## 主题

- `PA-032` replay/resume 与 hydration 收口状态复核
- `PA-034` reload roundtrip 与 submission plan 证据补强

## 本轮完成

1. 重新盘点任务系统、OpenSpec 与代码实况：
   - 核对了 `PA-032 / PA-033 / PA-034` 任务卡、OpenSpec tasks 与当前实现
   - 明确近线仍应先收口 lifecycle / recovery 事实源，不提前把 hooks runtime dispatch 硬接进去
2. 补齐 `PA-034` 后端 reload roundtrip 的 execution-plan 证据：
   - `src-tauri/src/agent/control_plane.rs` 的 `file_backed_reload_restores_lifecycle_boundary_projection` 已继续补断言
   - 现在该测试除 checkpoint 投影外，还会断言 `runtimeView.submissionPlan` 为：
     - `command = start_graph_run_stream`
     - `run_id = None`
     - `source = default`
   - 这证明 reload 后的 completed/lifecycle-boundary session 不会错误继续旧 run
3. 补齐 `PA-032` 前端 hydration 回归：
   - `tests/runtime-store.spec.ts` 已新增“hydrates default submissionPlan without reviving an active run id”
   - 该测试覆盖 session runtime view 提供 `submissionPlan = start_graph_run_stream` 时：
     - `latestGraphRunSubmissionPlan` 会被正确 hydrate
     - `activeRunId` 不会被错误复活
4. 补齐 `PA-032` 后端最终仲裁证据：
   - `src-tauri/src/agent/control_plane.rs` 已新增：
     - `submission_plan_starts_fresh_run_when_recovery_contract_requires_replay`
     - `submission_plan_switches_with_session_checkpoint_boundary`
   - 前者验证 `replay_required` recovery contract 会仲裁为 `start_graph_run_stream`
   - 后者验证同一 session 在 boundary 前后会从 `continue_graph_run_stream` 切到 `resume_graph_run_stream`
5. 同步主视图文档：
   - `management/task-system/01_TASK_BOARD.md` 已把 `PA-034` 纳入 `In Progress`
   - `management/task-system/00_DASHBOARD.md` 已把 `PA-034` 纳入当前主线与下一步动作
6. 同步任务/Spec 状态：
   - `PA-032` 任务卡已补“第十五轮 reload/hydration 执行计划收口”
   - `PA-032` 任务卡已补“第十六轮后端最终仲裁证据补强”
   - `openspec/changes/add-trace-persistence-and-recovery-contract/tasks.md` 已补 3.5 的“已完成/待补强”拆分说明
7. 完成 `PA-032` acceptance audit：
   - 已新增 [2026-06-04-pa032-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa032-acceptance-audit.md)
   - 审计结论：`PA-032` 已满足完成边界，可从 `In Progress` 进入 `Review`
   - `01_TASK_BOARD.md` 已同步把 `PA-032` 挪到 `Review`

## 子智能体审计结论

- 本轮委派只读 explorer 审计 `PA-032` 剩余 3.4 / 3.5
- 审计结论：
  - 当前更接近“实现大体到位，只差最终测试证据和验收回写”
  - 3.4 主要缺 Rust 侧 replay/resume 最终仲裁的精确执行证据
  - 3.5 主要缺 session 级 checkpoint 切换与 hydration 的最终验收勾销，而不是明显主逻辑缺口

## 验证

- `npx vitest run tests/runtime-store.spec.ts`
  - 结果：通过，`51 passed`
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet`
  - 结果：通过
  - 备注：仍有本机 Windows incremental `os error 5` warning，但未阻断编译
- `cargo test --manifest-path src-tauri/Cargo.toml submission_plan_starts_fresh_run_when_recovery_contract_requires_replay --lib -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml submission_plan_switches_with_session_checkpoint_boundary --lib -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml agent::control_plane::tests::completed_session_can_project_checkpoint_lifecycle_boundary_without_recovery --lib -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml agent::control_plane::tests::lifecycle_boundary_checkpoint_does_not_override_default_submission_plan --lib -- --exact`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml agent::session::tests::file_backend_roundtrip_restores_checkpoint_persist_evidence --lib -- --exact`
  - 结果：通过
  - 备注：以上 exact 用例通过独立 `CARGO_TARGET_DIR=src-tauri/target-codex-pa032` 绕开默认 target 目录锁冲突后获得稳定执行结论

## 当前判断

- `PA-034` 的 reload / hydration / control-plane 读面闭环进一步变硬
- `PA-032` 的 3.4 / 3.5 已拿到 exact Rust 与前端回归证据，当前主要差 acceptance audit 勾销
- `PA-034` 也已补到多条 exact Rust 证据，剩余重点进一步转向 runtime completion exact coverage 与完成态审计

## 下一步

1. 继续补 `PA-032` 的 Rust 侧 replay/resume 最终仲裁测试
2. 在条件允许时尽量拿到非 `LNK1104` 环境下的精确执行结论
3. 等 `PA-032 / PA-034` 验收再稳一轮后，再推进 `PA-033` 的下一段 foundation 收口
