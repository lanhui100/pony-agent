# PA-012 定义 graph run contract 与 runtime handoff 边界

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
在 `PA-010` 已完成 runtime execution control substrate、`PA-011` 已完成最小多模态 session memory 的前提下，正式定义 graph 层要消费的稳定产物、状态机和命令边界，让后续多 turn 编排建立在“完整 turn 结果”之上，而不是继续侵入 runtime 内部。

## 输出
- `GraphRun / GraphStep / GraphDecision` 第一版数据结构
- runtime -> graph handoff 契约
- graph 级状态迁移草案
- graph 层 continue / pause / wait_user / completed 判定边界
- 架构文档中的 graph/runtime 分层补充说明

## 验收标准
- 能明确说明 graph 层只消费稳定的 `TurnResult / SessionSnapshot / ExecutionCheckpoint`
- 能明确区分 `turn stop` 与 `goal/run stop`
- graph 状态至少能覆盖 `ready / running / waiting_user / paused / completed / failed / cancelled`
- adapter 不需要知道 graph 内部状态机，只消费统一命令与事件
- 文档明确本卡不直接实现自动多 turn loop

## 完成情况
- 已新增 `GraphRunPhase / GraphDecisionKind / GraphDecisionReason / GraphRun / GraphStep / GraphTurnHandoff`
- 已在 `GraphEngine` 中落地 `build_turn_handoff()` 与 `decide_after_turn()`
- 已在 `AgentRuntime` 中接入 `build_graph_turn_handoff()` 与 `decide_graph_after_turn()` 作为 runtime -> graph handoff 边界
- graph 默认判定已明确：
- runtime 仍在 `running` 时返回 `continue`
- turn `failed` / `cancelled` 时返回终态
- 其余完整收口 turn 默认返回 `wait_user`
- 已明确本卡只定义 contract，不实现自动多 turn loop；该职责继续留给 `PA-013`

## 验证
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`

## 后续衔接
- `PA-013` 继续消费本卡产出的 contract，把 graph decision 接成最小 orchestrator
- `PA-014` 再在 `PA-013` 之上补 graph stop / resume / checkpoint 矩阵

## 断点续跑提示
继续前先看：
- `docs/architecture/runtime.md`
- `docs/architecture/overview.md`
- `management/task-system/03_TASKS/PA-010-build-runtime-loop-and-stop-conditions.md`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/execution_control.rs`
