//! Per-call 指标结算快照（P0 单一真值）。语义见 .dev-team/contract-matrix-metrics-snapshot.md
//!
//! 契约要点（实现方在 task-3 落逻辑时必须逐条满足，见契约矩阵 I1–I7）：
//! - `context_input_tokens` 为**协议归一**口径：由上游生产侧
//!   （`runtime/stream_support.rs::context_input_tokens_for`）写入
//!   [`ProviderCallCacheRecord::context_input_tokens`]，本模块**直接透传**该字段
//!   （裁决 A1：`ProviderCallCacheRecord` 无 `cache_creation` 字段，重算必然少算）；
//!   缺失时回退裸 `input_tokens`，**禁止跨调用求和**（I4）。
//! - `cache_hit_ratio` / `context_ratio` 为 **0.0..=1.0 单位分数**（非百分数；I2/I3）；
//!   越界（> 1.0，分子分母不同量纲或上游数据自相矛盾）⇒ `None` fail-closed，
//!   **禁止 clamp 成 100%**（I6 精神：clamp 只是把错误藏起来）。
//! - `call_speed_tokens_per_second` = `output_tokens / (gen / 1000)`，其中
//!   `gen = turn_flow::call_generation_ms(duration_ms, first_token_latency_ms)`（已扣 TTFT，
//!   裁决 A2）；`turn_speed_tokens_per_second` 必须与
//!   `turn_flow::weighted_turn_speed(records).speed` 同源（同一函数产出，I5）。
use crate::agent::cost::{estimate_cost_with_protocol, ProviderCostProtocol};
use crate::agent::telemetry::ProviderCallCacheRecord;
use crate::agent::turn_flow::{call_generation_ms, weighted_turn_speed, ProviderEventMeta};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnMetricsSnapshot {
    pub step: u32,
    pub context_input_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub cache_miss_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub call_speed_tokens_per_second: Option<f64>,
    pub turn_speed_tokens_per_second: Option<f64>,
    pub turn_output_tokens: u64,
    pub turn_generation_ms: u64,
    pub turn_calls: u32,
    pub cache_hit_ratio: Option<f64>,
    pub context_window_tokens: Option<u64>,
    pub context_ratio: Option<f64>,
    pub origin: Option<String>,
    pub provider: String,
    pub model: String,
    pub cost_usd: Option<f64>,
}

/// 浮点卫生（裁决 A5）：`None` 或非有限值 ⇒ `None`，绝不让 NaN/Inf 上线缆。
fn sanitize_f64(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

/// 单位分数（0.0..=1.0），**fail-closed**：不可信输入一律 `None`，绝不 clamp 掩盖。
///
/// - `denominator == 0` ⇒ `None`（比值无定义，不得伪装成 0/1）；
/// - `numerator > denominator`（即比值 `> 1.0`）⇒ `None`：这表示分子分母**不同量纲**
///   （典型：轮/会话累计命中 ÷ 末次单次 input）或上游数据自相矛盾（代理类网关可能上报
///   `cache_hit > input`）。契约要求「未知/不可信 ⇒ 不展示」（UI 显示无值），
///   **禁止 clamp 成 100%**——clamp 只是把「算法/数据错了」藏起来（I6 精神）。
fn unit_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    let candidate = (numerator as f64) / (denominator as f64);
    if !candidate.is_finite() || candidate > 1.0 {
        return None;
    }
    Some(candidate.max(0.0))
}

/// 「可计数」调用判据（与 [`crate::agent::turn_flow::weighted_turn_speed`] 的有效调用一致）：
/// `dur > 0 && out > 0`。
fn is_countable(record: &ProviderCallCacheRecord) -> bool {
    record.turn_duration_ms.unwrap_or(0) > 0 && record.output_tokens.unwrap_or(0) > 0
}

/// 末条「可计数」记录（`dur > 0 && out > 0`，与 `weighted_turn_speed` 有效调用判据一致）；
/// 无可计数记录时回退末条记录（含全 `None` 伪记录：由字段层 `Option` 自然表达未知）。
fn last_record(records: &[ProviderCallCacheRecord]) -> Option<&ProviderCallCacheRecord> {
    records
        .iter()
        .rev()
        .find(|record| is_countable(record))
        .or_else(|| records.last())
}

