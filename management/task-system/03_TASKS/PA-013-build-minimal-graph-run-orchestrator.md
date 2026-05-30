# PA-013 实现最小 graph run orchestrator

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
基于 `PA-012` 定义的 graph run contract，落地第一版最小 graph orchestrator，让系统可以围绕单个 goal 在多个 turn 之间推进，并在“继续下一轮”与“等待用户输入”之间做出明确决策。

## 输出
- 第一版 `GraphRunner` / `GraphRunStore`
- graph 消费单个 `TurnResult` 后的状态迁移实现
- 最小 continue-next-turn 判定逻辑
- `run started / run updated / run paused / run completed / run failed` 事件草案
- run 级 trace / inspection 读取入口

## 验收标准
- graph runner 可以启动一个 run，并驱动至少一轮真实 turn
- 单轮完成后，graph 能基于稳定结果决定“继续 / 暂停等待用户 / 结束”
- graph 层不直接依赖 tool hop 中间态或 provider stream chunk
- session 仍由 `SessionStore` 持有，graph 只消费快照与追加后的结果
- 本卡不强行引入完整 budget 矩阵、run 级恢复或附件中心逻辑

## 当前进展
- runtime 已具备单 turn 内多 hop 执行能力
- `PA-010` 已把 turn stop / checkpoint / cancelled event 变成 graph 可消费的基础设施
- `PA-012` 预计提供 graph run contract 与 continue / pause 边界
- 当前宿主侧还没有统一的 run 级事件模型与查询面

## 本轮验收
- 已落地 `GraphRunStore / GraphRunner / GraphRunEvent / GraphRunTurnResponse`
- 已打通 `start_graph_run / continue_graph_run / inspect(include_run/include_runs)` 控制面
- 已通过：
  - `cargo check --manifest-path src-tauri/Cargo.toml --lib`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib`

## 下一步动作
- 在 `PA-012` 之后实现最小 run store 与 run lifecycle
- 把 graph 判定入口放在“单个 turn 完整收口之后”
- 为 `PA-014` 预留 run stop / checkpoint / resume 所需的稳定 run 元数据

## 当前卡点
- 如果过早把预算、恢复、分支子任务都塞进来，会让最小 orchestrator 失去边界，难以验证 graph 层是否真正建立成功

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-012-define-graph-run-contract-and-runtime-handoff.md`
- `management/task-system/03_TASKS/PA-010-build-runtime-loop-and-stop-conditions.md`
- `docs/architecture/runtime.md`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/session.rs`
