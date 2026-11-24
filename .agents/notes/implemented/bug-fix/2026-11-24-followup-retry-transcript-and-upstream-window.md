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
- **为「流式中途断开」单独造断点恢复机制（P2 后半）**：失败轮由
  `native_transcript_for_failed_turn` 写入「已闭合的工具往返 + 中断标记」，下一轮模型据此
  知道上一轮停在哪——这层持久化真值**不新增第二套**。真正的缺口是「重试时把半状态当完整
  状态重放」，已由本条的 pristine 重置修掉；再造第二套断点会与 transcript 形成两份真值，否决。
  > **二轮评审修正（同日）**：初版在此写的「流式臂 `StreamState` 在发出可见增量后禁止自动
  > 重试（`may_auto_retry`），现有机制已覆盖」与代码事实**不符**——重试循环两处 `decide`
  > 恒传 `StreamState::NoDelta`，流式臂的 `no_more_retries_hint` 只查用户取消（生产路径
  > `without_cancel()` 恒 false）⇒ SSE 发出半截正文后断流仍会重试，把 attempt 1 的增量与
  > attempt 2 的输出重复上屏。已按评审 blocker 真实接线（见下「Decision 6」），
  > ADR 论断随之改正：**「不另造持久化断点」成立，「现有机制已覆盖」不成立**。

## Decision（续）

6. **`emitted_visible` 真实接线（二轮评审 blocker）**：`continue_with_tool_result_stream_bounded`
   与 `decide_with_tools_stream_bounded` 各自持有跨 attempt 累计的
   `emitted_visible: Cell<bool>`，包装 `on_delta` 在收到 `ProviderStreamChunk::Text(_)` 时置位，
   并把 `|| emitted_visible.get()` 接入 `no_more_retries_hint`
   （decision 臂由 `retry_provider_decision_stream_with_policy` 提供带 hint 的入口）。
   只把 `Text` 计入：`Reasoning` / `StreamStart` 按既有 `may_auto_retry` 口径仍允许重试。
7. **`notify` 消费真实退避值（二轮评审 M3）**：`RetryNotifier::notify` 新增 `delay_ms` 入参，
   由调用方传入 `RetryDecision::Retry { delay_ms }`（或短窗口等待值），不再内部自算
   `compute_delay`——否则上游 `Retry-After: 90` 时实际睡 90s、`llm/retry` 却报 500ms。
   同时 `hint_blocking` 改为**失败当场早退**，不再先走 `decide` 发一条假重试事件再在下一轮
   顶部被拦下。
8. **`Normal` 分支用未截断的 `raw_window` 判生死（二轮评审 M1）**：先判
   `raw_window > RETRY_AFTER_WINDOW_MAX_MS`（超封顶即 Abort，诊断报原始值）与
   `raw_window > remaining`，通过后才算 `raw_window.max(base_delay).min(remaining)`。
   旧写法先 `.min(cap)` 再比较，`budget=700s + Retry-After=900s` 会「装得下」而白睡 10min。
9. **`Always` 分支窗口只能抬高曲线（二轮评审 M2）**：取
   `window.max(base_delay_ms).min(RETRY_AFTER_WINDOW_MAX_MS)`。旧写法直接返回窗口值，
   上游回 `Retry-After: 1` 会把已抬起的 20s RL 曲线压回 1s。
10. **`retry_with_policy` 的 `decide` attempt 统一 1-based（二轮评审 N2）**：循环内 `attempt`
    是 0-based，`decide` 期望「下一次尝试序号」⇒ 传 `attempt + 1`。该函数不消费判决出的
    `delay_ms`（退避在循环顶部自算），故不改任何 sleep 数值，只让判决面口径唯一。

## Consequences

- 流式 followup 重试不再导致转录 2N 增长：`streaming_followup_retry_does_not_duplicate_accumulated_messages`
  用回环 SSE 服务（首连 503、次连 200）断言最终转录 = base + 2。
- 上游窗口进入实际 sleep：`followup_backoff_honors_upstream_retry_after_window`
  断言 `retry_after=8` 时 `slept == 8_000` 且 `attempts == 2`（第 2 跳成功，精确值而非 `>=`）。
- 窗口装不进预算 ⇒ 首跳收口：`followup_aborts_fast_when_retry_after_exceeds_budget`
  断言 slept = 0、attempts = 1。
- `RetryMode::Always` 不得被预算早退枪毙：`followup_always_mode_waits_through_window_larger_than_budget`
  断言预算 60s + 窗口 90s 时仍 sleep 90s 后成功（评审 blocker 的回归锁）。
