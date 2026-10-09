pub mod client;
pub mod config;
pub mod models;
pub mod sanitizer;

pub use client::PonySentryClient;
pub use config::PonySentryConfig;
pub use models::{
    AgentTracePayload, Breadcrumb, EvalStatus, Exception, Frame, IngestPayload, ToolCallTraceItem,
    TurnTraceItem,
};
pub use sanitizer::{sanitize, sanitize_json};

use std::collections::HashMap;
use std::panic::PanicHookInfo;
use std::sync::{OnceLock, RwLock};

static GLOBAL_CLIENT: OnceLock<RwLock<PonySentryClient>> = OnceLock::new();

fn get_global() -> &'static RwLock<PonySentryClient> {
    GLOBAL_CLIENT.get_or_init(|| {
        let cfg = PonySentryConfig::from_env();
        RwLock::new(PonySentryClient::new(cfg))
    })
}

pub fn init(config: PonySentryConfig) {
    let client = PonySentryClient::new(config);
    if let Some(lock) = GLOBAL_CLIENT.get() {
        if let Ok(mut write_guard) = lock.write() {
            *write_guard = client;
            return;
        }
    }
    let _ = GLOBAL_CLIENT.set(RwLock::new(client));
}

pub fn capture_error(error_type: &str, message: &str, extra: Option<serde_json::Value>) {
    if let Ok(client) = get_global().read() {
        client.capture_error(error_type, message, extra);
    }
}

pub fn capture_payload(payload: IngestPayload) {
    if let Ok(client) = get_global().read() {
        client.capture_payload(payload);
    }
}

pub fn capture_agent_trace(trace: AgentTracePayload) {
    if let Ok(client) = get_global().read() {
        client.capture_agent_trace(trace);
    }
}

pub fn capture_panic(info: &PanicHookInfo) {
    if let Ok(client) = get_global().read() {
        client.capture_panic(info);
    }
}

pub fn add_breadcrumb(
    category: &str,
    message: &str,
    data: Option<HashMap<String, String>>,
) {
    if let Ok(client) = get_global().read() {
        client.add_breadcrumb(category, message, data);
    }
}

pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        capture_panic(info);
        prev(info);
    }));
}

/// 检查 PonySentry 全局客户端是否处于启用状态
pub fn is_enabled() -> bool {
    if let Ok(client) = get_global().read() {
        client.config.enabled
    } else {
        false
    }
}

/// 判定事件是否为可上报 Trace 的终态事件。
/// 兼容两种事件命名：运行时 `TurnStreamEvent.kind` 为短名（"completed"/"failed"/"cancelled"），
/// 事件发射名（emit name）为带前缀（"turn:completed" 等）。两者均视为可上报终态；
/// 其余事件（含 "turn:suspended"、"turn:trace"、"turn:step" 等）返回 false。
pub fn is_trace_reportable_terminal(name: &str) -> bool {
    matches!(
        name,
        "turn:completed" | "turn:failed" | "turn:cancelled" | "completed" | "failed" | "cancelled"
    )
}

/// 从终态 TurnStreamEvent 映射并构建 AgentTracePayload
/// 若 turn_id 为空或 phase 为空（或不在终态定义中），则返回 None
pub fn build_agent_trace_from_event(
    payload: &crate::agent::runtime::TurnStreamEvent,
) -> Option<AgentTracePayload> {
    let turn_id = payload.turn_id.trim();
    if turn_id.is_empty() {
        return None;
    }

    let phase = payload.phase.as_deref().unwrap_or("").trim();
    if phase.is_empty() {
        return None;
    }

    let session_id = payload
        .session_id
        .clone()
        .filter(|s| !s.trim().is_empty())?;

    let tool_calls: Vec<ToolCallTraceItem> = payload
        .tool_activities
        .as_ref()
        .map(|activities| {
            activities
                .iter()
                .map(|act| ToolCallTraceItem {
                    call_id: Some(act.id.clone()),
                    tool_name: act.name.clone(),
                    arguments_summary: act.arguments_text.clone(),
                    status: act.status.clone(),
                    duration_ms: act.duration_seconds.map(|d| (d * 1000.0) as u64),
                    // act.error 是 Option<serde_json::Value>：字符串取裸文本（as_str），
                    // 避免 to_string() 产生带引号/转义的 JSON 二次编码；其他类型降级为 to_string。
                    error: act.error.as_ref().map(|e| match e {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    }),
                })
                .collect()
        })
        .unwrap_or_default();

    let turn_item = TurnTraceItem {
        turn_id: turn_id.to_string(),
        sequence: payload.sequence,
        phase: Some(phase.to_string()),
        provider: payload.provider_name.clone(),
        model: payload.provider_model.clone(),
        input_tokens: payload.input_tokens,
        output_tokens: payload.output_tokens,
        cache_hit_tokens: payload.cache_hit_input_tokens,
        duration_ms: payload.turn_duration_ms,
        error: payload.error.clone(),
        tool_calls,
        started_at_ms: None,
        completed_at_ms: payload.emitted_at_ms,
    };

    let (environment, release) = if let Ok(client) = get_global().read() {
        (client.config.environment.clone(), client.config.release.clone())
    } else {
        ("dev".to_string(), "0.1.109".to_string())
    };

    let reported_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Some(AgentTracePayload {
        session_id,
        run_id: None,
        turn_id: Some(turn_id.to_string()),
        environment,
        release,
        eval_status: EvalStatus::Unreviewed,
        turns: vec![turn_item],
        tags: None,
        extra: None,
        total_input_tokens: payload.input_tokens,
        total_output_tokens: payload.output_tokens,
        total_duration_ms: payload.turn_duration_ms,
        reported_at_ms,
    })
}

/// 尝试自动上报 Turn Trace
/// 仅当全局 PonySentry 启用且事件为可上报终态时，执行映射并异步推入发送队列；
/// 否则零开销跳过（含 suspended 可恢复态，防重复上报）。
pub fn maybe_report_turn_trace(payload: &crate::agent::runtime::TurnStreamEvent) {
    if !is_enabled() || !is_trace_reportable_terminal(&payload.kind) {
        return;
    }
    if let Some(trace) = build_agent_trace_from_event(payload) {
        capture_agent_trace(trace);
    }
}

