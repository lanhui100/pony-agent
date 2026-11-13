# Agent Note: 指标百分比诚实化（部分命中不得显示 100%）

Status: implemented

## Problem

TUI 右上角指标（`crates/pony-agent-core/src/bin/tui.rs::metrics_segments`）与桌面端
（`src/lib/runtime/token-metrics.ts::resolveMetricsSnapshotDisplay`）都把手里的单位分数
直接乘 100 后**四舍五入到整数**：`format!("缓存 {:.0}%", ratio * 100)` / `Math.round(ratio * 100)`。
当会话累计命中率为 0.996（长前缀会话常态）时，UI 打印「缓存 100%」——把「几乎全命中」
**谎报成「全命中」**，这正是「右上角指标统计不准」的一类可复现来源：数字是舍入的产物，
不是统计口径的产物。

对照 `deepseek-harness`（DSH）的 token-meter：`packages/client/ui-chat/src/client/chat/token-format.ts::formatCacheHitPercent`
明确规定**部分命中绝不四舍五入到 100%**——整数舍入会顶到 100 时自动叠加精度（99.9x），
只有 `cacheRead === promptTokens` 才输出 100。DSH 的分母口径为
`uncachedInput + cacheRead + cacheWrite`（`StatsPills.tsx::billedInputTokens`），与 Pony 的
`context_input_tokens`（anthropic ⇒ input+cache_read+cache_creation；其余协议 ⇒ prompt_tokens）
语义一致，故差异只在**呈现舍入**，不在分子分母。

本轮对比还确认了三处**非缺陷**的口径差异，以及一处真实缺口，一并登记，避免后续误改：

1. 速率：DSH `decodeTokens / decodeMs`（不减 1、无下限）；Pony `(out-1) / max(200ms, 5%)`
   —— Pony 已由 `agent/turn_flow.rs` 的 `decode_eligible` / `decode_floor_ms` 立据，属实现选择，不动。
2. 输出：DSH `reasoningTokens` 是 `outputTokens` 的**子集**；Pony `extract_openai_usage` 取
   `completion_tokens`（DeepSeek 语义已含 reasoning），口径一致，不动。
3. 上下文占用：DSH 用 `projectedTokens = max(0, pressureTokens + surfaceTokens - sampledSurfaceTokens)`
   （`packages/llm/token-meter/src/usage-projection.ts`）——**下一次请求**的预估；Pony 用最近一次
   调用的 `context_input_tokens`。这不是 bug，但承认「工具结果追加后占用不前进」。补齐需要
   request-surface 增量估算器（新能力），单独立项，不在本条目内实现。
4. 缺口：DeepSeek 原生 `usage.prompt_cache_miss_tokens` 至今未被解析；`cache_miss_input_tokens`
   由 `input - hit` 反推（`runtime/stream_support.rs::derive_cache_miss_input_tokens`）。
   `prompt_tokens` 缺失而 hit/miss 俱在的网关（fixture `agent/runtime/tests.rs` 已长期使用
   hit+miss 形态）会得到 `input = None`，命中率整段消失。

## Decision

1. **新增单一真值** `crates/pony-agent-core/src/agent/metrics_snapshot.rs::format_ratio_percent(numerator, denominator)`
   与 `format_unit_fraction_percent(ratio)`：
   - `denominator == 0` 或 `numerator > denominator` ⇒ `None`（沿用 fail-closed，不 clamp）；
   - `numerator == denominator` ⇒ `"100"`（真全命中）；
   - 普通值 ⇒ 整数四舍五入（43% 语义不变）；
   - 四舍五入会顶到 100 而 `numerator < denominator` ⇒ 改用**两位小数向下取整并去掉尾随零**
     （`0.9995` ⇒ `99.95`，`0.996` ⇒ `99.6`，`0.9999` ⇒ `99.99`），保证**永不高报**。
2. TUI `metrics_segments` 的「缓存 x%」「上下文 y%」两段改调该函数，不再 `{:.0}` 直出；
   分母仍取会话台账 `Σ context_input` / 窗口（分子分母同源不变）。
3. 桌面端 `resolveMetricsSnapshotDisplay.cacheRatioPercent` 用 ratio 版同一规则（保 D1-4
   「四舍五入而非截断」：0.6667 ⇒ 67%）。
4. `extract_openai_usage` 在 `prompt_tokens` 缺失时回退 `prompt_cache_hit_tokens +
   prompt_cache_miss_tokens`（DeepSeek 原生字段），使命中率分母可成立；两字段都缺仍为 `None`。

## Alternatives considered

- **保留整数四舍五入**：99.6% 显示 100%，等于把「部分命中」谎报成「全命中」；本次问题的主诉之一
  就是数字与口径不符，保留等于不修 → 落选。
- **固定显示一位小数**（99.6%）：普通值会变成 43.0%，且 99.96 仍会显示 100.0，问题只被推后一位 → 落选。
- **显示「>99%」/「≈100%」**：信息量低于可判定精度，且与 DSH 的展示规则不兼容，跨端读数难对齐 → 落选。
- **本轮顺带补 DSH 的 `projectedTokens` 上下文预估**：需要 request-surface 增量估算器与
  compaction 有符号增量账本（DSH 由 `surface-fold.ts` + `token-meter` projection 承担），
  属于新能力而非口径修正，塞进本条目会把一个 bug-fix 变成未验收的架构变更 → 单独立项（见 Consequences）。

## Consequences

- 行为可见：接近全命中时右侧读数从「100%」变为「99.95%」这类更细分值；`上下文` 同样不再虚报到 100%。
- 新增 4 组机械可验用例：Rust `format_ratio_percent` 单测 + TUI 集成断言（19990/20000 不得出现
  「缓存 100%」）、前端 `tests/metrics-percent-honesty.spec.ts`（H1–H7，0.996 ⇒ 99.6%）。
  前端用例独立成文件，不改冻结的 `tests/metrics-snapshot-live.spec.ts`（Test Agent 独占写域）。
- 遗留（另立卡片，不属本条）：上下文占用的「下一跳预估」口径；DeepSeek `prompt_cache_miss_tokens`
  作为一等字段进入 `ProviderCallCacheRecord`（当前只做分母回退）。
- 桌面端 `contextPercent` 仍为整数字段（类型冻结），本轮不动；其消费点为侧栏刻度，非本诉点。
