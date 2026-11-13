# Agent Note: 速度口径可取证、重试窗口纯净与读循环去阻塞

Status: implemented

## Problem

用户报「同一模型在 DSH 显示 200+ t/s，在 Pony Agent 只有 60-70 t/s」，三路取证（上游 58 次
直连实测 + 本机会话库 2903 条 per-call 复算 + DSH 侧公式取证）后确认这是**三个独立缺陷叠加**，
而不是 Pony 真的更慢：

1. **口径不可取证**：轮级 `turn_speed_tokens_per_second` / `turn_speed_basis` 只进 OTel 根 span
   属性（`agent/otel/mod.rs::ATTR_AGENT_TURN_SPEED_TOKENS_PER_SECOND`），而本机 OTel 导出长期
   401 / TimedOut；`sessions.db` 无列、日志无行。于是「显示值到底怎么算的、样本多大」在事后
   **无法离线复核**——这正是争论无法收敛的根因。
2. **重试污染窗口**：per-call 的 `dur` / `ttft` 取样点在重试循环之外
   （`runtime/turn_stream.rs:2174/2395/978`、`runtime/tool_recovery.rs:19/103`），429/525 失败尝试
   （实测 1.9–8.0s）被算进窗口。`gen = dur − ttft` 因两端同源近似抵消，但 `dur` 单独当分母的
   `end_to_end` 口径被系统性压低（同库 gemini：decode 98.6 t/s vs e2e 10.4 t/s）。
3. **读循环阻塞节流**：`on_delta` 在阻塞 SSE 读循环内联执行（`provider/openai_sse.rs:289 → :133`），
   而节流用 `std::thread::sleep(16ms − gap)`（6 处：`runtime/turn_stream.rs:1078/1158/2283/2333`、
   `runtime/tool_recovery.rs:214/244`）。睡眠即停止读 socket ⇒ TCP 背压 ⇒ 上游被迫降速；
   突发路由（1 token/chunk、到达间隔中位 0ms）下会把 token 率硬顶在 ~62.5/s。

## Decision

**D1 口径落盘（可取证）**：新增落盘事件 `TurnEvent::TurnSpeed`（type=`turn/speed`），在 `turn/end`
（terminal → flush）之前派发，保证与整批终态同事务落盘；键为
`turnSpeedTokensPerSecond` / `turnSpeedBasis` / `turnCallsTotal` / `turnCallsEligible` /
`turnBurstExcluded`，另加 `turnExcludedTotal`（valid − eligible）与 `turnCallsTtftMissing`
（valid − 有 ttft），把「突发整包剔除」与「无首 token 不可测」分开。同帧打一行结构化日志
`turn speed settled`。无值一律整键缺席，禁止 0/NaN 占位。样本数唯一出口
`agent/turn_flow.rs::turn_call_census`（只复用 `call_valid` / `decode_eligible` / `decode_floor_ms`，
不新建判据）；速度唯一出口仍是 `weighted_turn_speed`。

**D2 显示诚实化**：TUI 指标段与桌面状态面板必须表达「样本量 + 口径」，突发剔除 >0 时可辨别
（TUI 后缀 `剔突发N`；桌面 tooltip + `data-speed-burst-excluded`）。无值仍不占位。
显示端禁止另写公式（契约 I9 不变）。

**D3 窗口纯净**：新增重试账本 `agent/retry.rs::ProviderAttemptWindow`
（`attempts` / `failed_attempt_ms` / `slept_ms` / `succeeded_attempt_ms` /
`succeeded_attempt_offset_ms`），由重试循环逐 attempt 记账（`provider/mod.rs` 调度语义零变化），
调用方以 `begin_provider_attempt_window()` 与 `take_pure_call_window()` 把 per-call 的 `dur` / `ttft`
收敛到「最后一次成功 attempt」。`attempts` 与失败耗时走独立结构化日志
`provider_call_retry_window`，**绝不进入** `dur` / `gen`。速度公式零改动。

**D4 读循环去阻塞**：删除 6 处 `thread::sleep`，改为 `runtime/stream_support.rs::StreamDeltaCoalescer`
（16ms 时间窗）：读线程每个 delta 只做一次拼接 + 一次时间戳比较，窗口内合并、窗口外上屏，
收尾 `flush` **无条件**执行（不丢最终帧与终态）。帧形状与旧实现逐字一致（先 reasoning 后 text，
合并只在同类型内）。`sink.emit` 允许内联，前提是 sink 契约为非阻塞投递
（Tauri `emit` fire-and-forget、TUI 无界 channel、headless 无消费者）。

## Alternatives considered

- **A：只改显示文案/加 `≈` 前缀，不落盘** —— 观感可解释但事后仍不可复核，下次争议无从收敛；否决。
- **B：把 `emit` 完全移出读循环（独立发送线程 + 有界队列 + 丢中间帧）** —— 本轮否决：几十行并发改造，
  引入丢帧/终态竞态风险，收益仅在「sink 本身慢」这一场景，而生产 sink 均非阻塞；登记为 backlog
  待解耦项（`.dev-team/diag-tps/fix/00-DELIVERY.md` §6.2）。
- **C：端到端统一改为「首 token 后到流结束」的 DSH 口径（不减 1、无下限）** —— 否决：会把
  「突发整包」算出 400–1521 t/s 的虚高值，等于用不可信的采样冒充可比解码速度。
- **D：修 per-call `dur` 而不引入账本（直接改取样点位置）** —— 否决：取样点分散在 4 条流式/同步路径，
  逐处挪动无法覆盖退避 sleep 与换手；账本是唯一能同时拿到 `attempts` / 失败耗时做留痕的载体。
- **E：把节流窗口从 16ms 调到更小以求更快** —— 否决：瓶颈是「阻塞在 sleep」而不是窗口长度，
  调小只是把 62.5/s 提到 125/s，仍是客户端限速。

## Consequences

- 正面：显示值从此可离线复核（`sessions.db` 事件 + 日志双载体）；`end_to_end` 不再被失败尝试污染
  （物理探针 2600ms → 500ms，5.2×）；读循环不再因客户端节流自我限速。
- 负面/已知限制：
  1. 慢 sink 仍会串进读循环（`emit` 内联），本 ADR 以「sink 契约非阻塞」为前提接受，解耦另立单。
  2. `llm.chat` span 的 per-call `dur` 仍含重试等待——attempt 账本是线程局部**一次性**台账，
     在 `turn_flow` 内取走会让 D3 的 per-call 记录退回脏值；正确修法是把已结算窗口传进 span
     或 retry 提供 peek 接口，另立单。
  3. 新增事件类型属对外契约增量：历史回放/导出消费面需识别 `turn/speed`（既有用例期望已在同一
     提交内更新）。
- 可见性：`turn/speed` 与 `turn speed settled`、`provider_call_retry_window` 增加了一条后台任务/LLM
  调用面的事实，`docs/observability.yaml` 的声明需同步更新（排在本提交之后、待该文件上的并行
  WIP 落定）。
- 关联：`.dev-team/contract-matrix-token-speed-v2.md`（I1–I10 口径不变量仍在效）；同族诚实化决策见
  `.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md`（该条已声明「速率口径属
  实现选择、不动」，本条目承接的是「让它可被取证 + 修窗口污染 + 去阻塞」）。