- **可见增量后断流不得重试（二轮评审 blocker 的回归锁）**：
  `stream_retry_hint_blocks_retry_after_visible_delta` 是**端到端**用例——回环服务声明
  `Content-Length: 65536` 却只发一小段后断连（`error decoding response body`，在册可重试
  传输错误），断言 `visible_chunks == 1` 且**只建立 1 次连接**；反向钉子
  `stream_retry_hint_allows_retry_when_no_visible_delta` 锁住「无可见增量时仍须重试」，
  防止「把 hint 写成恒 true」的假绿。fail-first 已验证：移除 `emitted_visible.get()`
  接线后该用例红在 `left: 3, right: 1`（重试 4 次、累计 3 条可见增量）。
- **二轮评审 M1/M2/M3 的边界锁**：`decide_aborts_when_window_exceeds_single_wait_cap_using_raw_value`
  （诊断必须报原始 `1800000ms`）、`always_mode_window_only_raises_never_lowers_curve`
  （`left: 1000 / right: 20000`）、`retry_notice_reports_actual_delay_not_recomputed_curve`
  （事件 `delay_ms == 90000`）。三者均做过 fail-first：分别红在上述数值。
- 三个窗口用例均已做 **fail-first 验证**：临时禁用窗口消费后分别红在
  `500 vs 8000`、`60000 vs 0`、`20000 vs 90000`（不是「删掉逻辑仍然绿」的假绿）。
- **429 的适用范围已从「一律短路」扩展（B1795506800001 落地）**：真实 HTTP 429 仍在 4 个
  出口先经 `enter_cooldown` 打标，但标记值现在区分**来源**
  （`CooldownMarkerSource::{Entered, ShortCircuit}`，标记落在头部段、只读头部段解析，
  正文不可劫持）。`Followup` 作用域在**全部**满足下列条件时允许在预算内等过冷却：
  来源 `entered`（本轮真收到过响应）、分类 `rate_limited`（排除 quota）、
  `0 < Retry-After <= SHORT_429_WAIT_MAX_MS`(30s)、`窗口 + COOLDOWN_EXPIRY_SLACK_MS`(250ms)
  装得进剩余预算、本跳等待次数 `< SHORT_429_MAX_WAITS`(2)。等待**不切入** 20s→40s 盲猜曲线
  （否则 `Retry-After: 8` 会被 `window.max(20_000)` 强拉成 20s），且醒来时注册表窗口已
  自然过期 ⇒ 全程不给 `ensure_not_cooling` 开后门。`Decision` 作用域与 `short_circuit`
  来源仍维持「冷却期 0 次 HTTP」的 PA-171 C3 契约。
- **请求体字节闸门已落地（B1795506800002）**：`serialize_request_body_with_size_gate` 在 4 个
  HTTP 出口序列化后校验字节硬上限（纯文本 512 KiB / 含图 10 MiB，另 256 KiB 软告警只落
  warn 不拦截）。超限时**0 HTTP** 返回带 `payload too large` 的错误文本 ⇒ 经既有
  `UPSTREAM_CONTEXT_OVERFLOW_HINTS` 归 `UpstreamContextOverflow`（`NonRetryable`）⇒
  不会把同一超大包重放成边缘防刷风暴。**不在 HTTP 出口静默裁剪**：HTTP 层已脱离会话语义，
  硬剪会破坏 prompt cache 前缀、破坏 `tool_call`/`tool_result` 闭合（Anthropic 400）或
  误删 `reasoning_content`（DeepSeek 400）。配套把 `estimate_native_message_tokens` 的漏算
  补齐（`reasoning_content` / `tool_calls` / `tool_call_id` 均计入，无这些字段的消息口径不变）。
- **仍未在本条收口（登记后续）**：落盘转录（`provider_native_tool_result_message`）仍存
  未修剪原始输出，跨轮回放才修剪——「已膨胀会话」缺少一次性自愈入口（拟以「超大输出外置
  为 blob 引用 + 幂等瘦身入口」收口，见 B1795506800002 的后半）。
- 证据（二轮评审收口后的最终收据）：`cargo test -p pony-agent-core --lib`
  **1681 passed / 0 failed**（较首轮 1668 新增 13 条用例）；
  `npm run rust:lint` **PASS**（实测 3842 / 基线 3870，新增/恶化 0）；
  `cargo check -p pony-agent-core --all-targets` exit 0；
  `npm run adr:check` 与 `npm run backlog:gate` 均 PASS。
- **门禁口径注记**：`scripts/cargo-lane.sh test` 默认 180s 硬超时，全量 lib 单测实测约
  163s（冷编后更易越线）。收口时用 `PONY_TEST_TIMEOUT=600s` 重跑才拿到上面的 1681 计数；
  180s 余量偏小属既有待办（B1795484554592 已登记），本条未改。
