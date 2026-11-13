//! turn 级事件的构造收口（`turn:started` / `turn:hop_complete`）。
//!
//! 三处内联 `emit_stream_event(...)` 调用（25 个位置参数）收敛成带类型的窄接口，
//! 供 `turn_stream` / `turn_sync` 共用。行为与内联调用**逐字等价**：字段顺序、
//! `Some/None` 分布、求值顺序与副作用一致（唯一变化是 usage 拆分在收口函数内计算，
//! `token_usage_parts` 为纯函数）。

use super::*;

/// 流式入口的 `turn:started`（`start_turn_stream_with_control_and_facts` 的发射点）。
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_stream_turn_started<S: TurnEventSink>(
    sink: &S,
    turn_id: String,
    user_message: String,
    provider_meta: &ProviderEventMeta,
    build_context_observation: BuildContextObservation,
    start_trace_steps: Vec<TurnTraceStep>,
    session_id: Option<String>,
) {
    let trace_timeline = build_stream_started_trace_timeline(
        user_message.as_str(),
        provider_meta,
        &build_context_observation,
    );
    emit_stream_event(
        sink,
        "turn:started",
        turn_id,
        "started",
        Some("calling_model"),
        Some(user_message),
        None,
        Some(provider_meta),
        None,
        None,
        None,
        Some(build_context_observation),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(start_trace_steps),
        Some(trace_timeline),
        None,
        None,
        None,
        None,
        session_id,
    );
}

/// 同步入口的 `turn:started`（`run_turn_inner` 的发射点；无 trace timeline / trace steps）。
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_sync_turn_started(
    turn_id: String,
    user_message: String,
    provider_meta: &ProviderEventMeta,
    build_context_observation: BuildContextObservation,
    session_id: Option<String>,
) {
    let sync_sink = crate::agent::turn_flow::NoopTurnEventSink;
    emit_stream_event(
        &sync_sink,
        "turn:started",
        turn_id,
        "started",
        Some("calling_model"),
        Some(user_message),
        None,
        Some(provider_meta),
        None,
        None,
        None,
        Some(build_context_observation),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        session_id,
    );
}

/// 流式 hop 的 `turn:hop_complete`（`handle_stream_tool_turn` 的发射点）。
///
/// Wave-15 R1（契约 §7.10）：hop_complete 携带本次已完成调用的 per-call 记录
/// （有序前缀）。push / merge / emit 属同一迭代——刚 push 的记录就是本次已完成调用，
/// 末条即最新；既有累加字段语义不变。
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_stream_hop_complete<S: TurnEventSink>(
    sink: &S,
    turn_id: &str,
    provider_meta: &ProviderEventMeta,
    provider_source: &str,
    provider_mode: &str,
    token_usage: Option<&TokenUsage>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: u64,
    provider_call_records: Vec<ProviderCallCacheRecord>,
    session_id: Option<String>,
) {
    let (input_tokens, cache_hit_input_tokens, reasoning_tokens, output_tokens, total_tokens) =
        token_usage_parts(token_usage);
    emit_stream_event(
        sink,
        "turn:hop_complete",
        turn_id.to_string(),
        "hop_complete",
        Some("streaming_response"),
        None,
        None,
        Some(provider_meta),
        Some(provider_source.to_string()),
        Some(provider_mode.to_string()),
        None,
        None,
        input_tokens,
        cache_hit_input_tokens,
        reasoning_tokens,
        output_tokens,
        total_tokens,
        first_token_latency_ms,
        Some(turn_duration_ms),
        None,
        None,
        None,
        Some(provider_call_records),
        None,
        None,
        session_id,
    );
}
