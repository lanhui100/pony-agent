# Agent Note: followup 重试的转录重复追加与上游恢复窗口脱钩（B1795488338107 / B1795488338167）

Status: implemented

## Problem

两个 P1/P2 待办在同一段重试编排上相邻，且互为放大器。

### B1795488338107（Defect · P1）：重试导致上下文堆积与请求体积几何膨胀

**现场**：轮次失败后简单问答请求膨胀到 300KB+，高频重发触发 EdgeOne 边缘防刷
RST 拒连（10~13ms 报错）。

**根因（本条）**：followup 的转录构造器在**每次尝试内**把
「assistant(tool_calls) + tool(result)」写回 `accumulated_messages`
（`openai_messages_with_tool_result` / `anthropic_messages_with_tool_result`）。
同步 1:1 与同步批量两条臂已在 PA-174 F1 / PA-173 D3 加了「每次尝试前重置为
pristine 快照」，**流式 1:1 臂从未加**（`continue_with_tool_result_stream_bounded`
的 `retry_provider_followup_stream_timeout_with_policy` 闭包直接复用
`accumulated_messages`）。于是第 N 次尝试以「已含 N-1 次追加」的转录为 base 再追加
一次：转录条数按 2N 增长，而 `tool_result` 单条最大可达 `PruneBudget::PROVIDER_FOLLOWUP`
的 12k 字符 ⇒ 请求体积随重试次数**线性翻倍**，不是「多一点点」。

另一条独立放大器：`native_transcript_for_*_turn` 与
`provider_native_tool_result_message` 都用**未修剪**的原始 `tool_result.output`
写落盘转录；跨轮回放侧（`load_native_transcript_messages`）才做
`prune_native_messages_in_place`。这意味着膨胀一旦写进会话库，每轮都要先读全量再修剪，
且「已膨胀」的会话没有任何自愈入口。

### B1795488338167（Debt · P2）：退避与上游窗口脱钩，流中断无断点

**根因**：`Retry-After` 在 HTTP 出口已被折叠成 `retry_after=` 结构化后缀
（`append_retry_after_marker`），冷却登记（`enter_cooldown`）也按它算窗口；但重试
调度侧三处 `decide(...)` 调用**恒传 `None`**，且循环体 sleep 用的是自算的
`compute_delay(attempt, ...)`——`decide` 里那段消费 `RetryAfter` 的代码**没有任何出口**。
净效果：上游说 90s 后恢复，调度仍按 0.5/1/2/4s 短退避硬撞（必然再失败），
或按固定 20s→40s 长退避白睡两轮后仍失败。

## Decision

1. **流式 1:1 臂补 pristine-base 重置**（与同步 1:1 / 同步批量同构）：
   进入入口时快照 `base_messages`，闭包首行 `accumulated_messages.clone_from(&base_messages)`。
   成功 = base + 恰好 1 次追加；失败 = 停在最后一次尝试写入后的转录（与既有两臂同语义，
   无回滚——调用方拿到 `Err` 后不得再把该转录当干净 base）。
2. **`RetryAfter::window_ms()` + `retry_after_from_error()`**：
   调度侧消费上游窗口的唯一接缝，解析仍只经既有唯一解析点 `parse_retry_after_marker`
   （只读头部段，正文劫持不生效）。不新增关键词表、不改错误文案。
3. **`decide` 的窗口语义**：
   - 窗口 > 剩余预算 ⇒ `Abort`（快速失败），由 provider 冷却（已按同一窗口登记）与换手接管；
   - 窗口 ≤ 剩余预算 ⇒ `Retry { delay_ms: window.max(base_delay).min(remaining) }`；
   - `RetryMode::Always` 同样尊重窗口（否则「等到成功」变成「按 8s 封顶反复撞同一窗口」）。
4. **`RETRY_AFTER_WINDOW_MAX_MS = 10min`**：单次等待封顶（**不是**预算上限）。
   上游可回任意秒数，不封顶等于把 wall-clock 交给上游。
5. **循环体只消费 `decide` 的判决结果**（`pending_delay_ms`），不再自算延迟、也不再
   自判预算——**判决点唯一**。无窗口时 `decide` 内部即 `compute_delay`，故既有
   `20s→40s`（rate-limit）与 `0.5/1/2/4/8s`（普通）契约逐字不变。

> **评审修正（同日）**：初版在循环体里另加了一处「窗口 > 剩余预算 ⇒ `return Err`」的
> 早退，并把 sleep 写成自算的 `compute_delay(...).max(window)`。独立评审（Antigravity）
> 判定为 blocker，复核成立，已改回上面的「单一判决点」：
> - 早退位于 RL 预算切换与 `decide` **之前**，`RetryMode::Always` 根本到不了 `decide`
>   的 Always 分支（「等到成功」被首跳枪毙）；
> - 早退用切换前的短预算（默认 60s）判生死，抢跑了 rate-limit 臂的 90s 预算；
> - 自算 sleep 未 `.min(remaining)`，`remaining=10s / window=5s / curve=20s` 时会睡 20s
>   超过剩余预算，且丢弃了 `decide` 已算好的 `delay_ms`。
>
> 同时修正 `decide` 的 `attempt` 口径：该参数是 **1-based 的下一次尝试序号**
> （`compute_delay(0)` 恒为 ZERO），初版传 0-based 旧值，把冻结的「首跳 500ms」
> 打成 0ms——由既有用例 `http_525_enters_retry_loop_and_succeeds_on_second_attempt`
> 与 `single_hop_report_success_after_timeout_retry` 抓红，已改为 `attempt + 1`。

