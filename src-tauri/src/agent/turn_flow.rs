use crate::agent::capability_bridge::SkillDescriptor;
use crate::agent::hooks::HookTraceRecord;
use crate::agent::provider::{
    BuildContextObservation, ProviderClient, ProviderDecision, ProviderManager, ProviderRequest,
    ProviderStreamChunk, TokenUsage,
};
use crate::agent::session::TraceTimelineEntry;
use crate::agent::telemetry::{ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep};
use crate::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::context::RetrievedContextState;
use super::runtime::{TurnResult, TurnStreamEvent};

pub trait TurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent);
}

pub struct PreparedTurn {
    pub user_message: String,
    pub display_message: String,
    pub retrieved: RetrievedContextState,
    pub provider: ProviderManager,
    pub tools: Vec<ToolDefinition>,
    pub planner_skills: Vec<SkillDescriptor>,
    pub planning_request: ProviderRequest,
    pub build_context_observation: BuildContextObservation,
}

pub struct PlannedTurn {
    pub first_decision: ProviderDecision,
    pub resolved_tool_call: Option<ToolCall>,
    pub initial_decision_duration_ms: Option<u64>,
    pub planner_hook_trace_records: Vec<HookTraceRecord>,
}

pub struct PersistedTurnOutcome {
    pub session_summary: String,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Clone)]
pub struct TurnEventEnvelope {
    pub event_id: String,
    pub event_type: String,
    pub event_version: String,
    pub sequence: u64,
    pub emitted_at_ms: u64,
}

pub struct SyncToolTurnOutcome {
    pub assistant_message: String,
    pub provider_native_transcript: Option<Vec<Value>>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub hook_trace_records: Vec<HookTraceRecord>,
    pub first_token_latency_ms: Option<u64>,
}

#[derive(Clone)]
pub struct ProviderEventMeta {
    pub requested_name: String,
    pub provider_name: String,
    pub protocol: String,
    pub model: String,
}

pub fn build_failed_turn_result(
    provider_meta: Option<&ProviderEventMeta>,
    user_message: String,
    assistant_message: String,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Vec<TurnToolActivity>,
) -> TurnResult {
    build_failed_turn_result_with_hooks(
        provider_meta,
        user_message,
        assistant_message,
        trace_steps,
        tool_activities,
        Vec::new(),
    )
}

pub fn build_failed_turn_result_with_hooks(
    provider_meta: Option<&ProviderEventMeta>,
    user_message: String,
    assistant_message: String,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Vec<TurnToolActivity>,
    hook_trace_records: Vec<HookTraceRecord>,
) -> TurnResult {
    TurnResult {
        event_id: None,
        event_type: None,
        event_version: None,
        sequence: None,
        emitted_at_ms: None,
        phase: "failed".to_string(),
        provider_requested_name: provider_meta
            .map(|meta| meta.requested_name.clone())
            .unwrap_or_default(),
        provider_name: provider_meta
            .map(|meta| meta.provider_name.clone())
            .unwrap_or_default(),
        provider_protocol: provider_meta
            .map(|meta| meta.protocol.clone())
            .unwrap_or_default(),
        provider_model: provider_meta
            .map(|meta| meta.model.clone())
            .unwrap_or_default(),
        provider_source: "failed".to_string(),
        provider_mode: "failed".to_string(),
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
        user_message,
        assistant_message: assistant_message.clone(),
        trace_steps,
        trace_timeline: Vec::new(),
        tool_activities,
        provider_call_records: Vec::new(),
        hook_trace_records,
        session_summary: assistant_message,
    }
}

pub fn provider_event_meta(provider: &ProviderManager) -> ProviderEventMeta {
    ProviderEventMeta {
        requested_name: provider.requested_name().to_string(),
        provider_name: provider.name().to_string(),
        protocol: provider.protocol_label().to_string(),
        model: provider.model().to_string(),
    }
}

