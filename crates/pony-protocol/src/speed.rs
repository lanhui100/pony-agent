use crate::telemetry::ProviderCallCacheRecord;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct ProviderEventMeta {
    pub requested_name: String,
    pub provider_name: String,
    pub protocol: String,
    pub model: String,
    /// P0：该 provider 的上下文窗口容量（tokens）。**additive** 字段：
    /// `None` ⇒ 未知窗口，消费者须回退既有路径（契约矩阵 I3）。
    /// 由 Executor 在 task-3 于 [`provider_event_meta`] 填真实取值。
    pub context_window_tokens: Option<u64>,
}


/// 解码窗口物理下限：绝对下限（毫秒）。
///
/// 突发包（SSE 被上游代理攒成一包）与计时抖动下的 `gen` 只有 1–2ms，不能代表生成速率；
/// 该下限是物理事实约束，不是数值美化（契约 I3）。
pub const DECODE_FLOOR_MIN_MS: u64 = 200;

/// 解码窗口物理下限：相对下限（百分比）。`floor(dur) = max(200ms, 5% * dur)`。
pub const DECODE_FLOOR_RATIO_PCT: u64 = 5;

/// turn / 单调用速度的**口径**（契约 I10 线缆自描述）。
///
/// `StreamDecode` = 分母只含真实解码窗口（已扣 TTFT 且过物理下限）；
/// `EndToEnd` = 端到端吞吐（含排队 + prefill），与解码速度**不可比**，消费面须加 `≈` 前缀。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSpeedBasis {
    /// I4：`Σ(out-1) / (Σgen/1000)`。
    StreamDecode,
    /// I5：`Σout / (Σdur/1000)`。
    EndToEnd,
}

impl TurnSpeedBasis {
    /// wire 取值：`stream_decode` | `end_to_end`。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StreamDecode => "stream_decode",
            Self::EndToEnd => "end_to_end",
        }
    }
}

/// I3：解码窗口物理下限 `floor(dur) = max(200ms, 5% * dur)`。
pub fn decode_floor_ms(duration_ms: u64) -> u64 {
    (duration_ms * DECODE_FLOOR_RATIO_PCT / 100).max(DECODE_FLOOR_MIN_MS)
}

/// I1：per-call 有效性 `dur > 0 && out > 0`。
///
/// 缺失或为 0 的记录不参与任何速度计算，也不计入 calls（禁止把「未知」当 0 计入）。
pub fn call_valid(duration_ms: Option<u64>, output_tokens: Option<u64>) -> bool {
    duration_ms.unwrap_or(0) > 0 && output_tokens.unwrap_or(0) > 0
}

/// I3：解码样本判据（**唯一出处**，消费面禁止各写一份）。
///
/// `call_valid && ttft 存在 && dur > ttft && out >= 2 && gen >= floor(dur)`：
/// 无 TTFT 观测（批量同步 followup / 非流式决策）、单 token 调用、突发包一律不合格，
/// 只能进 I5 的端到端回退。
pub fn decode_eligible(
    duration_ms: Option<u64>,
    ttft_ms: Option<u64>,
    output_tokens: Option<u64>,
) -> bool {
    if !call_valid(duration_ms, output_tokens) {
        return false;
    }
    let duration = duration_ms.unwrap_or(0);
    let output = output_tokens.unwrap_or(0);
    let Some(ttft) = ttft_ms else {
        return false;
    };
    if duration <= ttft || output < 2 {
        return false;
    }
    call_generation_ms(Some(duration), Some(ttft)) >= decode_floor_ms(duration)
}

/// 比值 → 速度（A5 浮点卫生）：分子/分母为 0、非有限、非正 ⇒ `None`（禁止 0/NaN 上线缆）。
pub fn speed_from_ratio(numerator: u64, denominator_ms: u64) -> Option<f64> {
    if numerator == 0 || denominator_ms == 0 {
        return None;
    }
    let candidate = (numerator as f64) / ((denominator_ms as f64) / 1_000.0);
    (candidate.is_finite() && candidate > 0.0).then_some(candidate)
}

