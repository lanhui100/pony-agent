# Agent Note: 轮末过程摘要落到消息区（耗时是持久信息，不是进行中状态）

Status: implemented

## Problem

会话 `pa_01a1df2f-8315-7090-8a4c-978934c4d9cb` 的实测证据：
首轮 20:29:57 → 20:46:50，`turn_duration_ms=1013009`（16.9 分钟）、26 次 provider 调用、
35 次工具调用；`turn_end reason=completed` 之后，TUI 的状态行渲染门（`turn_lifecycle`）
立刻回到 `Terminal`，进行中的那一行消失，界面上**不留任何**「本轮用了多久 / 跑了多少工具」。

后果有两层：

1. **可回看性缺失**：耗时与工具次数只留在 `session_turn_traces` / `provider_usage` 里，
   用户与后续诊断都要回到日志侧才能取到；同一会话的第二轮提问只能读到「上一轮诊断未完成」，
   无法从界面判断上一轮究竟跑了多久。
2. **误判放大器**：长轮次结束得「和刚开始一样安静」，与 B197/B198 的进行中静默叠加后，
   用户把「轮次还在跑」读成「会话无故停止」，又把「轮次已收尾」读成「什么都没发生」。

状态行与轮末摘要解决的是**两个不同问题**：状态行回答「现在在干什么」（易失），
摘要回答「上一轮干了多久、跑了多少」（持久）。用同一个组件承载会逼迫状态行在终态继续
驻留，与「终态不渲染假进行中」的既有契约（ADR 0043 / B218）直接冲突。

## Decision

1. **纯函数在展示层**：`bin/tui/status_dwell.rs::settled_turn_summary(outcome, elapsed_ms, tool_calls)`，
   终态口径 `SettledTurnOutcome::{Completed, Cancelled, Failed}`：
   - `Completed`：`本轮完成 · 用时 <d> · 工具 <n> 次`；
   - `Cancelled` / `Failed`：`本轮已停止/失败 · 工具 <n> 次`，**不显示耗时**；
   - `tool_calls == 0`：省略工具段（不渲染空计数）。
   耗时格式化复用 `format_elapsed_seconds`（1 秒/分秒/整分三档，与状态行同一真源）。
2. **写入点只有三个终态事件**：`UiEvent::StreamCompleted` / `StreamFailed` / `StopTerminal`，
   统一走 `TuiState::push_settled_turn_summary(outcome)`；不新增事件通道、不在渲染帧里推算。
3. **锚点即闸门**：方法以 `turn_started_at.take()` 为前置条件——
   - 无锚点（会话装载 / 历史重放 / 非真实开轮）⇒ 直接返回，不得凭空冒出「本轮完成」；
   - 同一轮的重复终态（`stop` → `completed` 竞态、重复回执）⇒ 只写一行，不重复记账。
4. **摘要落消息区而非状态行**：作为 `MessageRole::System` 单行进入 `state.messages`（UI-only，
   不写回后端会话历史，不进模型上下文），`auto_scroll` 置真保证用户看得到收尾。

为什么不用状态行驻留：终态行一旦驻留就会与「本轮已结束」的语义打架（B218 已把终态排除在
`Running` 之外），而且状态行只有一行，容纳不下「耗时 + 工具数 + 终态」三段事实。

## Alternatives considered

- **终态时让状态行驻留若干秒（TTL）**：实现最省，但把持久信息降级为临时提示——用户滚动或
  切会话后再也读不到；且 TTL 与 `status_dwell` 的 min-dwell 语义纠缠，容易退化成「假进行中」。
- **写进 `status_text` 由状态行渲染**：同上，且 `status_text` 已被失败摘要占用（错误只出现一次
  的契约），再塞摘要会互相覆盖。
- **只落日志 / trace 不落界面**：等于维持现状，`session_turn_traces` 本来就有数据，
  缺的正是「用户可回看」这一层。
- **把摘要写回后端会话历史（持久化到 store）**：会让摘要进入下一轮模型上下文，成为纯噪音，
  且需要新的历史消息种类与迁移；当前证据只要求「界面可回看」，UI-only 已满足，后续若需跨重启
  回看应作为独立 ADR 处理（与 DSH 的 turn-process 节点持久化对齐）。
- **错误/停止轮也显示耗时**：会误导成正常完成（DSH 明确不在 `stopped/error` 显示时长），
  且中断轮耗时的解释力取决于中断点，单值呈现反而制造误读。

## Consequences

- 收益：每轮结束界面上留一行可回看的过程摘要；轮末不再是「什么都没发生」。
- 代价：消息流多一行 `System`；`turn_started_at` 由「终态不清空」改为「终态 `take()`」，
  语义更严格（重复终态不再可能重复记账），与 B218 的注释同步更新。
- 验收（机器证据）：
  - `cargo test -p pony-agent-core --bin pony settled_turn_summary` = 4 passed
    （完成带耗时/工具数、停止与失败隐藏耗时、零工具省略、缺锚点不伪造耗时）；
  - `cargo test -p pony-agent-core --bin pony running_status_visibility_tests` 与
    `provider_retry_status_tests` 保持全绿（状态行契约不被摘要改动牵连）。
- 对齐面：deepseek harness `packages/client/ui-chat/src/client/chat/TurnProcessNodeView.tsx`
  —— `turn.status === 'closed'` 后渲染 settled duration，`stopped/reason === 'error'` 不显示时长。
