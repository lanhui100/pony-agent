# Wave-Metrics Lead 裁决记录

> 本文件为 Lead 一票裁决，优先于空桩注释中的歧义表述；契约矩阵 I1–I7 不变量原文不变（只读）。

## A1（Test Agent 提出）— `contextInputTokens` 取值来源

**裁决：实现方必须直接取 `ProviderCallCacheRecord.context_input_tokens`，禁止在 builder 内按协议重算。**

理由：
- 协议归一已在生产侧完成（`agent/runtime/stream_support.rs::context_input_tokens_for`，anthropic = `input + cache_read + cache_creation`，其余 = `input`），并写入 `ProviderCallCacheRecord.context_input_tokens`。
- `ProviderCallCacheRecord` **没有** `cache_creation` 字段，builder 无法重算 anthropic 全量口径；重算只会得到 `input + cache_hit`，系统性少算 `cache_creation`。
- 回退顺序（冻结）：`last.context_input_tokens` → `last.input_tokens` → `None`。**任何跨 records 求和一律禁止**（I4）。

## A2（Test Agent 提出）— `call_speed_tokens_per_second` 分母

**裁决：`call_speed = output_tokens / (call_generation_ms(duration_ms, first_token_latency_ms) / 1000)`，即分母为 `gen`（已扣 TTFT），非 `duration_ms`。**

理由：
- 与 ADR 0035 S3、`turn_flow::weighted_turn_speed` 的 `gen = dur - ttft`（仅 `ttft` 存在且 `dur > ttft` 时抵扣；否则 `gen = dur`）完全同源，避免同一 UI 内出现两套速度。
- 空桩 `metrics_snapshot.rs` 顶部注释中「`call_speed = output / (duration / 1000)`」系表述疏漏，**由 task-3 一并更正为 `gen`**（属实现文件写域，不涉及冻结不变量）。
- `gen <= 0` 或 `output <= 0` 或结果非有限 ⇒ `None`（ADR 0035 有效调用判据）。

## A3 — `step` 取值

`step = records.len().saturating_sub(1) as u32`（调用方不额外传参；builder 内成型）。语义：该快照对应 records 中最后一条（最新结算调用）的逻辑 hop 索引。契约矩阵 §3 已授权交实施方决定。

## A4 — `turn_calls` / `turn_output_tokens` / `turn_generation_ms` 同源

三者一律取自 `turn_flow::weighted_turn_speed(records)`（`calls` / `output_tokens` / `generation_ms`），禁止自算（I5）。

## A5 — 浮点卫生

所有 `Option<f64>` 字段在写入前必须 `is_finite()`；非有限一律 `None`。`cache_hit_ratio` / `context_ratio` 写入前 `clamp(0.0, 1.0)`（这是**值域归一**，不是 I6 禁止的「用 clamp 掩盖渲染越界」）。

## A6 — 语义边界（事件附着）

`TurnStreamEvent.metrics` 仅在 `provider_call_records` 为 `Some` 且非空时写入；`is_delta_event`（`turn:delta`）路径一律 `None`。唯一附着点：`turn_flow::emit_stream_event_with_step` 内的 payload 构造前计算。

## A7（2026-11-06 增补，修正 A5）— 比值越界 fail-closed，取消 clamp

`unit_ratio` 对 `numerator > denominator` 一律返回 `None`（不展示），**禁止** `clamp(0.0, 1.0)`。
原因：clamp 会把「分子分母不同量纲 / 上游自相矛盾」静默变成 100%，与本波要消灭的「缓存命中恒 100%」同构。
A5 中「clamp 是值域归一」的表述**作废**，由本条取代。

## A8（增补）— `costUsd` 为轮级 Σ 逐调用成本

对本轮可计数调用逐条计价求和；不可计价条目跳过、全部不可计价 ⇒ `None`。
部分不可计价时总成本可能低估（已知取舍，逐调用真值由 `llm.chat` span 承担）。

## A9（增补）— 附着边界与残余窗口如实登记

`metrics` 仅在 `provider_call_records` 非空时附着；每轮**首个调用**完成前该轮无快照，此窗口回退 legacy 渲染。
该残余已登记契约矩阵 §7.1 R1，不属缺陷而是本波明确取舍。
