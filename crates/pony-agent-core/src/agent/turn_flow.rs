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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::context::RetrievedContextState;
use super::runtime::{TurnResult, TurnStreamEvent};

pub trait TurnEventSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent);
    /// 事件溯源（PA-091）：将 turn 内过程事实入事件缓冲（默认空实现——
    /// 生产路径经全局注册表持久化；测试 sink 可覆写为收集）。
    /// 持久化失败 contained（只记日志不阻断流）；构造失败由调用方 fail loud。
    fn persist(&self, _event: crate::agent::turn_event::TurnEvent) {}
}

/// PA-091：全局事件持久化注册表（OnceLock 模式，与 turn_event_sequence_registry 一致）。
/// 生产路径由 HostControlPlane 初始化时注册（持有 sessions_rwlock 的 Arc）；
/// 测试可注册 mock / 清空。签名：(session_id, turn_id, event, is_terminal)。
type EventPersistFn = dyn Fn(&str, &str, crate::agent::turn_event::TurnEvent, bool) + Send + Sync;

static EVENT_PERSIST_REGISTRY: OnceLock<Mutex<Option<Arc<EventPersistFn>>>> = OnceLock::new();

fn event_persist_registry() -> &'static Mutex<Option<Arc<EventPersistFn>>> {
    EVENT_PERSIST_REGISTRY.get_or_init(|| Mutex::new(None))
}

/// 注册生产事件持久化通道（幂等：重复注册覆盖）。
pub fn register_event_persist(f: Arc<EventPersistFn>) {
    let mut slot = event_persist_registry()
        .lock()
        .expect("event persist registry lock poisoned");
    *slot = Some(f);
}

/// 清空事件持久化通道（测试用）。
pub fn clear_event_persist() {
    let mut slot = event_persist_registry()
        .lock()
        .expect("event persist registry lock poisoned");
    *slot = None;
}

/// PA-093：非 turn 生命周期事实（checkpoint/checkout、fork/created）经全局
/// 注册表落盘（立即 flush）。turn_id 用语义化字面量（事件无 turn 归属，列非空）。
/// 未注册通道（测试/老路径）时静默跳过——与 turn 事件路径的 contained 语义一致。
pub fn emit_global_event(session_id: &str, turn_id: &str, event: crate::agent::turn_event::TurnEvent) {
    if let Some(persist) = current_event_persist() {
        persist(session_id, turn_id, event, true);
    }
}

/// 当前注册的持久化通道（None = 未注册，事件不落盘）。
fn current_event_persist() -> Option<Arc<EventPersistFn>> {
    event_persist_registry()
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
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

pub struct ModelHopTraceContent {
    pub text: String,
    pub reasoning_content: Option<String>,
}

pub struct SyncToolTurnOutcome {
    pub assistant_message: String,
    pub assistant_reasoning_content: Option<String>,
    pub provider_native_transcript: Option<Vec<Value>>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub model_hop_trace_contents: Vec<ModelHopTraceContent>,
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
            text: Some("用户终止，发送消息可继续。".to_string()),
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
    let is_delta_event = name == "turn:delta";
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
            build_context_observation: if is_delta_event {
                None
            } else {
                build_context_observation
            },
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            trace_steps: if is_delta_event { None } else { trace_steps },
            trace_timeline: if is_delta_event { None } else { trace_timeline },
            tool_activities: if is_delta_event {
                None
            } else {
                tool_activities
            },
            provider_call_records: if is_delta_event {
                None
            } else {
                provider_call_records
            },
            hook_trace_records: if is_delta_event {
                None
            } else {
                hook_trace_records
            },
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
        "turn:output_end" => "turn.output_end".to_string(),
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
    sink.emit(name, payload.clone());
    // PA-091：事件溯源——构造 TurnEvent 经全局注册表持久化（缓冲 + turn 终态 flush）。
    // 构造失败 fail loud（数据完整性错误）；持久化失败由注册通道 contained。
    if let Some(event) = build_turn_event(name, &payload) {
        if let Some(persist) = current_event_persist() {
            let session_id = payload.session_id.as_deref().unwrap_or("");
            let is_terminal = matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled");
            persist(session_id, &payload.turn_id, event, is_terminal);
        }
    }
    if matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled") {
        clear_turn_event_sequence(&terminal_turn_id);
    }
}