/// 单次调用生成窗口（**分母候选，不等于有效生成窗口**）。
///
/// `gen = (ttft 存在且 dur > ttft) ? dur - ttft : dur`（`ttft > dur` 退化，不出负值）。
///
/// ⚠️ 本函数只按 I2 定义**窗口**，不定义「合格性」：能否作为解码样本必须走
/// [`decode_eligible`]（I3）。历史上把 `gen` 直接当分母用过，导致无 TTFT 调用与突发包
/// 污染速度（契约 §0 根因 1/2），不要重犯。
pub fn call_generation_ms(duration_ms: Option<u64>, ttft_ms: Option<u64>) -> u64 {
    let duration = duration_ms.unwrap_or(0);
    match ttft_ms {
        Some(ttft) if duration > ttft => duration - ttft,
        _ => duration,
    }
}

/// I4 累加器：`decode_eligible` 侧的 `Σout` / `Σgen` / 调用数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodeTotals {
    pub output_tokens: u64,
    pub generation_ms: u64,
    pub calls: u32,
}

/// I5 累加器：`valid` 侧的 `Σout` / `Σdur` / 调用数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ValidTotals {
    pub output_tokens: u64,
    pub duration_ms: u64,
    pub calls: u32,
}

/// 速度真值（I4–I7 同源恒等式 + I10 自描述；UI / 快照 / 根 span 共用）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnSpeedTruth {
    /// 参与计算集合的 `Σout / (分母/1000)`；无参与计算调用 ⇒ `None`（禁止 0 占位）。
    pub speed: Option<f64>,
    /// 口径（I10）：`stream_decode` | `end_to_end`；`None` ⇔ 无值。
    pub basis: Option<TurnSpeedBasis>,
    /// 参与计算集合的 Σout（`stream_decode` ⇒ Σeligible out；`end_to_end` ⇒ Σvalid out），I7。
    pub output_tokens: u64,
    /// 分母时长（`stream_decode` ⇒ Σgen；`end_to_end` ⇒ Σdur），毫秒，I7。
    pub generation_ms: u64,
    /// 首个带 ttft 的 valid per-call 值（**不**跨 hop 采样，不是 turn elapsed 内首包）。
    pub ttft_ms: Option<u64>,
    /// 参与计算集合的调用数（按 basis 口径，I7）。
    pub calls: u32,
}

/// I4/I5/I6：由两路累加器择一结算速度（**唯一公式出口**，禁止复制粘贴后独立演化）。
///
/// - `decode.calls > 0` ⇒ `Σ(out-1) / (Σgen/1000)`，`basis = stream_decode`（I4）；
/// - 否则 `valid.calls > 0` ⇒ `Σout / (Σdur/1000)`，`basis = end_to_end`（I5）；
/// - 否则全 `None` / `0`（I6：既无 eligible 也无 valid ⇒ 无值，禁止伪造）。
///
/// 禁止把两路口径混进同一个 Σ。`otel::read_active_turn_call_truth` 与
/// `metrics_snapshot::build_turn_metrics_snapshot` 都经此函数结算，保证同源（I9）。
pub fn resolve_turn_speed(
    decode: DecodeTotals,
    valid: ValidTotals,
    ttft_ms: Option<u64>,
) -> TurnSpeedTruth {
    if decode.calls > 0 {
        return TurnSpeedTruth {
            // 分子扣 1/token：首 token 已计入 TTFT，解码窗口只新增 `out - 1` 个 token。
            speed: speed_from_ratio(
                decode.output_tokens.saturating_sub(u64::from(decode.calls)),
                decode.generation_ms,
            ),
            basis: Some(TurnSpeedBasis::StreamDecode),
            output_tokens: decode.output_tokens,
            generation_ms: decode.generation_ms,
            ttft_ms,
            calls: decode.calls,
        };
    }
    if valid.calls > 0 {
        return TurnSpeedTruth {
            speed: speed_from_ratio(valid.output_tokens, valid.duration_ms),
            basis: Some(TurnSpeedBasis::EndToEnd),
            output_tokens: valid.output_tokens,
            generation_ms: valid.duration_ms,
            ttft_ms,
            calls: valid.calls,
        };
    }
    TurnSpeedTruth::default()
}

