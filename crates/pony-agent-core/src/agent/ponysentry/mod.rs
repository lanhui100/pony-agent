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

/// 直接异步上报单个 AgentTracePayload 并等待结果（手动上传用）
pub async fn send_agent_trace_direct(trace: AgentTracePayload) -> Result<(), String> {
    let (endpoint, client_token, enabled) = {
        let client = get_global()
            .read()
            .map_err(|e| format!("client lock poisoned: {e}"))?;
        (
            client.config.endpoint.clone(),
            client.config.client_token.clone(),
            client.config.enabled,
        )
    };

    if !enabled {
        return Err("PonySentry is disabled".to_string());
    }

    client::send_trace_direct_http(&endpoint, client_token.as_deref(), trace).await
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
        input_text: None,
        output_text: None,
    };

    let (environment, release) = if let Ok(client) = get_global().read() {
        (client.config.environment.clone(), client.config.release.clone())
    } else {
        ("dev".to_string(), env!("CARGO_PKG_VERSION").to_string())
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
        project: None,
        stats_incomplete: false,
        wall_clock_ms: None,
        inter_turn_pause_ms: None,
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

/// 从会话的持久化 TurnTraceRecord 列表构造单个聚合 Session Trace Payload。
/// 符合业界标准（Session 级 Trace + 时序 turns 列表）：
/// - 将多轮对话按时序汇总进同一个 `AgentTracePayload` 的 `turns` 列表中；
/// - 排序键为 `(emitted_at_ms, sequence)`（先时间戳、后 sequence，缺失按 0 兜底）；
/// - 自动汇总全局 `total_input_tokens`, `total_output_tokens`, `total_duration_ms`；
///   若任一 turn 的 input/output tokens 缺失则 `stats_incomplete = true`；
/// - `wall_clock_ms` = max(completed_at_ms) - min(emitted_at_ms)（以 emitted_at 充当
///   起止时间；存在缺失时 None）；`inter_turn_pause_ms` = 相邻 turn emitted_at 间隔的均值
///   （不足 2 轮或存在缺失时 None）；
/// - turn 的 `input_text` / `output_text` 经 `sanitize` 脱敏后写入；
/// - `project` 透传为会话级元数据；
/// - 空会话无有效 turn 时返回 None。
pub fn build_session_aggregated_trace_payload(
    session_id: &str,
    project: Option<String>,
    mut traces: Vec<crate::agent::session::TurnTraceRecord>,
) -> Option<AgentTracePayload> {
    if traces.is_empty() {
        return None;
    }

    // 先时间戳、后 sequence 的确定性时序排序（时间戳为主键，sequence 兜底）。
    traces.sort_by_key(|t| (t.emitted_at_ms.unwrap_or(0), t.sequence.unwrap_or(0)));

    let (environment, release) = if let Ok(client) = get_global().read() {
        (client.config.environment.clone(), client.config.release.clone())
    } else {
        ("dev".to_string(), env!("CARGO_PKG_VERSION").to_string())
    };

    let reported_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut turn_items = Vec::new();
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut total_duration_ms: u64 = 0;
    let mut latest_turn_id: Option<String> = None;
    // 任一 turn 的 input/output tokens 缺失即视为统计不完整。
    let mut stats_incomplete = false;
    // 已纳入 payload 的 turn 数与其 emitted_at_ms（排序后；用于 wall_clock 与间隔统计）。
    let mut included_count: usize = 0;
    let mut emitted_ats: Vec<u64> = Vec::new();

    for trace in traces {
        let turn_id = trace.turn_id.trim();
        if turn_id.is_empty() {
            continue;
        }

        latest_turn_id = Some(turn_id.to_string());
        total_input_tokens = total_input_tokens.saturating_add(trace.input_tokens.unwrap_or(0));
        total_output_tokens = total_output_tokens.saturating_add(trace.output_tokens.unwrap_or(0));
        total_duration_ms = total_duration_ms.saturating_add(trace.turn_duration_ms.unwrap_or(0));
        if trace.input_tokens.is_none() || trace.output_tokens.is_none() {
            stats_incomplete = true;
        }
        included_count += 1;
        if let Some(ts) = trace.emitted_at_ms {
            emitted_ats.push(ts);
        }

        let tool_calls: Vec<ToolCallTraceItem> = trace
            .tool_activities
            .into_iter()
            .map(|act| ToolCallTraceItem {
                call_id: Some(act.id.clone()),
                tool_name: act.name.clone(),
                arguments_summary: act.arguments_text.clone(),
                status: act.status.clone(),
                duration_ms: act.duration_seconds.map(|d| (d * 1000.0) as u64),
                error: act.error.as_ref().map(|e| match e {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                }),
            })
            .collect();

        turn_items.push(TurnTraceItem {
            turn_id: turn_id.to_string(),
            sequence: trace.sequence,
            phase: Some(trace.phase.clone()),
            provider: trace.provider_name.clone(),
            model: trace.provider_model.clone(),
            input_tokens: trace.input_tokens,
            output_tokens: trace.output_tokens,
            cache_hit_tokens: trace.cache_hit_input_tokens,
            duration_ms: trace.turn_duration_ms,
            error: trace.error.clone(),
            tool_calls,
            started_at_ms: None,
            completed_at_ms: trace.emitted_at_ms,
            // 对话文本经 PonySentry 脱敏管线处理后写入，避免路径/密钥泄漏。
            input_text: trace.input_text.as_deref().map(sanitize),
            output_text: trace.output_text.as_deref().map(sanitize),
        });
    }

    if turn_items.is_empty() {
        return None;
    }

    // 全部纳入 turn 均有 emitted_at 时方可计算 wall_clock 与间隔均值，否则 None。
    let all_timestamps_present = emitted_ats.len() == included_count;
    let wall_clock_ms = if all_timestamps_present && !emitted_ats.is_empty() {
        Some(
            emitted_ats.iter().max().copied().unwrap_or(0)
                - emitted_ats.iter().min().copied().unwrap_or(0),
        )
    } else {
        None
    };
    let inter_turn_pause_ms = if all_timestamps_present && emitted_ats.len() >= 2 {
        let total_gap: u64 = emitted_ats
            .windows(2)
            .map(|w| w[1].saturating_sub(w[0]))
            .sum();
        Some(total_gap / (emitted_ats.len() as u64 - 1))
    } else {
        None
    };

    Some(AgentTracePayload {
        session_id: session_id.to_string(),
        run_id: None,
        turn_id: latest_turn_id,
        environment,
        release,
        eval_status: EvalStatus::Unreviewed,
        turns: turn_items,
        tags: None,
        extra: None,
        total_input_tokens: Some(total_input_tokens),
        total_output_tokens: Some(total_output_tokens),
        total_duration_ms: Some(total_duration_ms),
        project,
        stats_incomplete,
        wall_clock_ms,
        inter_turn_pause_ms,
        reported_at_ms,
    })
}

/// 构造单个分片 AgentTracePayload。
/// `is_first`：首个分片携带聚合汇总元数据（继承 base 的所有会话级字段）；
/// 其余分片仅携带 turns 子集与 session_id（其余字段置默认/None，仅保留上报必需字段）。
#[allow(clippy::too_many_arguments)]
fn build_shard_payload(
    session_id: &str,
    is_first: bool,
    turns: Vec<TurnTraceItem>,
    base: &AgentTracePayload,
) -> AgentTracePayload {
    if is_first {
        let mut payload = base.clone();
        payload.turns = turns;
        return payload;
    }
    AgentTracePayload {
        session_id: session_id.to_string(),
        run_id: None,
        turn_id: None,
        environment: base.environment.clone(),
        release: base.release.clone(),
        eval_status: EvalStatus::Unreviewed,
        turns,
        tags: None,
        extra: None,
        total_input_tokens: None,
        total_output_tokens: None,
        total_duration_ms: None,
        project: None,
        stats_incomplete: false,
        wall_clock_ms: None,
        inter_turn_pause_ms: None,
        reported_at_ms: base.reported_at_ms,
    }
}

/// 将会话全部持久化 TurnTraceRecord 聚合并按时序切分为多个 AgentTracePayload 分片。
/// 每个分片携带同一 session_id 的 turns 连续子集，且分片本身序列化后
/// （`serde_json::to_vec` 长度）不超过 `max_bytes`；首个分片携带聚合汇总元数据
/// （project/stats_incomplete/wall_clock_ms/inter_turn_pause_ms/全局 tokens），
/// 其余分片仅 turns 与 session_id 语义字段。
/// 空会话（无有效 turn）返回空 Vec；单个 turn 超过 max_bytes 时仍独立成片（尽力约束）。
pub fn build_session_trace_payloads_sharded(
    session_id: &str,
    project: Option<&str>,
    traces: Vec<crate::agent::session::TurnTraceRecord>,
    max_bytes: usize,
) -> Vec<AgentTracePayload> {
    let Some(mut aggregated) =
        build_session_aggregated_trace_payload(session_id, project.map(str::to_string), traces)
    else {
        return Vec::new();
    };
    let all_turns = std::mem::take(&mut aggregated.turns);

    // serde_json 为紧凑输出：分片大小 = 空 turns 基座长度 + Σ(turn 长度) + n - 1（逗号分隔）。
    // 先一次性测量基座与每个 turn 的序列化长度，贪心切分为 O(n) 精确计算。
    let first_base_len = serde_json::to_vec(&build_shard_payload(session_id, true, Vec::new(), &aggregated))
        .map(|b| b.len())
        .unwrap_or(0);
    let minimal_base_len = serde_json::to_vec(&build_shard_payload(session_id, false, Vec::new(), &aggregated))
        .map(|b| b.len())
        .unwrap_or(0);

    let mut chunks: Vec<Vec<TurnTraceItem>> = Vec::new();
    let mut current_start = 0usize;
    let mut current_sum = 0usize; // 当前片内 turn 序列化长度之和
    let mut current_count = 0usize;

    for (index, turn) in all_turns.iter().enumerate() {
        let turn_len = serde_json::to_vec(turn).map(|b| b.len()).unwrap_or(0);
        // 候选片（current + 本 turn）的精确长度：base + Σ + n' - 1（n' = current_count + 1）。
        // 首个分片携带汇总元数据（基座更大）；未 flush 前当前片始终按首个分片计量。
        let is_first_candidate = chunks.is_empty();
        let base_len = if is_first_candidate {
            first_base_len
        } else {
            minimal_base_len
        };
        let candidate_len = base_len + current_sum + turn_len + current_count;
        if current_count > 0 && candidate_len > max_bytes {
            chunks.push(all_turns[current_start..index].to_vec());
            current_start = index;
            current_sum = 0;
            current_count = 0;
        }
        current_sum += turn_len;
        current_count += 1;
    }
    if current_count > 0 {
        chunks.push(all_turns[current_start..].to_vec());
    }

    chunks
        .into_iter()
        .enumerate()
        .map(|(index, turns)| build_shard_payload(session_id, index == 0, turns, &aggregated))
        .collect()
}

/// 兼容老接口
pub fn build_agent_trace_payloads_from_turns(
    session_id: &str,
    traces: Vec<crate::agent::session::TurnTraceRecord>,
) -> Vec<AgentTracePayload> {
    build_session_aggregated_trace_payload(session_id, None, traces)
        .map(|p| vec![p])
        .unwrap_or_default()
}

