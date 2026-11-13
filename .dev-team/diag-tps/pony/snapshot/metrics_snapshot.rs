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
//! - `call_speed_tokens_per_second` 一律取自 `turn_flow::call_speed(last)`，口径
//!   （`call_speed_basis`）同源（token 速度口径 v2 / I8）：可解码 ⇒ `(out-1)/(gen/1000)`，
//!   valid 但不可解码 ⇒ `out/(dur/1000)`，无效 ⇒ 两键皆无。
//! - `turn_speed_tokens_per_second` / `turn_output_tokens` / `turn_generation_ms` / `turn_calls`
//!   一律取自 `turn_flow::weighted_turn_speed(records)`（I4–I7 同源恒等式），
//!   `turn_speed_basis` 与速度同源（I10：缺席 ⇔ 无值）。
use crate::agent::cost::{estimate_cost_with_protocol, ProviderCostProtocol};
use crate::agent::telemetry::ProviderCallCacheRecord;
use crate::agent::turn_flow::{call_speed, weighted_turn_speed, ProviderEventMeta, TurnSpeedBasis};
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
    /// I8/I10：单调用口径（`stream_decode` | `end_to_end`）；缺席 ⇔ 无
    /// `callSpeedTokensPerSecond`（禁止自写第三种公式）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_speed_basis: Option<TurnSpeedBasis>,
    pub turn_speed_tokens_per_second: Option<f64>,
    /// I10：轮速度口径；缺席 ⇔ 无 `turnSpeedTokensPerSecond`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_speed_basis: Option<TurnSpeedBasis>,
    pub turn_output_tokens: u64,
    pub turn_generation_ms: u64,
    pub turn_calls: u32,
    /// 轮级 Σ 上下文输入（逐条取 `context_input_tokens`，缺失回退裸 `input_tokens` 后相加）。
    ///
    /// 与 `turn_output_tokens` 同属**轮级聚合**组：per-call 字段只描述末次调用，会话累计
    /// 必须从这里成型（TUI header 的 `↓out` 与会话缓存命中率分母都取本组，不得用末次调用冒充）。
    #[serde(default)]
    pub turn_context_input_tokens: u64,
    /// 轮级 Σ 缓存命中输入（只累加**上报了** `cache_hit_input_tokens` 的调用）。
    #[serde(default)]
    pub turn_cache_hit_input_tokens: u64,
    /// 轮级**上报了**缓存命中的调用数（会话命中率分母的可信度判据：「未知」不拉低命中率）。
    #[serde(default)]
    pub turn_calls_with_cache: u32,
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

/// 上下文占用比例（`0.0..=1.0`），供 UI 在**快照缺窗口**时用注册表真实窗口回退计算。
///
/// 与快照内 `context_ratio` 共用同一 [`unit_ratio`]（I3：越界/窗口未知 ⇒ `None`，禁止 clamp）。
/// 消费方只传入窗口值，比例仍在 core 侧算出 —— UI 不得自写第三种公式。
pub fn context_ratio_for_window(context_input_tokens: u64, window_tokens: u64) -> Option<f64> {
    if window_tokens == 0 {
        return None;
    }
    unit_ratio(context_input_tokens, window_tokens)
}

/// **诚实百分比文本**（不含 `%` 后缀）：`Σ分子 / Σ分母` 的展示值。
///
/// 口径契约（ADR `.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md`）：
/// - `denominator == 0` 或 `numerator > denominator` ⇒ `None`（与 [`unit_ratio`] 同一条
///   fail-closed：不可信数据不展示，**禁止 clamp**）；
/// - 真全量（`numerator == denominator`）⇒ `"100"`；
/// - 普通值 ⇒ 整数四舍五入（`3/5` ⇒ `"15"`，`2/3` ⇒ `"67"`）；
/// - 四舍五入会顶到 100 而分子小于分母 ⇒ 改用**两位小数向下取整**并去掉尾随零
///   （`19990/20000` ⇒ `"99.95"`，`9999/10000` ⇒ `"99.99"`），保证**永不高报**
///   ——「部分命中」不得被舍入成「全命中」（对齐 DSH `formatCacheHitPercent`）。
pub fn format_ratio_percent(numerator: u64, denominator: u64) -> Option<String> {
    if denominator == 0 || numerator > denominator {
        return None;
    }
    if numerator == denominator {
        return Some("100".to_string());
    }
    Some(percent_text(numerator as f64 / denominator as f64))
}

/// [`format_ratio_percent`] 的单位分数版（消费方已持有 `0.0..=1.0` 比值时用）。
///
/// 非有限 / `< 0` / `> 1` ⇒ `None`（fail-closed）；`1.0` ⇒ `"100"`；其余同上。
pub fn format_unit_fraction_percent(ratio: f64) -> Option<String> {
    if !ratio.is_finite() || ratio < 0.0 || ratio > 1.0 {
        return None;
    }
    if ratio == 1.0 {
        return Some("100".to_string());
    }
    Some(percent_text(ratio))
}

