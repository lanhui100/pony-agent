# PA-038 构建 run hooks 与 execution-control boundary

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## OpenSpec Change
- [add-run-hooks-and-execution-control-boundaries](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-run-hooks-and-execution-control-boundaries)

## Delta Spec
- [run-hooks-and-execution-control-boundaries/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-run-hooks-and-execution-control-boundaries/specs/run-hooks-and-execution-control-boundaries/spec.md)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
在 turn hooks foundation 与 stable-boundary runtime dispatch 都已稳定之后，把 hooks 扩展到 `graph run / execution control` 这一层，让 `start / wait_user / stop / resume / submission-plan` 的关键边界也能被统一观察、拦截和审计，而不是继续散落在 graph / control-plane / runtime 决策分支里。

## 输出
- graph-run / execution-control hook point 第一版
- graph run canonical boundary 与 hook binding
- stop / resume / submission-plan 相关 hook 失败策略与恢复口径
- `GraphRun / GraphRunCheckpoint` persisted evidence、runtime view / control-plane read-plane 与 frontend 可见的最小审计链
- 与 `PA-033 / PA-035 / PA-037` 的范围隔离说明

## 验收标准
- run hooks 只允许挂在稳定 `graph run / execution control` boundary 上
- hooks 不直接改 graph run store 或 runtime turn store，而是通过结构化结果参与决策
- stop / resume / submission-plan 语义不因 hooks 退化成前端私有补偿逻辑
- submission-plan 相关 hook 不得新增执行入口类型、不得直接推进 run phase、不得替代既有 arbitration truth-source
- hook evidence 至少覆盖 `submission_plan / wait_user / stop_requested / run_resume` 四类 canonical boundary，并能进入 `GraphRun / GraphRunCheckpoint` persisted evidence 后被 runtime view / control-plane 读回 `boundary / result kind / duration`
- 测试至少覆盖 `submission_plan / wait_user / stop_requested / run_resume` 的 canonical boundary 断言，以及一次 reload 后 control-plane/runtime-view 的 roundtrip 读回；`cancel` 可作为与 `stop_requested` 共用的 failure-policy 子场景

## 当前进展
- `PA-033` 已完成 hooks foundation / binding / traceability
- `PA-035` 已完成 turn stable-boundary runtime dispatch
- `PA-037` 已把 session 控制 UX 收口为真实用户入口
- `PA-038` 负责把 hooks 扩展到 graph run 与 execution control，不回吞 planner / memory / capability mediation
- 已完成一轮独立 spec 审核并采纳修订，见：
  [2026-06-04-pa038-pa039-pa040-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa038-pa039-pa040-spec-review.md)
- 已启动第一段 contract/scaffolding 实现：
  - hooks 基础设施开始补 run-level hook point、canonical graph-run vocabulary 与 control envelope 骨架
- 已完成第二段 boundary read-model 对齐：
  - `submission_plan` 现已输出 `hook_point / canonical_event_type / canonical_phase / hook_envelope`
  - `continue_graph_run_stream` 在 graph-run fallback 场景下会被规范化到 `ready` boundary，而不是泄露内部 `running` phase
  - hooks 契约层改为依赖抽象 `RunControlCheckpointContext`，避免反向耦合到具体 checkpoint 存储类型
- 已完成第三段 graph-run event boundary 对齐：
  - `GraphRunEvent` 现已直接携带可选 `hook_point / canonical_event_type / canonical_phase`
  - `run_start / wait_user / run_paused / run_completed / run_failed / run_cancelled` 不再需要由 read-plane 或前端自行猜测
  - 对于 `Updated + Ready/Running` 这类存在语义歧义的事件，当前显式保持 `None`，避免把 hooks 扩展面误做成第二调度层
- 已完成第四段 command-boundary evidence 落地：
  - `GraphRun` / `GraphRunCheckpoint` 已新增 `control_boundary_evidence`
  - `stop_graph_run` 现在会生成独立的 `stop_requested` boundary evidence，而不是只剩 `run_paused` 结果态
  - `resume_graph_run` / `prepare_resume_graph_run_stream` 现在会生成独立的 `run_resume` boundary evidence，并与后续 graph 结果事件分离
  - `GraphRunControlResponse / GraphRunStreamStartResponse` 已开始暴露当前命令边界 evidence，供 control-plane 与运行时读面消费
  - `SessionRuntimeView` 与前端 runtime store 已开始统一读取这批 evidence，避免 session control 只看得到局部读面
- 已完成第五段 session control 前端消费：
  - `HomeWorkspace` 与 `HomeSessionSidebar` 已开始展示最近一条 `control_boundary_evidence`
  - session control 相关 Vitest 回归已补，避免这批命令边界证据继续只停留在 read-model
- Rust 定向验证已补到真实边界：
  - `agent::graph::tests::graph_runner_can_start_run_and_record_waiting_user_turn`
  - `agent::control_plane::tests::graph_run_can_stop_resume_and_expose_checkpoint`
  - `agent::graph::tests::persistent_graph_run_store_roundtrips_checkpointable_state`
  - 既有 submission-plan 测试已补 canonical boundary 断言
- 已完成 acceptance audit 与 closeout，见：
  [2026-06-05-pa038-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa038-acceptance-audit.md)
- 当前卡的实现缺口已全部关闭：
  - `submission_plan / wait_user / stop_requested / run_resume` 的 canonical boundary 已进入 `GraphRun / GraphRunCheckpoint` persisted evidence
  - runtime view / control-plane / session control 前端已形成统一最小读面
  - reload / roundtrip / session control 回归已完成并可复核

## 下一步动作
- 本卡已完成 closeout；后续若要把 run-level evidence 升格到更正式 persisted trace，应另开新卡承接

## 当前卡点
- 无；本卡已完成

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-022-build-lifecycle-hooks-pipeline.md`
- `management/task-system/03_TASKS/PA-033-build-agent-hooks-pipeline-foundation.md`
- `management/task-system/03_TASKS/PA-035-integrate-runtime-hook-dispatch-on-stable-boundaries.md`
- `src-tauri/src/agent/graph.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/agent/control_plane.rs`