/// PA-091：TurnStreamEvent → TurnEvent 映射（design.md 映射表）。
/// `turn:output_end` / `turn:trace` / `turn.context_built` 显式不落盘（返回 None）。
fn build_turn_event(name: &str, payload: &TurnStreamEvent) -> Option<crate::agent::turn_event::TurnEvent> {
    use crate::agent::turn_event::{TurnEndReason, TurnEvent};
    match name {
        "turn:started" => Some(TurnEvent::TurnStart {
            turn_id: payload.turn_id.clone(),
        }),
        "turn:delta" => {
            // reasoning-only chunk（无 text）也落盘（text 为空串），保证
            // "every emitted event SHALL be persisted" 的语义完整。
            Some(TurnEvent::AssistantChunk {
                turn_id: payload.turn_id.clone(),
                // 阶段 1 单步语义：step 固定 0（写时聚合键 (turn_id, step) 生效）。
                step: 0,
                text: payload.text.clone().unwrap_or_default(),
            })
        }
        "turn:completed" => Some(TurnEvent::AssistantMessage {
            turn_id: payload.turn_id.clone(),
            step: 0,
            text: payload.text.clone().unwrap_or_default(),
            reasoning_content: payload.reasoning_content.clone(),
            usage: payload_usage(payload),
            chunk_missing: None,
        }),
        "turn:failed" => Some(TurnEvent::TurnEnd {
            turn_id: payload.turn_id.clone(),
            reason: TurnEndReason::Error,
            turn_duration_ms: payload.turn_duration_ms,
        }),
        "turn:cancelled" => Some(TurnEvent::TurnEnd {
            turn_id: payload.turn_id.clone(),
            reason: TurnEndReason::Cancelled,
            turn_duration_ms: payload.turn_duration_ms,
        }),
        "turn:tool" => build_tool_event(payload),
        _ => None,
    }
}

/// 从终态 payload 的 token 字段构造 usage（全 None 时返回 None）。
fn payload_usage(payload: &TurnStreamEvent) -> Option<crate::agent::provider::TokenUsage> {
    if payload.input_tokens.is_none()
        && payload.cache_hit_input_tokens.is_none()
        && payload.reasoning_tokens.is_none()
        && payload.output_tokens.is_none()
        && payload.total_tokens.is_none()
    {
        return None;
    }
    Some(crate::agent::provider::TokenUsage {
        input_tokens: payload.input_tokens,
        cache_hit_input_tokens: payload.cache_hit_input_tokens,
        cache_hit_source: None,
        reasoning_tokens: payload.reasoning_tokens,
        output_tokens: payload.output_tokens,
        total_tokens: payload.total_tokens,
    })
}

