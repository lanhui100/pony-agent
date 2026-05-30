# PA-010 构建 runtime execution control substrate（停止条件 / 取消 / checkpoint 底座）

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
把当前以“单个 turn 执行”为核心的 runtime，推进到“可被未来 graph 编排层复用的执行控制底座”，而不是直接在 runtime 层实现高层 goal-driven loop。

## 输出
- `ExecutionCheckpoint` 结构与 registry
- turn / graph 边界约束
- turn 级 stop / cancel 命令契约
- cooperative cancel 执行路径
- `turn:cancelled` 事件与前端收口

## 验收标准
- 可以明确区分“单 turn 内多 hop”与“graph 级多 turn 编排”
- runtime 暴露 turn 级 stop 与 checkpoint 读取入口
- runtime 在执行链路中能响应 cooperative cancel，并收口为 `cancelled`
- adapter 层不需要理解执行控制内部细节，只消费统一命令与事件契约
- 文档明确说明本任务不包含 graph loop

## 当前进展
- `runtime` 已支持单 turn 内多 hop tool follow-up
- `ToolPlan` 已进入 runtime 级数据结构，便于后续 loop 消费
- `TurnEventSink` 已把 Tauri / SSE adapter 与 core 解耦
- 架构文档已明确区分 `turn loop` 与未来 `graph loop`
- 已明确 stop / checkpoint 也必须按 runtime 与 graph 分层
- `src-tauri/src/lib.rs` 已暴露 `stop_turn` 与 `load_execution_checkpoint` Tauri 命令
- `src-tauri/src/agent/execution_control.rs` 已实现 `ExecutionControlRegistry` 与 `ExecutionCheckpoint`
- checkpoint 已覆盖 `status / phase / provider meta / completed_hops / max_hops / active_tool_name / trace_steps / tool_activities / stop_requested_at_ms`
- `runtime.rs` 已在流式执行关键节点检查 `is_stop_requested(turn_id)`，以 cooperative cancel 方式结束当前 turn
- `turn_flow.rs` 已新增 `emit_stream_cancelled()`，统一发出 `turn:cancelled`
- `src/stores/runtime.ts` 已监听 `turn:cancelled`，会结束提交态、写入停止提示并记录 turn trace
- 前端现已在会话初始化与切换时消费 `load_execution_checkpoint(session_id)`，可恢复运行中的 turn、trace 与 tool activity
- cancelled turn 现会持久化进 session history，reload 前后不会丢失最后一轮用户输入与停止结果
- 前后端已统一建模 `cancelled` phase，不再把主动停止误记成 `failed`

## 本轮实际结果
- 本轮完成的是 runtime 执行控制底座，不是 graph loop。
- 当前已落地能力：
- 停止当前 turn：前端 `stopTurn()` -> Tauri `stop_turn` -> `ExecutionControlRegistry::request_stop()`
- 读取最近执行快照：`load_execution_checkpoint(turn_id | session_id)`
- cooperative cancel：执行链自己检查 stop 标记后收口，不依赖宿主强杀线程
- cancelled 事件：runtime 发出 `turn:cancelled`，前端更新消息与 trace
- checkpoint 恢复：前端刷新或重新切回会话时，可基于 `load_execution_checkpoint()` 接住尚未结束的流式 turn
- 历史一致性：取消回合会把用户消息与取消结果一并落到 session history
- 当前未落地能力：
- goal / run 级停止
- graph loop
- graph checkpoint 恢复
- 完整 stop-condition 矩阵，例如 `budget_exhausted / timeout / consecutive_error`

## Review 收口
- 已修复 review 发现的 3 个高优先级问题：
- 前端未消费 `load_execution_checkpoint`，刷新/切会话会丢失运行中 turn
- cancelled turn 只记 trace、不写 session history，reload 后最后一轮会消失
- `cancelled` 语义未进入统一 phase，前端 trace 曾错误记成 `failed`

## 验证结果
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml --test session_regression --test provider_registry_regression --test tool_router_regression`
- `npx vitest run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts`

## 下一步动作
- 如果继续推进，下一张任务应放在 graph 层，而不是继续扩大本卡范围
- 为 goal / run 级停止单独定义命令契约
- 为 graph checkpoint 建立在 `TurnResult / SessionSnapshot / ExecutionCheckpoint` 之上的组合层
- 视需要补 `budget_exhausted / timeout` 等更多终态

## 当前卡点
- 当前实现已经把 runtime 与 graph 的职责线划出来；剩余卡点是不要在后续迭代里把 graph 能力继续塞回本卡

## 断点续跑提示
继续前先看：
- `docs/architecture/runtime.md`
- `docs/architecture/overview.md`
- `src-tauri/src/agent/execution_control.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `src/stores/runtime.ts`
