# Agent Note: 轮内增量落盘——按工具步骤边界提交，而不是只在轮末

Status: proposed

## Problem

稀疏会话（`store_sparse`）当前把整轮事件累积在内存，**只在轮末**经
`commit_boundary`（`label=save_to_backend_sparse`）一次性提交。现场证据（`pa_01a1df2f-8315`）：

- 轮首 `targeted_session_not_found: session=… — 视同不存在（不创建空会话）`；
- 整轮耗时 16.9 min、35 次工具调用，期间磁盘上**没有任何该轮记录**；
- 直到轮末 flush 才出现 `history_rebuilt_from_turn_events: 1 个投影空会话由事件日志重建历史`。

后果：进程崩溃 / 被强杀 / 机器重启 ⇒ 整轮（含已执行的工具步骤与已完成的诊断）**全部消失**，
用户重进会话看到的是「这轮从没发生过」。这与「中文 TUI 长达十几分钟的排障轮」是天然高危组合。

## Proposal

1. **提交节奏改为「工具步骤边界」**：每完成一次 tool_result（成功或失败）触发一次
   **增量提交**，而不是按时间轮询——工具边界是确定性事件、频率天然有界（一轮 35 次工具
   ⇒ 最多 35 次提交，远低于逐 delta 提交的量级）。
2. **新增显式 API**：`SessionStore::flush_turn_events_incremental(session_id, turn_id)`，
   语义 = 「把该轮已发生的事件追加到事件日志（幂等，按 `(session_id, turn_id, seq)` 去重）」，
   由 turn 循环在工具结果落定后调用；失败只降级为 warn，不阻断本轮。
3. **幂等锚点复用 B222**：历史行身份已按 `turn_id` 判重，事件追加同样以 seq 为身份，
   重复 flush 不产生重复事件；轮末的整轮 commit 仍是权威收口（增量提交只是把「已发生」
   提前落盘，不改变最终状态机）。
4. **性能预算**：单次增量提交 ≤ 5ms（工具边界处已有人在等工具结果，不新增可感知延迟），
   超过即降级为「每 N=5 次工具再提交」，并把实际节奏写进运行日志一行。

## Why not implemented in this wave

- 需要新增 store 层 API + backend 事件追加路径 + 水位（`event_watermark`）簿记，
  与既有 `commit_boundary` / lazy commit (`lazy_commit_sessions`) 的交互必须逐条验证；
  在共享工作区里与「TUI 任务可见性重构」并行修改 `tui.rs`/`store.rs` 会制造写冲突。
- 缺少压测数据支撑「≤5ms/次」：必须先有 commit 边界的耗时基线（现有日志只覆盖轮末 commit）。
- 与 B217 的 `TurnEndReason::Interrupted` 写侧是**同一条恢复链**：崩溃后要能把「开放轮」
  补写成 `Interrupted` 历史行，需要增量落盘先存在，否则恢复路径无事件可读。故 B209
  应作为独立波次整体交付（增量落盘 + 恢复补写 + 压测基线），而不是拆成两个半成品。

## Alternatives considered

- **按时间轮询（每 2s flush）**：与事件频率无关、空闲轮也写盘；且抖动不可预测。否决。
- **逐 `assistant_chunk` 落盘**：一轮可产生上千次写，稀疏存储的原子替换成本不可接受。否决。
- **只把「最终检查点文本」周期性落盘**：丢掉的正是「已执行了哪些工具」这类过程事实，
  而这恰是用户/续跑最需要的诊断线索。否决。
- **依赖 SQLite WAL 自动提交**：提交边界不是事务边界，`span`/事件仍在进程内存里，
  崩溃时 WAL 里也没有未提交的内存内容。否决。

## Consequences（预期）

- 崩溃窗口从「整轮」缩到「最近一个工具步骤」；`/resume` 能看到已执行的步骤清单。
- 恢复路径可据此补写 `TurnEndReason::Interrupted`（B217 已立法的 wire 值）。
- 需要一条新的压测/验收：`tests/` 下模拟「工具完成后强杀」，断言事件日志已含该工具步骤。

## Related

- B209（本源任务，仍为 `todo`）
- `2026-11-17-turn-end-reason-interrupted.md`（恢复补写的终态口径）
- B222（历史行按 `turn_id` 幂等，本方案复用它作为去重锚点）
