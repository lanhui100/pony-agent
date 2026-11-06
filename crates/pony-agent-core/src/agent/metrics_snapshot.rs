//! Per-call 指标结算快照（P0 单一真值）。语义见 .dev-team/contract-matrix-metrics-snapshot.md
//!
//! 契约要点（实现方在 task-3 落逻辑时必须逐条满足，见契约矩阵 I1–I7）：
//! - `context_input_tokens` 为**协议归一**口径：`protocol` 以 `"anthropic"` 开头 ⇒
//!   `input + cache_read + cache_creation`；其余协议 ⇒ `input`；缺失时回退裸
//!   `input_tokens`，**禁止跨调用求和**（I4）。
//! - `cache_hit_ratio` / `context_ratio` 为 **0.0..=1.0 单位分数**（非百分数；I2/I3）。
//! - `call_speed_tokens_per_second` = `output_tokens / (duration_ms / 1000)`；
//!   `turn_speed_tokens_per_second` 必须与 `turn_flow::weighted_turn_speed(records).speed`
//!   同源（I5）。
use crate::agent::telemetry::ProviderCallCacheRecord;
use crate::agent::turn_flow::ProviderEventMeta;
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

pub fn build_turn_metrics_snapshot(
    _records: &[ProviderCallCacheRecord],
    _context_window_tokens: Option<u64>,
    _provider_meta: Option<&ProviderEventMeta>,
) -> Option<TurnMetricsSnapshot> {
    unimplemented!("build_turn_metrics_snapshot: 由 Executor 在 task-3 实现；语义见契约矩阵 I1–I7")
}
