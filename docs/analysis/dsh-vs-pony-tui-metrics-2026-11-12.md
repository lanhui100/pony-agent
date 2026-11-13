# TUI / 桌面右上角指标 vs DeepSeek Harness：口径逐项对比（2026-11-12）

> 参考源码：`/home/dm/deepseek-harness`（DSH，commit `de5d733a18`）
> 被检源码：`crates/pony-agent-core/src/bin/tui.rs`、`crates/pony-agent-core/src/agent/{metrics_snapshot.rs,session/usage.rs,turn_flow.rs,provider/mod.rs,provider/openai_sse.rs}`、`src/lib/runtime/token-metrics.ts`
> 相关决策：[`.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md`](../../.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md)

## 1. 两侧指标体系

| 维度 | DSH | Pony |
|---|---|---|
| 指标真源 | `@deepseek-ai/dsh-token-meter`（session projection，durable） | `TurnMetricsSnapshot` + `SessionUsageLedger`（结算快照 + 会话台账） |
| 呈现位 | `packages/client/ui-chat/src/client/chat/StatsPills.tsx`（ActivityPill / UsagePill） | `tui.rs::metrics_segments` → header 右侧单簇 |
| 共享口径 | `billedInputTokens = uncachedInput + cacheRead + cacheWrite` | `context_input_tokens`（anthropic ⇒ input+cache_read+cache_creation；其余 ⇒ prompt_tokens） |

两侧的**分子分母语义等价**：都是「prompt 侧全量」。差异集中在呈现规则与时间维度。

## 2. 逐项对比

| 指标 | DSH 口径 | Pony 口径 | 判定 |
|---|---|---|---|
| 缓存命中 | `formatCacheHitPercent(cacheRead, uncached+read+write)`：整数四舍五入，**顶到 100 时自动加精度**（99.6 ⇒ `99.6`），仅 `cacheRead === prompt` 才 `100`（`token-format.ts`） | 会话台账 `Σhit / Σcontext`，`{:.0}%` 直出 ⇒ 99.6% 显示 **100%** | **缺陷（已修）**：虚报全命中 |
| 上下文占用 | `projectedTokens = max(0, pressureTokens + surfaceTokens - sampledSurfaceTokens)`（`usage-projection.ts`）＝**下一次请求**预估，含 compaction 负增量 | 最近一次调用的 `context_input_tokens` / 窗口 | **差异（未修，立项）**：工具结果追加后读数不前进 |
| 速度 | `decodeTokens / decodeMs`，decodeMs 为 step 解码壁时间（不含 TTFT），不减 1、无下限 | `(out-1) / max(200ms, 5%·dur)`（`turn_flow.rs::call_speed_from_raw` / `decode_floor_ms`） | **实现选择（不动）**：Pony 已由 ADR 立据，误差 <5% |
| 首 token | `ttftMs` 每 step 单独统计、可平均 | `first_token_latency_ms`：首个非 `StreamStart` 增量（含 reasoning）采样，call TTFT ≤ turn TTFT 不变式 | **等价** |
| 输出 | `outputTokens`，`reasoningTokens` 是其**子集**，仅全部 attempt 上报才给 | `completion_tokens`（DeepSeek 已含 reasoning），`reasoning_tokens` 单列 | **等价** |
| 输入展示 | `UsagePill` 展示 billed input **总额**，明细列 uncached/read/write/output | `↑in` = 上下文总量、`↓out` = 会话 Σ输出；**无「总计」**，且未命中量不可见 | **差异（记录，未改）**：可读性，不是数值错 |
| 会话累计方式 | projection 事件重放，`addReplacing` 保证 attempt 替换不重复计费 | `SessionUsageLedger` per-call Σ；`restore()` 用历史真值覆盖 | **等价** |
| 未知值处理 | 无值 ⇒ pill 不渲染 | 无值 ⇒ 段不渲染（v3 R11），越界 ⇒ `None` fail-closed | **等价（Pony 更严）** |
| 成本 | 独立 pricing 投影（`route-pricing.ts`） | `cost_usd` 逐条计价，不可计价不补 0 | 等价方向 |

## 3. 结论（为什么「看起来不准」）

1. **舍入虚报**（已修）：0.996 ⇒ 打印 100%，让「几乎全命中」被读成「全命中」。这是用户能直接抓到的数字错。
2. **时间维度错位**（未修）：右上角 `↑in` / `上下文%` 描述的是**上一次请求**的真实占用，而用户直觉是「现在还剩多少」。工具结果刚追加、下一跳未返回 usage 的窗口期内，Pony 冻结、DSH 前进。这是设计差异，补齐需要 surface 增量估算器。
3. **DeepSeek 原生 `prompt_cache_miss_tokens` 未被消费**（已修分母回退）：`prompt_tokens` 缺失的兼容网关会让命中率整段消失。
4. 速度/输出/TTFT 三项经逐条核对**不存在系统性偏差**，此前怀疑不成立。

## 4. 落地动作

- 诚实百分比：`metrics_snapshot.rs::format_ratio_percent` / `format_unit_fraction_percent`，TUI 两段 + 桌面 `cacheRatioPercent` 同规则。
- 分母回退：`extract_openai_usage` 在 `prompt_tokens` 缺失时用 `hit + miss`。
- 未落地（另立项）：`projectedTokens` 式上下文预估；`↓out`/`↑in` 之外的 billed-input 明细。
