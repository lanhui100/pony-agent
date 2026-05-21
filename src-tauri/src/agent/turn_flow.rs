use crate::agent::provider::{
    ProviderClient, ProviderDecision, ProviderManager, ProviderRequest, TokenUsage,
};
use crate::agent::telemetry::{TurnToolActivity, TurnTraceStep};
use crate::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use tauri::{AppHandle, Emitter};

use super::runtime::{TurnResult, TurnStreamEvent};
use super::session::SessionSnapshot;

pub struct PreparedTurn {
    pub user_message: String,
    pub session: SessionSnapshot,
    pub provider: ProviderManager,
    pub tools: Vec<ToolDefinition>,
    pub planning_request: ProviderRequest,
}

pub struct PlannedTurn {
    pub first_decision: ProviderDecision,
    pub resolved_tool_call: Option<ToolCall>,
}

pub struct PersistedTurnOutcome {
    pub session_summary: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

pub struct SyncToolTurnOutcome {
    pub assistant_message: String,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
}

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
    TurnResult {
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
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        user_message,
        assistant_message: assistant_message.clone(),
        trace_steps,
        tool_activities,
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
    app: &AppHandle,
    turn_id: String,
    provider_meta: Option<&ProviderEventMeta>,
    trace_steps: Vec<TurnTraceStep>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    first_token_latency_ms: Option<u64>,
    error: String,
) {
    emit_event(
        app,
        "turn:failed",
        TurnStreamEvent {
            turn_id,
            kind: "failed".to_string(),
            phase: Some("failed".to_string()),
            text: Some("This turn failed.".to_string()),
            error: Some(error),
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms,
            trace_steps: Some(trace_steps),
            tool_activities,
            session_summary: None,
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub fn emit_stream_event(
    app: &AppHandle,
    name: &str,
    turn_id: String,
    kind: &str,
    phase: Option<&str>,
    text: Option<String>,
    provider_meta: Option<&ProviderEventMeta>,
    provider_source: Option<String>,
    provider_mode: Option<String>,
    fallback_reason: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    trace_steps: Option<Vec<TurnTraceStep>>,
    tool_activities: Option<Vec<TurnToolActivity>>,
    session_summary: Option<String>,
) {
    emit_event(
        app,
        name,
        TurnStreamEvent {
            turn_id,
            kind: kind.to_string(),
            phase: phase.map(|value| value.to_string()),
            text,
            error: None,
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source,
            provider_mode,
            fallback_reason,
            input_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            trace_steps,
            tool_activities,
            session_summary,
        },
    );
}

pub fn emit_turn_failed(
    app: &AppHandle,
    turn_id: String,
    provider_requested_name: Option<String>,
    provider_name: Option<String>,
    provider_protocol: Option<String>,
    provider_model: Option<String>,
    trace_steps: Vec<TurnTraceStep>,
    error: String,
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
        app,
        turn_id,
        provider_meta.as_ref(),
        trace_steps,
        None,
        None,
        error,
    );
}

pub fn emit_event(app: &AppHandle, name: &str, payload: TurnStreamEvent) {
    eprintln!(
        "[pony-agent][runtime] emit {} turn={} phase={:?} text_len={} tools={}",
        name,
        payload.turn_id,
        payload.phase,
        payload.text.as_ref().map(|text| text.len()).unwrap_or(0),
        payload
            .tool_activities
            .as_ref()
            .map(|tools| tools.len())
            .unwrap_or(0)
    );
    let _ = app.emit(name, payload);
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
) -> (Option<u64>, Option<u64>, Option<u64>) {
    match token_usage {
        Some(token_usage) => (
            token_usage.input_tokens,
            token_usage.output_tokens,
            token_usage.total_tokens,
        ),
        None => (None, None, None),
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

pub fn stream_text_chunks(
    app: &AppHandle,
    turn_id: &str,
    phase: &str,
    text: &str,
    started_at: &std::time::Instant,
) -> Option<u64> {
    let mut first_token_latency_ms = None;

    for chunk in text.as_bytes().chunks(48) {
        let delta = String::from_utf8_lossy(chunk).to_string();
        let latency = if first_token_latency_ms.is_none() {
            let value = started_at.elapsed().as_millis() as u64;
            first_token_latency_ms = Some(value);
            Some(value)
        } else {
            None
        };

        emit_stream_event(
            app,
            "turn:delta",
            turn_id.to_string(),
            "delta",
            Some(phase),
            Some(delta),
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
        );
    }

    first_token_latency_ms
}

pub fn runtime_log(message: String) {
    eprintln!("[pony-runtime] {}", message);
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

pub fn provider_followup<P: ProviderClient>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Result<crate::agent::provider::ProviderResponse, String> {
    provider.continue_with_tool_result(request, tools, tool_call, tool_result)
}

pub fn provider_followup_stream<P, F>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    tool_call: &ToolCall,
    tool_result: &ToolResult,
    on_delta: F,
) -> Result<crate::agent::provider::ProviderResponse, String>
where
    P: ProviderClient,
    F: FnMut(String),
{
    provider.continue_with_tool_result_stream(request, tools, tool_call, tool_result, on_delta)
}