/// turn:tool → ToolCall（running）/ ToolResult（completed），取最后一个 activity。
fn build_tool_event(payload: &TurnStreamEvent) -> Option<crate::agent::turn_event::TurnEvent> {
    use crate::agent::turn_event::TurnEvent;
    let activity = payload.tool_activities.as_deref()?.last()?;
    let step = payload.sequence.unwrap_or(0) as u32;
    if activity.status == "running" {
        Some(TurnEvent::ToolCall {
            turn_id: payload.turn_id.clone(),
            step,
            call_id: activity.id.clone(),
            name: activity.name.clone(),
            arguments: activity.arguments_text.clone().unwrap_or_default(),
            started_at_ms: None,
        })
    } else {
        Some(TurnEvent::ToolResult {
            turn_id: payload.turn_id.clone(),
            step,
            call_id: activity.id.clone(),
            result: activity.result_text.clone(),
            error: activity.error.as_ref().map(|e| e.to_string()),
            status: Some(activity.status.clone()),
            duration_ms: activity.duration_seconds.map(|seconds| (seconds * 1000.0) as u64),
            artifacts: activity.artifacts.clone(),
            capability_invocation: activity.capability_invocation.clone(),
        })
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
    use std::cell::RefCell;

    struct RecordingSink {
        events: RefCell<Vec<(String, TurnStreamEvent)>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
            }
        }
    }

    impl TurnEventSink for RecordingSink {
        fn emit(&self, name: &str, payload: TurnStreamEvent) {
            self.events.borrow_mut().push((name.to_string(), payload));
        }
    }

    fn sample_payload(turn_id: &str, kind: &str, text: Option<&str>) -> TurnStreamEvent {
        TurnStreamEvent {
            event_id: None,
            session_id: Some("s1".to_string()),
            turn_id: turn_id.to_string(),
            kind: kind.to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: Some("calling_model".to_string()),
            text: text.map(str::to_string),
            reasoning_content: None,
            error: None,
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            trace_steps: None,
            trace_timeline: None,
            tool_activities: None,
            provider_call_records: None,
            hook_trace_records: None,
            session_summary: None,
        }
    }

    #[test]
    fn event_name_mapping_table_driven() {
        use crate::agent::turn_event::TurnEvent;
        // 8 种现有发射名 → 映射断言（design.md 映射表）
        let cases: Vec<(&str, Option<&str>)> = vec![
            ("turn:started", Some("turn/start")),
            ("turn:delta", Some("assistant/chunk")),
            ("turn:output_end", None),
            ("turn:completed", Some("assistant/message")),
            ("turn:failed", Some("turn/end")),
            ("turn:cancelled", Some("turn/end")),
            ("turn:trace", None),
            ("turn:tool", None), // 无 tool_activities 时不落盘
        ];
        for (name, expected) in cases {
            let payload = sample_payload("turn-1", name, Some("hi"));
            let event = build_turn_event(name, &payload);
            match expected {
                Some(type_name) => {
                    let event = event.unwrap_or_else(|| panic!("{name} must map to {type_name}"));
                    assert_eq!(event.type_name(), type_name, "{name} mapping");
                }
                None => assert!(event.is_none(), "{name} must not persist"),
            }
        }
        // reason 分支断言：failed → Error，cancelled → Cancelled
        use crate::agent::turn_event::TurnEndReason;
        let failed = build_turn_event("turn:failed", &sample_payload("t1", "turn:failed", None))
            .expect("failed event");
        match failed {
            TurnEvent::TurnEnd { reason, .. } => {
                assert_eq!(reason, TurnEndReason::Error, "failed reason");
            }
            _ => panic!("expected TurnEnd"),
        }
        let cancelled =
            build_turn_event("turn:cancelled", &sample_payload("t1", "turn:cancelled", None))
                .expect("cancelled event");
        match cancelled {
            TurnEvent::TurnEnd { reason, .. } => {
                assert_eq!(reason, TurnEndReason::Cancelled, "cancelled reason");
            }
            _ => panic!("expected TurnEnd"),
        }
    }

    #[test]
    fn emit_event_persist_channel_buffers_aggregates_and_flushes() {
        use crate::agent::turn_event::TurnEvent;
        use std::sync::{Arc, Mutex};
        // 注册测试持久化通道：收集 (session_id, turn_id, events, is_terminal)
        let received: Arc<Mutex<Vec<(String, String, Vec<TurnEvent>, bool)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let sink = RecordingSink::new();
        let received_clone = Arc::clone(&received);
        register_event_persist(Arc::new(
            move |session_id, turn_id, event, is_terminal| {
                received_clone
                    .lock()
                    .expect("received lock")
                    .push((
                        session_id.to_string(),
                        turn_id.to_string(),
                        vec![event],
                        is_terminal,
                    ));
            },
        ));
        // emit 序列：started → delta ×2（同 turn 应合并为一条 chunk）→ completed
        emit_event(&sink, "turn:started", sample_payload("turn-1", "turn:started", None));
        emit_event(&sink, "turn:delta", sample_payload("turn-1", "turn:delta", Some("hel")));
        emit_event(&sink, "turn:delta", sample_payload("turn-1", "turn:delta", Some("lo")));
        emit_event(
            &sink,
            "turn:completed",
            sample_payload("turn-1", "turn:completed", Some("hello")),
        );
        // 验证：4 次 emit → 4 条通道记录（聚合在持久化闭包内做，通道收到原事件；
        // 聚合正确性由 control_plane 闭包负责——此处验证 emit→persist 链路与终态标记）
        let records = received.lock().expect("received lock");
        assert_eq!(records.len(), 4, "every emitted event reaches persist");
        assert_eq!(records[0].3, false, "started not terminal");
        assert_eq!(records[3].3, true, "completed is terminal");
        assert_eq!(records[0].2[0].type_name(), "turn/start");
        assert_eq!(records[1].2[0].type_name(), "assistant/chunk");
        assert_eq!(records[3].2[0].type_name(), "assistant/message");
        drop(records);
        clear_event_persist();
    }

    #[test]
    fn event_mapping_tool_started_and_completed() {
        use crate::agent::turn_event::TurnEvent;
        use crate::agent::telemetry::TurnToolActivity;
        // running → ToolCall
        let mut payload = sample_payload("turn-1", "turn:tool", None);
        payload.tool_activities = Some(vec![TurnToolActivity {
            id: "act-1".to_string(),
            name: "bash".to_string(),
            canonical_tool_name: None,
            display_name_zh: None,
            status: "running".to_string(),
            description: "run".to_string(),
            arguments_text: Some("{}".to_string()),
            result_text: None,
            duration_seconds: None,
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }]);
        let event = build_turn_event("turn:tool", &payload).expect("tool call event");
        assert_eq!(event.type_name(), "tool/call");
        // completed → ToolResult
        payload.tool_activities = Some(vec![TurnToolActivity {
            id: "act-1".to_string(),
            name: "bash".to_string(),
            canonical_tool_name: None,
            display_name_zh: None,
            status: "done".to_string(),
            description: "run".to_string(),
            arguments_text: Some("{}".to_string()),
            result_text: Some("ok".to_string()),
            duration_seconds: Some(0.5),
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }]);
        let event = build_turn_event("turn:tool", &payload).expect("tool result event");
        assert_eq!(event.type_name(), "tool/result");
        match event {
            TurnEvent::ToolResult {
                result, duration_ms, ..
            } => {
                assert_eq!(result.as_deref(), Some("ok"));
                assert_eq!(duration_ms, Some(500));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn event_mapping_completed_carries_usage() {
        use crate::agent::turn_event::TurnEvent;
        let mut payload = sample_payload("turn-1", "turn:completed", Some("done"));
        payload.input_tokens = Some(100);
        payload.output_tokens = Some(50);
        payload.total_tokens = Some(150);
        let event = build_turn_event("turn:completed", &payload).expect("completed event");
        match event {
            TurnEvent::AssistantMessage { usage, text, .. } => {
                assert_eq!(text, "done");
                let usage = usage.expect("usage present");
                assert_eq!(usage.input_tokens, Some(100));
                assert_eq!(usage.output_tokens, Some(50));
            }
            _ => panic!("expected AssistantMessage"),
        }
    }

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
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
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
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
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
    fn delta_stream_event_strips_heavy_payload_fields() {
        let sink = RecordingSink::new();
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
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
            request_messages_text: "messages".to_string(),
            tool_definitions_text: "tools".to_string(),
        };

        emit_stream_event(
            &sink,
            "turn:delta",
            "turn-heavy-delta".to_string(),
            "delta",
            Some("calling_model"),
            Some("partial".to_string()),
            Some("thinking".to_string()),
            None,
            None,
            None,
            None,
            Some(observation),
            Some(11),
            Some(3),
            Some(5),
            Some(7),
            Some(18),
            Some(42),
            None,
            Some(vec![TurnTraceStep {
                id: "call-model".to_string(),
                label: "Call model".to_string(),
                state: "active".to_string(),
            }]),
            Some(vec![TraceTimelineEntry {
                id: "model-1".to_string(),
                kind: "call_model".to_string(),
                label: "CALL MODEL #1".to_string(),
                state: "active".to_string(),
                sequence: 1,
                text: Some("partial".to_string()),
                ..TraceTimelineEntry::default()
            }]),
            Some(vec![TurnToolActivity {
                id: "tool-1".to_string(),
                name: "workspace.read_file".to_string(),
                canonical_tool_name: Some("Read".to_string()),
                display_name_zh: Some("读取".to_string()),
                status: "done".to_string(),
                description: "read done".to_string(),
                arguments_text: None,
                result_text: None,
                duration_seconds: Some(0.1),
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: None,
            }]),
            Some(vec![ProviderCallCacheRecord::default()]),
            Some(Vec::new()),
            Some("summary".to_string()),
            Some("session-1".to_string()),
        );

        let events = sink.events.borrow();
        let (_, payload) = events
            .first()
            .expect("delta event should have been emitted");

        assert_eq!(payload.text.as_deref(), Some("partial"));
        assert_eq!(payload.reasoning_content.as_deref(), Some("thinking"));
        assert_eq!(payload.first_token_latency_ms, Some(42));
        assert_eq!(payload.session_id.as_deref(), Some("session-1"));
        assert_eq!(payload.session_summary.as_deref(), Some("summary"));
        assert!(payload.build_context_observation.is_none());
        assert!(payload.trace_steps.is_none());
        assert!(payload.trace_timeline.is_none());
        assert!(payload.tool_activities.is_none());
        assert!(payload.provider_call_records.is_none());
        assert!(payload.hook_trace_records.is_none());
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
            context_refresh_reason: None,
            instruction_scope_sources: Vec::new(),
            conversation_carry_mode: None,
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
            canonical_tool_name: Some("Read".to_string()),
            display_name_zh: Some("读取".to_string()),
            status: "running".to_string(),
            description: "running".to_string(),
            arguments_text: None,
            result_text: None,
            duration_seconds: None,
            parent_activity_id: None,
            artifacts: None,
            error: None,
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
            canonical_tool_name: Some("Read".to_string()),
            display_name_zh: Some("读取".to_string()),
            status: "done".to_string(),
            description: "ok".to_string(),
            arguments_text: None,
            result_text: None,
            duration_seconds: Some(0.1),
            parent_activity_id: None,
            artifacts: None,
            error: None,
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
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Result<crate::agent::provider::ProviderResponse, String> {
    provider.continue_with_tool_result(
        request,
        tools,
        accumulated_messages,
        assistant_message,
        tool_call,
        tool_result,
    )
}

pub fn provider_followup_stream<P, F>(
    provider: &P,
    request: &ProviderRequest,
    tools: &[ToolDefinition],
    accumulated_messages: &mut Vec<Value>,
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
        accumulated_messages,
        assistant_message,
        tool_call,
        tool_result,
        on_delta,
    )
}
