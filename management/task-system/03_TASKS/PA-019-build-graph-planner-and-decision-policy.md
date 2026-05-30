# PA-019 建立 graph planner 与计划决策策略

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## 目标
在 `PA-012 ~ PA-014` 建立 graph run contract、最小 orchestrator 与 run 级 stop/resume 之后，把当前零散存在于 runtime preflight 中的局部 planning 能力提升到 graph 层，形成统一的 planner / policy 模块，负责“目标推进、下一步决策、是否继续下一轮”的上层计划逻辑。

## 输出
- `GraphPlanner / PlanningPolicy / PlanStep` 第一版结构
- graph planner 输入输出契约
- continue / pause / ask_user / delegate / complete 的决策策略草案
- planner 与 memory / tools / skills / MCP 的依赖边界
- planner 观测与审计字段

## 验收标准
- planner 的职责清楚地位于 graph 层，而不是继续堆在 runtime turn preflight 中
- planner 消费稳定的 run state / memory retrieval / tool/skill capability facts
- planner 能明确输出“下一轮做什么”，而不是直接执行底层 turn
- adapter 不需要理解 planner 内部规则
- 文档明确本卡不要求一次性实现复杂 DAG planner 或完整任务树

## 当前进展
- 已新增 `GraphPlanner / GraphPlanningContext / DefaultGraphPlanner`，并把 graph policy 明确放在 graph 层消费。
- `HostControlPlane` 现在会在 graph run 推进时调用 graph planner，而不是继续把“是否下一轮”塞回 runtime turn preflight。
- 第一版 policy 只做保守判断：
  - assistant 明确在向用户提问或等待确认时，输出 `wait_user`
  - goal 带有明确的多轮推进信号、且本轮已稳定收口时，输出 `continue`
  - 其他情况默认 `wait_user`
- 当前版本不会自动递归跑第二个 turn；`continue` 只把 run 保持在可继续状态，供宿主再次调用 `continue_graph_run`
- 已补齐 `planner.rs / graph.rs` 单测，并通过 `cargo test --manifest-path src-tauri/Cargo.toml --lib`

## 下一步动作
- 先把 turn-local planner 与 graph planner 的边界拆清
- 再定义 planner 基于 goal/run state 的输入输出
- 为 skills、MCP 与 hooks 留出 planner 侧挂接点

## 当前卡点
- 若过早让 planner 直接操作 runtime stream 或 tool hop，会把 graph 与 runtime 再次缠在一起

## 本轮实现说明
- graph planner 继续只消费稳定产物：`GraphRun + GraphTurnHandoff`
- runtime 仍只负责单 turn 内的 `model -> tool -> model` 收口，不承担 graph loop
- `continue` 决策当前映射为 `GraphRunPhase::Ready`，表示“可以继续下一轮，但本轮不自动再跑”

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-012-define-graph-run-contract-and-runtime-handoff.md`
- `management/task-system/03_TASKS/PA-013-build-minimal-graph-run-orchestrator.md`
- `management/task-system/03_TASKS/PA-014-add-graph-stop-resume-and-checkpoint-matrix.md`
- `management/task-system/03_TASKS/PA-008-expand-tool-runtime-robustness.md`
- `docs/architecture/runtime.md`
