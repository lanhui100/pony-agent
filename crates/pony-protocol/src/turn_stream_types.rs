use crate::hooks::HookTraceRecord;
use crate::provider_types::BuildContextObservation;
use crate::session_types::TraceTimelineEntry;
use crate::telemetry::{ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep};
use crate::turn_event::TurnEvent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStreamEvent {
    pub event_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: String,
    /// Wave-4.1 B1 闭环：本事件的 **导出态** OTel trace_id（camelCase 下发 `traceId`）。
    ///
    /// 语义：仅当该 turn 真实创建并被采样了 root span 时才有值（`otel::exported_trace_id()`）；
    /// 门禁关闭 / 未 init / 未采样 ⇒ `None`（R1：绝不回落到本地相关性 id）。
    /// 前端 `runtime` store 用它回填 `activeTraceId`，使 live turn 期间的错误上报也带真实 trace_id。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Wave-5 K2：本轮成本（USD，camelCase 下发 `costUsd`）。
    ///
    /// 语义：仅当本轮拿到真实 usage 且模型命中内置价格表时有值；
    /// 未知模型/缺 usage ⇒ `None`，键缺席（前端显示「—」，严禁用 0.0 伪造）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub kind: String,
    pub event_type: Option<String>,
    pub event_version: Option<String>,
    pub sequence: Option<u64>,
    pub emitted_at_ms: Option<u64>,
    pub phase: Option<String>,
    pub text: Option<String>,
    pub reasoning_content: Option<String>,
    pub error: Option<String>,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub trace_steps: Option<Vec<TurnTraceStep>>,
    pub trace_timeline: Option<Vec<TraceTimelineEntry>>,
    pub tool_activities: Option<Vec<TurnToolActivity>>,
    pub provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    pub hook_trace_records: Option<Vec<HookTraceRecord>>,
    pub session_summary: Option<String>,
    /// PA-095 #4：逻辑 hop 索引（0-based；None = 缺省归 step 0，wire 兼容）。
    /// delta/chunk 携带所属 hop；turn:trace(calling_model) 携带即将发起的
    /// followup hop 索引（≥1，初始 call 的 step/start 由 turn:started 承担）。
    #[serde(default)]
    pub step: Option<u32>,
    /// wave-60 §1/§C3：结构化停止原因（wire `stopReason`，取值见
    /// `crate::agent::stop_escalation::StopReason`）。为**追加**字段：
    /// `#[serde(default)]` + TS 可选，旧消费者不读即行为不变；
    /// `turn:cancelled` 携带该轮原因（无记录 ⇒ `"legacy"`），`turn:completed`/`turn:failed` ⇒ `None`。
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// P0：per-call 指标结算快照（单一真值）。仅当事件携带 provider_call_records 时存在。
    /// 旧消费者不读即行为不变（wire additive）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<crate::metrics_snapshot::TurnMetricsSnapshot>,
    /// PA-171 C4：失败归类（wire `errorClass`；取值 `timeout|rate_limited|quota|network|auth|
    /// bad_request|other`，唯一来源 [`crate::agent::retry::classify_error_class`]）。
    ///
    /// 语义：`turn:failed` 恒有值（审计机唯一判据，禁止第二套关键词表）；非失败事件 `None`
    /// ⇒ 键缺席（additive，旧消费者不读即行为不变）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
}


/// 取消回合哨兵句：字符串真源在 `session::CANCELLED_TURN_SENTINEL`（这里只是本模块别名，
/// 避免同一条线协议出现第二个字面量）。

pub trait TurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent);
    fn persist(&self, _event: TurnEvent) {}
}

pub struct NoopTurnEventSink;

impl TurnEventSink for NoopTurnEventSink {
    fn emit(&self, _name: &str, _payload: TurnStreamEvent) {}
}

pub type EventPersistFn = dyn Fn(&str, &str, TurnEvent, bool) + Send + Sync;

pub use crate::turn_flow_registry::*;