/// 由原始观测求单调用速度与口径（I8，**唯一出处**）。
///
/// eligible ⇒ `(out-1) / (gen/1000)`，`stream_decode`；valid 但不可解码 ⇒
/// `out / (dur/1000)`，`end_to_end`；无效 ⇒ `(None, None)`。
pub fn call_speed_from_raw(
    duration_ms: Option<u64>,
    ttft_ms: Option<u64>,
    output_tokens: Option<u64>,
) -> (Option<f64>, Option<TurnSpeedBasis>) {
    if !call_valid(duration_ms, output_tokens) {
        return (None, None);
    }
    let duration = duration_ms.unwrap_or(0);
    let output = output_tokens.unwrap_or(0);
    if decode_eligible(Some(duration), ttft_ms, Some(output)) {
        return (
            speed_from_ratio(
                output.saturating_sub(1),
                call_generation_ms(Some(duration), ttft_ms),
            ),
            Some(TurnSpeedBasis::StreamDecode),
        );
    }
    (
        speed_from_ratio(output, duration),
        Some(TurnSpeedBasis::EndToEnd),
    )
}

/// 单条 per-call 记录的速度与口径（I8；快照 `callSpeed*` 用）。
pub fn call_speed(record: &ProviderCallCacheRecord) -> (Option<f64>, Option<TurnSpeedBasis>) {
    call_speed_from_raw(
        record.turn_duration_ms,
        record.first_token_latency_ms,
        record.output_tokens,
    )
}

/// per-hop `llm.chat` span 的单调用速度（I8/I9：与快照 per-call 同一公式，不另写一份）。
pub fn calculate_call_generation_speed(
    output_tokens: Option<u64>,
    elapsed_ms: u64,
    ttft_ms: Option<u64>,
) -> Option<f64> {
    call_speed_from_raw(Some(elapsed_ms), ttft_ms, output_tokens).0
}

/// I4/I5/I6：由 per-call 记录求 turn 速度真值。
///
/// 先一次遍历把记录分成 `decode_eligible` 与 `valid` 两路累加，再由
/// [`resolve_turn_speed`] 择一结算；**禁止**在一次 Σ 中混两种口径。
pub fn weighted_turn_speed(records: &[ProviderCallCacheRecord]) -> TurnSpeedTruth {
    let mut decode = DecodeTotals::default();
    let mut valid = ValidTotals::default();
    let mut ttft_ms: Option<u64> = None;
    for record in records {
        if !call_valid(record.turn_duration_ms, record.output_tokens) {
            // I1：planner-only 伪记录（dur/out 为 None）与 0 值记录不进任何 Σ、不计 calls。
            continue;
        }
        let duration = record.turn_duration_ms.unwrap_or(0);
        let output = record.output_tokens.unwrap_or(0);
        valid.output_tokens = valid.output_tokens.saturating_add(output);
        valid.duration_ms = valid.duration_ms.saturating_add(duration);
        valid.calls = valid.calls.saturating_add(1);
        if ttft_ms.is_none() {
            ttft_ms = record.first_token_latency_ms;
        }
        if decode_eligible(Some(duration), record.first_token_latency_ms, Some(output)) {
            decode.output_tokens = decode.output_tokens.saturating_add(output);
            decode.generation_ms = decode.generation_ms.saturating_add(call_generation_ms(
                Some(duration),
                record.first_token_latency_ms,
            ));
            decode.calls = decode.calls.saturating_add(1);
        }
    }
    resolve_turn_speed(decode, valid, ttft_ms)
}

