# PA-014 补齐 graph stop / resume / checkpoint 与 stop-condition 矩阵

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
在 `PA-013` 的最小 graph orchestrator 之上，补齐 run 级停止、恢复与 checkpoint 语义，并把 `budget_exhausted / timeout / consecutive_error / user_stop` 等停止原因提升到 graph 层统一管理。

## 输出
- `GraphRunCheckpoint` 结构
- run / goal 级 stop 命令契约
- graph resume 入口与恢复约束
- graph 级 stop-condition 矩阵
- graph / runtime 停止原因映射规则

## 验收标准
- 宿主可以明确停止当前 run，而不破坏 runtime 的单 turn 收口语义
- graph checkpoint 只建立在稳定 turn 产物之上，不读取 runtime 瞬时内部变量
- graph resume 可以从已持久化的 run 状态继续，而不是重新拼接半成品 stream
- 至少能区分 `completed / cancelled / failed / paused / budget_exhausted / timeout`
- 文档明确本卡处理的是 graph 层 stop/resume，不回写到 `PA-010` 范围

## 当前进展
- `PA-010` 已完成 turn 级 stop 与 execution checkpoint substrate
- 当前仍没有 goal / run 级 stop、graph checkpoint 与 graph resume
- `PA-013` 预计先建立最小 graph run 生命周期与状态迁移
- 现有任务板已明确完整 stop-condition 矩阵不属于 `PA-010`

## 本轮验收
- 已补 `GraphRunStopReason / GraphRunCheckpoint / active_turn_id / last_completed_turn_id / stop_reason / last_handoff / resume_count`
- 已打通 `stop_graph_run / resume_graph_run / load_graph_run_checkpoint`
- 已让 graph run store 持久化 run 状态，并验证可从持久化后的 paused run 恢复
- 已通过：
  - `cargo check --manifest-path src-tauri/Cargo.toml --lib`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib`

## 下一步动作
- 在 graph run store 稳定后定义 checkpoint 持久化形态
- 把 run 级停止原因与 runtime `cancelled / failed` 终态做清晰映射
- 为宿主控制面补充 run stop / run inspect / run resume 入口

## 当前卡点
- 最大风险是把“graph checkpoint”做成 runtime 内部变量快照；这会让恢复语义不稳定，也会让 adapter 和宿主被迫理解底层执行细节

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-013-build-minimal-graph-run-orchestrator.md`
- `management/task-system/03_TASKS/PA-010-build-runtime-loop-and-stop-conditions.md`
- `docs/architecture/runtime.md`
- `docs/architecture/overview.md`