/// 单位分数 → 百分比文本（**只在 `ratio < 1` 时调用**，`100` 由调用方判定）。
fn percent_text(ratio: f64) -> String {
    let percent = ratio * 100.0;
    let rounded = percent.round();
    if rounded < 100.0 {
        return format!("{rounded:.0}");
    }
    // 部分命中：两位小数向下取整（floor），数学上保证 `<= 99.99`，绝不高报。
    let floored = (percent * 100.0).floor() / 100.0;
    let text = format!("{floored:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 「可计数」调用判据：与 [`crate::agent::turn_flow::call_valid`]（I1）同一判据 `dur > 0 && out > 0`。
fn is_countable(record: &ProviderCallCacheRecord) -> bool {
    record.turn_duration_ms.unwrap_or(0) > 0 && record.output_tokens.unwrap_or(0) > 0
}

/// 末条「可计数」记录（`dur > 0 && out > 0`，与 `turn_flow::call_valid` 同一判据）；
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

    // I8：单调用速度与口径**同源同帧**取自 `turn_flow::call_speed`（唯一出处），
    // 本模块禁止自算；无效调用（`dur <= 0 || out <= 0`）⇒ 两键皆无（I1/I6）。
    let (call_speed_value, call_speed_basis) = call_speed(last);
    let call_speed_tokens_per_second = sanitize_f64(call_speed_value);

    // A4/I4-I7：轮级四项 + 口径一律取自 `weighted_turn_speed`，禁止自算。
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

    // 轮级 Σ（会话累计的唯一原料，TUI 会话台账由此累加）：逐条 per-call 记录相加，
    // **禁止**用末次调用字段冒充会话累计。缺 `cache_hit_input_tokens` 的调用只计入
    // `turn_calls`，不计入 `turn_calls_with_cache`（「未知」不得被当成「0 命中」）。
    let mut turn_context_input_tokens: u64 = 0;
    let mut turn_cache_hit_input_tokens: u64 = 0;
    let mut turn_calls_with_cache: u32 = 0;
    for record in records {
        turn_context_input_tokens = turn_context_input_tokens.saturating_add(
            record
                .context_input_tokens
                .or(record.input_tokens)
                .unwrap_or(0),
        );
        if let Some(cache_hit) = record.cache_hit_input_tokens {
            turn_cache_hit_input_tokens = turn_cache_hit_input_tokens.saturating_add(cache_hit);
            turn_calls_with_cache = turn_calls_with_cache.saturating_add(1);
        }
    }

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
        call_speed_basis,
        turn_speed_tokens_per_second: sanitize_f64(turn.speed),
        turn_speed_basis: turn.basis,
        turn_output_tokens: turn.output_tokens,
        turn_generation_ms: turn.generation_ms,
        turn_calls: turn.calls,
        turn_context_input_tokens,
        turn_cache_hit_input_tokens,
        turn_calls_with_cache,
        cache_hit_ratio,
        context_window_tokens,
        context_ratio,
        origin: last.usage_origin.clone(),
        provider,
        model,
        cost_usd: resolve_cost_usd(records, provider_meta),
    })
}

#[cfg(test)]
mod percent_format_tests {
    use super::{format_ratio_percent, format_unit_fraction_percent};

    #[test]
    fn ordinary_ratios_keep_integer_rounding() {
        assert_eq!(format_ratio_percent(3, 5).as_deref(), Some("60"));
        assert_eq!(format_ratio_percent(3, 20).as_deref(), Some("15"));
        assert_eq!(format_ratio_percent(2, 3).as_deref(), Some("67"));
        assert_eq!(format_ratio_percent(0, 500).as_deref(), Some("0"));
        assert_eq!(format_unit_fraction_percent(0.6667).as_deref(), Some("67"));
    }

    #[test]
    fn full_hit_is_the_only_way_to_reach_100() {
        assert_eq!(format_ratio_percent(5, 5).as_deref(), Some("100"));
        assert_eq!(format_unit_fraction_percent(1.0).as_deref(), Some("100"));
    }

    #[test]
    fn partial_hit_never_rounds_up_to_full_hit() {
        // 0.998 ⇒ 整数四舍五入会到 100，必须降精度（向下取整）到 99.8。
        assert_eq!(format_ratio_percent(1_996, 2_000).as_deref(), Some("99.8"));
        assert_eq!(format_ratio_percent(9_999, 10_000).as_deref(), Some("99.99"));
        assert_eq!(format_unit_fraction_percent(0.996).as_deref(), Some("99.6"));
        assert_eq!(format_unit_fraction_percent(0.99999).as_deref(), Some("99.99"));
        assert_eq!(
            format_unit_fraction_percent(0.9999999).as_deref(),
            Some("99.99")
        );
    }

    #[test]
    fn untrusted_inputs_fail_closed_instead_of_clamping() {
        assert_eq!(format_ratio_percent(1, 0), None);
        assert_eq!(format_ratio_percent(6, 5), None);
        assert_eq!(format_unit_fraction_percent(f64::NAN), None);
        assert_eq!(format_unit_fraction_percent(f64::INFINITY), None);
        assert_eq!(format_unit_fraction_percent(-0.1), None);
        assert_eq!(format_unit_fraction_percent(1.2), None);
    }
}