/// **该轮**成本（契约 §4 `costUsd` 语义）= 对 `records` 中**可计数**记录逐条计价后求和。
///
/// 口径（与同帧顶层轮聚合 `cost_usd`、落库侧保持一致：都是轮级 Σ，而非「末次调用成本」）：
/// - 逐条 `estimate_cost_with_protocol(model, rec.input, rec.output, rec.cache_hit, protocol)`；
/// - 跳过**不可计价**条目（未知模型 / 缺 usage 字段）：不为未知项补 0（禁 0 伪造）；
/// - 全部条目都不可计价 ⇒ `None`（「未知」不得伪装成 `$0`）；
/// - 求和结果走 A5 浮点卫生（非有限 ⇒ `None`）。
///
/// ⚠️ 取舍登记：**部分**条目不可计价时，本函数只累加可计价部分（成本可能低估）。
/// 理由：宁可少报也不伪造单价；逐调用明细成本由 `llm.chat` span 侧独立记录，可回溯核对。
fn resolve_cost_usd(
    records: &[ProviderCallCacheRecord],
    provider_meta: Option<&ProviderEventMeta>,
) -> Option<f64> {
    let meta = provider_meta?;
    let protocol = ProviderCostProtocol::from_label(&meta.protocol);
    let mut total_usd: f64 = 0.0;
    let mut priced_calls: u32 = 0;
    for record in records.iter().filter(|record| is_countable(record)) {
        let (Some(input_tokens), Some(output_tokens)) =
            (record.input_tokens, record.output_tokens)
        else {
            continue;
        };
        let Some(estimate) = estimate_cost_with_protocol(
            &meta.model,
            input_tokens,
            output_tokens,
            record.cache_hit_input_tokens,
            protocol,
        ) else {
            continue;
        };
        if estimate.total_cost_usd.is_finite() {
            total_usd += estimate.total_cost_usd;
            priced_calls = priced_calls.saturating_add(1);
        }
    }
    if priced_calls == 0 {
        return None;
    }
    sanitize_f64(Some(total_usd))
}

pub fn build_turn_metrics_snapshot(
    records: &[ProviderCallCacheRecord],
    context_window_tokens: Option<u64>,
    provider_meta: Option<&ProviderEventMeta>,
) -> Option<TurnMetricsSnapshot> {
    // I1：空记录无法产出任何 per-call 值（用 Default 冒充会让「未知」变成「0」真值）。
    let last = last_record(records)?;

    // A1/I4：协议归一已由生产侧写入 `context_input_tokens`，此处直接透传；
    // 缺失回退裸 input；**禁止重算、禁止跨记录求和**。
    let context_input_tokens = last.context_input_tokens.or(last.input_tokens);

    // A2：单次调用速度分母为 `gen`（已扣 TTFT），与加权轮速度同源；
    // `gen <= 0 || output <= 0` ⇒ 无定义（ADR 0035 有效调用判据），不得 Infinity。
    let call_generation = call_generation_ms(last.turn_duration_ms, last.first_token_latency_ms);
    let last_output = last.output_tokens.unwrap_or(0);
    let call_speed_tokens_per_second = if call_generation > 0 && last_output > 0 {
        sanitize_f64(Some(
            (last_output as f64) / ((call_generation as f64) / 1_000.0),
        ))
    } else {
        None
    };

    // A4/I5：轮级四项一律取自 `weighted_turn_speed`，禁止自算。
    let turn = weighted_turn_speed(records);

    // I2：分子分母必须同源（末次调用的 cache_hit 与 context_input），禁止混量纲。
    // 越界（cache_hit > context_input，上游数据自相矛盾）⇒ None，不得 clamp 成 100%。
    let cache_hit_ratio = match (last.cache_hit_input_tokens, context_input_tokens) {
        (Some(cache_hit), Some(context_input)) if context_input > 0 => {
            unit_ratio(cache_hit, context_input)
        }
        _ => None,
    };

    // I3：窗口未知 ⇒ None（不得回退 0/1，也不得用固定 200k 常量兜底）；
    // 上下文输入超过窗口（>1.0，上游口径异常）⇒ 同样 None，fail-closed。
    let context_ratio = match (context_input_tokens, context_window_tokens) {
        (Some(context_input), Some(window)) if window > 0 => unit_ratio(context_input, window),
        _ => None,
    };

    // 身份字段一律取 provider_meta（缺失 ⇒ 空串，不伪造）。
    let (provider, model) = match provider_meta {
        Some(meta) => (meta.provider_name.clone(), meta.model.clone()),
        None => (String::new(), String::new()),
    };

    Some(TurnMetricsSnapshot {
        // A3：快照对应 records 中最后一条（最新结算调用）的逻辑 hop 索引。
        step: records.len().saturating_sub(1) as u32,
        context_input_tokens,
        input_tokens: last.input_tokens,
        cache_hit_input_tokens: last.cache_hit_input_tokens,
        cache_miss_input_tokens: last.cache_miss_input_tokens,
        output_tokens: last.output_tokens,
        reasoning_tokens: last.reasoning_tokens,
        total_tokens: last.total_tokens,
        first_token_latency_ms: last.first_token_latency_ms,
        duration_ms: last.turn_duration_ms,
        call_speed_tokens_per_second,
        turn_speed_tokens_per_second: sanitize_f64(turn.speed),
        turn_output_tokens: turn.output_tokens,
        turn_generation_ms: turn.generation_ms,
        turn_calls: turn.calls,
        cache_hit_ratio,
        context_window_tokens,
        context_ratio,
        origin: last.usage_origin.clone(),
        provider,
        model,
        cost_usd: resolve_cost_usd(records, provider_meta),
    })
}