/// F1 取证口径：一轮 provider 调用的**样本普查**（唯一出处；事件 / 日志 / 显示端共用）。
///
/// 定义：
/// - `total`：该轮 per-call 记录总数（含无效/不可解码记录，回答「这轮到底调了几次」）；
/// - `valid`：`call_valid` 记录数（`dur > 0 && out > 0`）；
/// - `eligible`：`decode_eligible` 记录数（流式解码样本，进入 `stream_decode` 的分子/分母）；
/// - `burst_excluded`：**突发整包剔除数**——已具备解码前提（ttft 存在、`dur > ttft`、`out >= 2`）
///   但解码窗口 `gen` 低于物理下限 [`decode_floor_ms`]（SSE 被上游代理攒成一包）的调用数；
/// - `no_ttft_excluded`：`valid` 但**无 TTFT 观测**（无法定解码窗口，只能进端到端回退）的调用数；
/// - `other_excluded`：其余不合格（`dur <= ttft` 或 `out < 2`）。
///
/// 恒等式：`valid == eligible + burst_excluded + no_ttft_excluded + other_excluded`。
/// [`Self::excluded_total`]（= `valid − eligible`）是上述三类剔除之和，对外键名
/// `turnExcludedTotal`，不丢任何真值。
///
/// 线缆口径（契约 v3，Lead 裁决 2026-11-12；两键定义各一行，禁止读者从名字猜）：
/// - `turnBurstExcluded` = `burst_excluded`（**真突发整包数**：有 ttft 且 `gen < floor(dur)`）；
/// - `turnExcludedTotal` = `excluded_total()`（**全部解码剔除**：`valid − decode_eligible`）；
/// - `turnCallsTtftMissing` = `no_ttft_excluded`（`valid − 有 ttft 的 valid`，无 TTFT 不可测）。
///
/// 判据只复用 [`call_valid`] / [`decode_eligible`] / [`decode_floor_ms`]（I3：判据只在 turn_flow
/// 定义一次，本函数不复制判据细节）；速度本身不在此计算（仍只由 [`weighted_turn_speed`] 结算）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnCallCensus {
    pub total: u32,
    pub valid: u32,
    pub eligible: u32,
    pub burst_excluded: u32,
    pub no_ttft_excluded: u32,
    pub other_excluded: u32,
}

impl TurnCallCensus {
    /// `valid 记录数 − decode_eligible 记录数`（契约 I1 原文口径 = 全部解码剔除之和）。
    pub fn excluded_total(&self) -> u32 {
        self.burst_excluded
            .saturating_add(self.no_ttft_excluded)
            .saturating_add(self.other_excluded)
    }
}

/// 由 per-call 记录派生样本普查（[`TurnCallCensus`]）。
pub fn turn_call_census(records: &[ProviderCallCacheRecord]) -> TurnCallCensus {
    let mut census = TurnCallCensus {
        total: u32::try_from(records.len()).unwrap_or(u32::MAX),
        ..TurnCallCensus::default()
    };
    for record in records {
        if !call_valid(record.turn_duration_ms, record.output_tokens) {
            continue;
        }
        census.valid = census.valid.saturating_add(1);
        if decode_eligible(
            record.turn_duration_ms,
            record.first_token_latency_ms,
            record.output_tokens,
        ) {
            census.eligible = census.eligible.saturating_add(1);
            continue;
        }
        // 不合格原因分解（判据顺序与 `decode_eligible` 内部一致：先 ttft，再 dur/out，最后物理下限）。
        match record.first_token_latency_ms {
            None => census.no_ttft_excluded = census.no_ttft_excluded.saturating_add(1),
            Some(ttft) => {
                let duration = record.turn_duration_ms.unwrap_or(0);
                let output = record.output_tokens.unwrap_or(0);
                if duration <= ttft || output < 2 {
                    census.other_excluded = census.other_excluded.saturating_add(1);
                } else {
                    // 解码前提全部满足，仍不合格 ⇒ 唯一可能是 `gen < floor(dur)`：突发整包。
                    census.burst_excluded = census.burst_excluded.saturating_add(1);
                }
            }
        }
    }
    census
}

/// F1 I2：轮速度结算结构化日志的 message（单行 JSON 的 `msg` 字段）。
///
/// 日志键名与事件字段**逐字同名**（camelCase，契约冻结键名），便于离线取证时
/// 「日志 ↔ `turn_events.payload`」两侧共用同一套解析。
pub const TURN_SPEED_LOG_MESSAGE: &str = "turn speed settled";