pub fn emit_stream_failed(
    sink: &impl TurnEventSink,
    turn_id: String,
    provider_meta: Option<&ProviderEventMeta>,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    build_context_observation: Option<BuildContextObservation>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    hook_trace_records: Option<Vec<HookTraceRecord>>,
    error: String,
    session_id: Option<String>,
) {
    emit_event(
        sink,
        "turn:failed",
        TurnStreamEvent {
            event_id: None,
            session_id,
            turn_id,
            kind: "failed".to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: Some("failed".to_string()),
            text: Some("This turn failed.".to_string()),
            reasoning_content: None,
            error: Some(error),
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms,
            turn_duration_ms,
            trace_steps: Some(trace_steps),
            trace_timeline,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            session_summary: None,
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub fn emit_stream_cancelled(
    sink: &impl TurnEventSink,
    turn_id: String,
    provider_meta: Option<&ProviderEventMeta>,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    build_context_observation: Option<BuildContextObservation>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    error: String,
    session_id: Option<String>,
) {
    emit_event(
        sink,
        "turn:cancelled",
        TurnStreamEvent {
            event_id: None,
            session_id,
            turn_id,
            kind: "cancelled".to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: Some("cancelled".to_string()),
            text: Some("This turn was cancelled.".to_string()),
            reasoning_content: None,
            error: Some(error),
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms,
            turn_duration_ms,
            trace_steps: Some(trace_steps),
            trace_timeline,
            tool_activities,
            provider_call_records,
            hook_trace_records: None,
            session_summary: None,
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub fn emit_stream_event(
    sink: &impl TurnEventSink,
    name: &str,
    turn_id: String,
    kind: &str,
    phase: Option<&str>,
    text: Option<String>,
    reasoning_content: Option<String>,
    provider_meta: Option<&ProviderEventMeta>,
    provider_source: Option<String>,
    provider_mode: Option<String>,
    fallback_reason: Option<String>,
    build_context_observation: Option<BuildContextObservation>,
    input_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    trace_steps: Option<Vec<TurnTraceStep>>,
    trace_timeline: Option<Vec<TraceTimelineEntry>>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    hook_trace_records: Option<Vec<HookTraceRecord>>,
    session_summary: Option<String>,
    session_id: Option<String>,
) {
    emit_event(
        sink,
        name,
        TurnStreamEvent {
            event_id: None,
            session_id,
            turn_id,
            kind: kind.to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: phase.map(|value| value.to_string()),
            text,
            reasoning_content,
            error: None,
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source,
            provider_mode,
            fallback_reason,
            build_context_observation,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            trace_steps,
            trace_timeline,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            session_summary,
        },
    );
}

fn resolve_canonical_event_type(
    name: &str,
    phase: Option<&str>,
    build_context_observation: Option<&BuildContextObservation>,
    tool_activities: Option<&[TurnToolActivity]>,
) -> String {
    match name {
        "turn:started" => "turn.created".to_string(),
        "turn:delta" => "turn.output_delta".to_string(),
        "turn:completed" => "turn.completed".to_string(),
        "turn:failed" => "turn.failed".to_string(),
        "turn:cancelled" => "turn.cancelled".to_string(),
        "turn:trace" => {
            if build_context_observation.is_some() && matches!(phase, Some("building_context")) {
                "turn.context_built".to_string()
            } else {
                match phase {
                    Some("calling_model") => "turn.model_call_started".to_string(),
                    _ => "turn.trace_updated".to_string(),
                }
            }
        }
        "turn:tool" => {
            if tool_activities
                .unwrap_or(&[])
                .iter()
                .any(|activity| activity.status == "running")
            {
                "turn.tool_call_started".to_string()
            } else {
                "turn.tool_call_completed".to_string()
            }
        }
        _ => name.replace(':', "."),
    }
}

fn turn_event_version() -> &'static str {
    "turn-event-v1"
}

fn now_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn next_turn_event_sequence(turn_id: &str) -> u64 {
    let registry = turn_event_sequence_registry();
    let mut state = registry.lock().expect("turn event sequence lock poisoned");
    let next = state.get(turn_id).copied().unwrap_or(0).saturating_add(1);
    state.insert(turn_id.to_string(), next);
    next
}

fn clear_turn_event_sequence(turn_id: &str) {
    let registry = turn_event_sequence_registry();
    let mut state = registry.lock().expect("turn event sequence lock poisoned");
    state.remove(turn_id);
}

fn turn_event_sequence_registry() -> &'static Mutex<HashMap<String, u64>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn build_terminal_turn_event_envelope(
    turn_id: &str,
    name: &str,
    phase: Option<&str>,
    build_context_observation: Option<&BuildContextObservation>,
    tool_activities: Option<&[TurnToolActivity]>,
) -> TurnEventEnvelope {
    let sequence = next_turn_event_sequence(turn_id);
    let envelope = TurnEventEnvelope {
        event_id: format!("{}:{}", turn_id, sequence),
        event_type: resolve_canonical_event_type(
            name,
            phase,
            build_context_observation,
            tool_activities,
        ),
        event_version: turn_event_version().to_string(),
        sequence,
        emitted_at_ms: now_timestamp_ms(),
    };

    if matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled") {
        clear_turn_event_sequence(turn_id);
    }

    envelope
}

pub fn emit_turn_failed(
    sink: &impl TurnEventSink,
    turn_id: String,
    provider_requested_name: Option<String>,
    provider_name: Option<String>,
    provider_protocol: Option<String>,
    provider_model: Option<String>,
    trace_steps: Vec<TurnTraceStep>,
    error: String,
    session_id: Option<String>,
) {
    let provider_meta = match (
        provider_requested_name,
        provider_name,
        provider_protocol,
        provider_model,
    ) {
        (Some(requested_name), Some(provider_name), Some(protocol), Some(model)) => {
            Some(ProviderEventMeta {
                requested_name,
                provider_name,
                protocol,
                model,
            })
        }
        _ => None,
    };
    emit_stream_failed(
        sink,
        turn_id,
        provider_meta.as_ref(),
        trace_steps,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        error,
        session_id,
    );
}

pub fn emit_event(sink: &impl TurnEventSink, name: &str, payload: TurnStreamEvent) {
    let mut payload = payload;
    let terminal_turn_id = payload.turn_id.clone();
    let next_sequence = next_turn_event_sequence(&payload.turn_id);
    payload.event_id = Some(format!("{}:{}", payload.turn_id, next_sequence));
    payload.event_type = Some(resolve_canonical_event_type(
        name,
        payload.phase.as_deref(),
        payload.build_context_observation.as_ref(),
        payload.tool_activities.as_deref(),
    ));
    payload.event_version = Some(turn_event_version().to_string());
    payload.sequence = Some(next_sequence);
    payload.emitted_at_ms = Some(now_timestamp_ms());

    eprintln!(
        "[pony-agent][runtime] emit {} event_type={} turn={} seq={:?} phase={:?} text_len={} tools={}",
        name,
        payload.event_type.as_deref().unwrap_or("unknown"),
        payload.turn_id,
        payload.sequence,
        payload.phase,
        payload.text.as_ref().map(|text| text.len()).unwrap_or(0),
        payload
            .tool_activities
            .as_ref()
            .map(|tools| tools.len())
            .unwrap_or(0)
    );
    sink.emit(name, payload);
    if matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled") {
        clear_turn_event_sequence(&terminal_turn_id);
    }
}

pub fn normalize_user_message(message: &str) -> String {
    let normalized = message.trim();
    if normalized.is_empty() {
        "请先输入问题。".to_string()
    } else {
        normalized.to_string()
    }
}

pub fn token_usage_parts(
    token_usage: Option<&TokenUsage>,
) -> (
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
) {
    match token_usage {
        Some(token_usage) => (
            token_usage.input_tokens,
            token_usage.cache_hit_input_tokens,
            token_usage.reasoning_tokens,
            token_usage.output_tokens,
            token_usage.total_tokens,
        ),
        None => (None, None, None, None, None),
    }
}

pub fn provider_failure_message(
    provider_mode: &str,
    fallback_reason: Option<&str>,
) -> Option<String> {
    if provider_mode == "mock" {
        return Some(
            fallback_reason
                .unwrap_or("模型调用失败，未返回真实结果。")
                .to_string(),
        );
    }

    None
}

fn stream_delta_chunks(
    sink: &impl TurnEventSink,
    turn_id: &str,
    phase: &str,
    text: Option<&str>,
    reasoning_content: Option<&str>,
    started_at: &std::time::Instant,
    initial_latency_ms: Option<u64>,
    measure_first_delta_latency: bool,
) -> Option<u64> {
    let source = text.or(reasoning_content).unwrap_or_default();
    let mut first_token_latency_ms = initial_latency_ms;
    let mut latency_emitted = false;
    let chunks = source
        .as_bytes()
        .chunks(48)
        .map(|chunk| String::from_utf8_lossy(chunk).to_string())
        .collect::<Vec<_>>();

    for (index, delta) in chunks.iter().enumerate() {
        let latency = if !latency_emitted && index == 0 {
            if let Some(value) = first_token_latency_ms {
                latency_emitted = true;
                Some(value)
            } else if measure_first_delta_latency {
                let value = started_at.elapsed().as_millis() as u64;
                first_token_latency_ms = Some(value);
                latency_emitted = true;
                Some(value)
            } else {
                None
            }
        } else {
            None
        };

        emit_stream_event(
            sink,
            "turn:delta",
            turn_id.to_string(),
            "delta",
            Some(phase),
            text.as_ref().map(|_| delta.clone()),
            reasoning_content.as_ref().map(|_| delta.clone()),
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
            latency,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        if index + 1 < chunks.len() {
            std::thread::sleep(std::time::Duration::from_millis(14));
        }
    }

    first_token_latency_ms
}

pub fn stream_reasoning_chunks(
    sink: &impl TurnEventSink,
    turn_id: &str,
    phase: &str,
    reasoning_content: &str,
    started_at: &std::time::Instant,
    initial_latency_ms: Option<u64>,
    measure_first_delta_latency: bool,
) -> Option<u64> {
    stream_delta_chunks(
        sink,
        turn_id,
        phase,
        None,
        Some(reasoning_content),
        started_at,
        initial_latency_ms,
        measure_first_delta_latency,
    )
}

pub fn stream_text_chunks(
    sink: &impl TurnEventSink,
    turn_id: &str,
    phase: &str,
    text: &str,
    started_at: &std::time::Instant,
    initial_latency_ms: Option<u64>,
    measure_first_delta_latency: bool,
) -> Option<u64> {
    stream_delta_chunks(
        sink,
        turn_id,
        phase,
        Some(text),
        None,
        started_at,
        initial_latency_ms,
        measure_first_delta_latency,
    )
}

pub fn runtime_log(message: String) {
    eprintln!("[pony-runtime] {}", message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_trace_event_aligns_with_context_build_end_hook_point() {
        let observation = BuildContextObservation {
            request_format: "chat".to_string(),
            message_count: 3,
            image_count: 0,
            tool_count: 0,
            temperature: 0.2,
            max_output_tokens: 512,
            stable_prefix_text: "prefix".to_string(),
            semi_stable_context_text: "ctx".to_string(),
            volatile_input_text: "input".to_string(),
            prefix_mutation_reasons: Vec::new(),
            request_messages_text: "messages".to_string(),
            tool_definitions_text: String::new(),
        };

        let event_type = resolve_canonical_event_type(
            "turn:trace",
            Some("building_context"),
            Some(&observation),
            None,
        );

        assert_eq!(event_type, "turn.context_built");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
            &event_type,
            "building_context"
        ));
    }

    #[test]
    fn canonical_trace_event_aligns_with_model_call_start_hook_point() {
        let event_type =
            resolve_canonical_event_type("turn:trace", Some("calling_model"), None, None);

        assert_eq!(event_type, "turn.model_call_started");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ModelCallStart,
            &event_type,
            "calling_model"
        ));
    }

    #[test]
    fn canonical_trace_event_prefers_model_call_started_over_context_built_outside_building_context(
    ) {
        let observation = BuildContextObservation {
            request_format: "responses".to_string(),
            message_count: 4,
            image_count: 0,
            tool_count: 1,
            temperature: 0.2,
            max_output_tokens: 512,
            stable_prefix_text: "prefix".to_string(),
            semi_stable_context_text: "ctx".to_string(),
            volatile_input_text: "input".to_string(),
            prefix_mutation_reasons: Vec::new(),
            request_messages_text: "messages".to_string(),
            tool_definitions_text: "tools".to_string(),
        };

        let event_type = resolve_canonical_event_type(
            "turn:trace",
            Some("calling_model"),
            Some(&observation),
            None,
        );

        assert_eq!(event_type, "turn.model_call_started");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ModelCallStart,
            &event_type,
            "calling_model"
        ));
    }

    #[test]
    fn delta_event_aligns_with_model_response_end_hook_point() {
        let event_type =
            resolve_canonical_event_type("turn:delta", Some("streaming_response"), None, None);

        assert_eq!(event_type, "turn.output_delta");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ModelResponseEnd,
            &event_type,
            "streaming_response"
        ));
    }

    #[test]
    fn trace_event_with_observation_outside_building_context_falls_back_to_trace_updated() {
        let observation = BuildContextObservation {
            request_format: "responses".to_string(),
            message_count: 4,
            image_count: 0,
            tool_count: 1,
            temperature: 0.2,
            max_output_tokens: 512,
            stable_prefix_text: "prefix".to_string(),
            semi_stable_context_text: "ctx".to_string(),
            volatile_input_text: "input".to_string(),
            prefix_mutation_reasons: Vec::new(),
            request_messages_text: "messages".to_string(),
            tool_definitions_text: "tools".to_string(),
        };

        let event_type = resolve_canonical_event_type(
            "turn:trace",
            Some("tool_result_integrating"),
            Some(&observation),
            None,
        );

        assert_eq!(event_type, "turn.trace_updated");
    }

    #[test]
    fn tool_event_aligns_with_tool_call_start_hook_point() {
        let tool_activities = vec![TurnToolActivity {
            id: "tool-1".to_string(),
            name: "workspace.read_file".to_string(),
            status: "running".to_string(),
            summary: "running".to_string(),
            arguments_text: None,
            result_text: None,
            duration_seconds: None,
            capability_invocation: None,
        }];
        let event_type = resolve_canonical_event_type(
            "turn:tool",
            Some("executing_tool"),
            None,
            Some(&tool_activities),
        );

        assert_eq!(event_type, "turn.tool_call_started");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ToolCallStart,
            &event_type,
            "executing_tool"
        ));
    }

    #[test]
    fn tool_event_aligns_with_tool_call_end_hook_point() {
        let tool_activities = vec![TurnToolActivity {
            id: "tool-1".to_string(),
            name: "workspace.read_file".to_string(),
            status: "done".to_string(),
            summary: "ok".to_string(),
            arguments_text: None,
            result_text: None,
            duration_seconds: Some(0.1),
            capability_invocation: None,
        }];
        let event_type = resolve_canonical_event_type(
            "turn:tool",
            Some("executing_tool"),
            None,
            Some(&tool_activities),
        );

        assert_eq!(event_type, "turn.tool_call_completed");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::ToolCallEnd,
            &event_type,
            "tool_result_integrating"
        ));
    }

    #[test]
    fn terminal_event_aligns_with_turn_finalize_end_hook_point() {
        let event_type =
            resolve_canonical_event_type("turn:completed", Some("completed"), None, None);

        assert_eq!(event_type, "turn.completed");
        assert!(crate::agent::hooks::hook_point_matches_canonical_boundary(
            &crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
            &event_type,
            "completed"
        ));
    }
}

pub fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let count = normalized.chars().count();
    if count <= max_chars {
        normalized
    } else {
        let preview = normalized.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

pub fn provider_decision<P: ProviderClient>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
) -> Result<ProviderDecision, String> {
    provider.decide_with_tools(request, tools)
}

pub fn provider_decision_stream<P, F>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    on_delta: F,
) -> Result<ProviderDecision, String>
where
    P: ProviderClient,
    F: FnMut(ProviderStreamChunk),
{
    provider.decide_with_tools_stream(request, tools, on_delta)
}

pub fn provider_followup<P: ProviderClient>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Result<crate::agent::provider::ProviderResponse, String> {
    provider.continue_with_tool_result(request, tools, assistant_message, tool_call, tool_result)
}

pub fn provider_followup_stream<P, F>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
    on_delta: F,
) -> Result<crate::agent::provider::ProviderResponse, String>
where
    P: ProviderClient,
    F: FnMut(ProviderStreamChunk),
{
    provider.continue_with_tool_result_stream(
        request,
        tools,
        assistant_message,
        tool_call,
        tool_result,
        on_delta,
    )
}
