# Agent Note: `llm.chat` span 时长重基到成功 attempt（B135 落地）

Status: implemented

## Problem

`llm.chat` span 的 `llm.call.duration_ms` / `llm.call.duration_generation` 取自
`start.elapsed()`（外层耗时），因此在发生过重试时**包含退避等待与失败 attempt 的耗时**。
一次 3 次尝试、退避 20s+40s 的调用会被记成 60s+，与模型真实延迟无关——这样的数不能用来
回答「模型多慢」，也不能拿去和别的模型比较。

根因不是「没人想过要纯净窗口」，而是**账本是一次性的**：

- `agent::runtime` 的记录构建走 `retry::take_pure_call_window`，它调用
  `take_provider_attempt_window()`——**取走即空**（`retry.rs:1312`）；
- `turn_flow` 的 6 处 span 记账（3191 / 3317 / 3404 / 3476 / 3586 / 3691）在之后运行，
  此时账本已被取走，只能退化为外层耗时；
- 也就是说 per-call 记录（`turn_duration_ms`）已经是纯净的，**只有 span 侧不纯净**——
  两条观测口径在同一个调用里对不上。

## Decision

1. **给账本加只读窥视**：`retry::peek_provider_attempt_window()`（`snapshot()` 后不 take），
   以及 `retry::rebase_timing_to_pure_call(duration, ttft) -> RebasedTiming`（含
   `duration_ms` / `first_token_ms` / `retry_wait_ms`）。取走语义不变，记录构建零改动。
2. **在 `LlmSpanGuard::record_call_timing` 内部重基**（唯一入口）：
   `call_duration_ms` / `call_ttft_ms` 都替换为重基后的值，`retry_wait_ms` 单独保存。
   **6 个调用点一行不改**——它们传的仍是外层耗时，口径统一收在 guard 里，避免「六处各写一遍」。
3. **等待单列成属性** `llm.retry.wait_ms`（`ATTR_LLM_RETRY_WAIT_MS`）：仅当 `> 0` 才输出，
   无重试 ⇒ 无键（禁止 0 占位冒充真值）。`duration_ms + retry.wait_ms` 即外层总耗时，
   所以「模型多慢」与「用户等了多久」都能答。
4. **turn 级累加同口径**：`accumulate_active_turn_call` 改用重基后的值，
   否则根 span 的 `agent.turn.duration_ms` 与子 span 之和会对不上。
5. **无账本 ⇒ 原样返回**：三方直接调用 provider 或测试桩不开启账本时，不重基、不产出等待键，
   绝不臆造「零等待」。

## Alternatives considered

- **在 6 处调用点各自 peek 后传参**：能实现同样效果，但等于把同一个口径抄 6 遍，
  新增第 7 个 provider 路径时必然漏掉（这正是 B135 最初形态）。
- **把 `take_provider_attempt_window` 改成 peek + 手动清理**：会打破「一次性台账」的既有语义，
  记录构建侧与 span 侧谁先跑就决定谁拿到窗口——顺序依赖变成隐性契约，比现在更难维护。
- **改用 `saturating_sub` 强行夹到 0**：账本失真时会伪造「零延迟」，比属性缺席更危险；
  ADR `2026-11-22-upstream-status-and-span-duration-evidence` 已明确拒绝。
- **只加 `retry.wait_ms` 属性、duration 保持原值**：能查到等待，但所有下游（Langfuse/Tempo 面板、
  速率比较）默认读 `duration_ms`，不重基等于让错误口径继续被消费。
- **在 span 里自己重新计时（去掉 sleep 的 span 边界）**：需要把 span 生命周期切进重试循环内部，
  改动面远大于读一份现成账本，而且会丢掉「整轮耗时」这个真实指标。