## Alternatives considered

- **流式臂直接改成「重试不重置、改用局部 messages 变量」**：会改变 `accumulated_messages`
  的对外契约（runtime 侧 `turn_stream.rs` 依赖它跨 hop 累积），需要同时改三处调用面与
  `tool_recovery` 的恢复路径；而同步两臂已经用「pristine 重置」解决了同一问题，
  再造第二套机制只会让三条臂语义分叉，否决。
- **把 `tool_result.output` 在构造时就修剪（`prune_tool_result_default`）**：会与
  `PruneBudget::PROVIDER_FOLLOWUP`（12k/4.5k/2.5k）的既有契约冲突，且丢失「原始输出」
  的唯一真值——模型看到的是摘要，后续若需要精确定位行号就无从恢复，否决。
- **重试时整段丢弃 `accumulated_messages`（重置为空）**：等于丢掉本轮已发生的全部工具往返，
  模型会重复调用同一工具（既有 `same_signature` 熔断会命中），否决。
- **把窗口当作「唯一延迟来源」（`decide` 给多少就睡多少）**：会让无窗口时的首跳
  sleep 变成 0（`compute_delay(0) == ZERO`），破坏既有 20s→40s 契约；改用「取较大者」
  让窗口只能抬高不能压低，否决「唯一来源」方案。
- **窗口超预算时 `Escalate` 而非 `Abort`**：`Escalate` 在既有语义里表示「预算耗尽、交上层」，
  而这里的语义是「上游明确说等不到」，用 `Abort` 才能让 provider 冷却/换手立即接管；
  且循环体对 `Escalate` 与 `Abort` 的处理逐字相同（都返回 `Err`），选语义更准的一个。
- **把窗口直接写进 `active_config.max_delay_ms`**：会污染配置（配置是持久化的用户意图），
  且每次重试都要重算配置，否决。
- **为「流式中途断开」单独造断点恢复机制（P2 后半）**：现有机制已经覆盖——流式臂
  `StreamState` 在发出可见增量后禁止自动重试（`may_auto_retry`），失败轮由
  `native_transcript_for_failed_turn` 写入「已闭合的工具往返 + 中断标记」，
  下一轮模型据此知道上一轮停在哪。真正的缺口是「重试时把半状态当完整状态重放」，
  已由本条的 pristine 重置修掉；再造第二套断点会与 transcript 形成两份真值，否决。

## Consequences

- 流式 followup 重试不再导致转录 2N 增长：`streaming_followup_retry_does_not_duplicate_accumulated_messages`
  用回环 SSE 服务（首连 503、次连 200）断言最终转录 = base + 2。
- 上游窗口进入实际 sleep：`followup_backoff_honors_upstream_retry_after_window`
  断言 `retry_after=8` 时 slept ≥ 8s。
- 窗口装不进预算 ⇒ 首跳收口：`followup_aborts_fast_when_retry_after_exceeds_budget`
  断言 slept = 0、attempts = 1。
- `RetryMode::Always` 不得被预算早退枪毙：`followup_always_mode_waits_through_window_larger_than_budget`
  断言预算 60s + 窗口 90s 时仍 sleep 90s 后成功（评审 blocker 的回归锁）。
- 三个窗口用例均已做 **fail-first 验证**：临时禁用窗口消费后分别红在
  `500 vs 8000`、`60000 vs 0`、`20000 vs 90000`（不是「删掉逻辑仍然绿」的假绿）。
- **本条的适用范围（诚实收窄，评审 finding 3）**：真实 HTTP **429** 在 4 个 HTTP 出口
  会先经 `enter_cooldown` 打上 `provider_cooldown_active` 标记，重试循环对带该标记的
  错误**全 scope 立即 Abort**（冷却期 0 次 HTTP，PA-171 C3 冻结契约）——因此 429 走的是
  「冷却登记 + 短路」，**不会**进入本条新增的窗口等待。本条真正生效的场景是
  **503/5xx + `Retry-After`**，以及任何未触发冷却的可重试失败。429 的「短窗口是否值得
  在预算内等待」属于冷却短路与窗口等待的边界设计，不在本条范围，另立待办。
- **未在本条收口（登记后续）**：落盘转录（`provider_native_tool_result_message`）仍存
  未修剪原始输出，跨轮回放才修剪——「已膨胀会话」缺少一次性自愈入口；单请求体积
  也仍无硬上限（只有 token 预算）。这两项需要独立的请求体积闸门设计。
- 证据：`cargo test -p pony-agent-core --lib` 1668 passed / 0 failed；
  `npm run rust:lint` PASS（新增/恶化 0）；`cargo check -p pony-agent-core --lib` exit 0。
