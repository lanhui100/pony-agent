use super::*;

use std::collections::BTreeMap;

pub(super) const STREAM_REASONING_BATCH_CHARS: usize = 96;

/// F3（I2）：followup 收尾把 pending 的 delta 帧**无视合并窗口**一次发完。
///
/// 语义契约：允许合并，禁止丢最终帧——收尾点若因窗口未到而丢帧，前端会少最后一段正文。
/// 从 `turn_stream.rs::handle_stream_tool_turn` 外移（该函数已顶到规模基线）。
pub(super) fn flush_tail_delta_frames<S: TurnEventSink>(
    sink: &S,
    coalescer: &std::cell::RefCell<StreamDeltaCoalescer>,
    delta_turn_id: &str,
    session_id: Option<String>,
    followup_step: u32,
    now_ms: u64,
) {
    let tail_batch = coalescer.borrow_mut().flush(now_ms);
    if let Some(batch) = tail_batch {
        for frame in batch.into_frames().into_iter().flatten() {
            emit_stream_event_with_step(
                sink,
                "turn:delta",
                delta_turn_id.to_string(),
                "delta",
                Some("calling_model"),
                frame.text,
                frame.reasoning,
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
                frame.latency_ms,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                session_id.clone(),
                Some(followup_step),
            );
        }
    }
}


/// 工具参数别名归一（P1-2，真实 trace 实证 9 次无谓失败）。
///
/// 背景：job 系列工具的 JSON Schema 声明 camelCase（`jobId` / `timeoutMs`），而 Rust 侧
/// serde 结构体字段是 snake_case。模型两种拼法都试过，于是分别被 schema 预检
/// （`unexpected argument job_id`）与 serde 解析（`missing field job_id`）拒绝，来回振荡。
///
/// 修法：进入 schema 预检之前，把 snake_case 别名改写成 schema 口径；仅在目标键**缺席**时
/// 改写（已按 schema 写的一律不动）。serde 侧另有 alias 兜底，host/direct 调用同样吃得住。
pub(super) fn canonicalize_tool_argument_aliases(tool_call: &mut ToolCall) {
    use crate::agent::tools::{TOOL_JOB_KILL, TOOL_JOB_OUTPUT, TOOL_JOB_START};
    const ALIASES: &[(&str, &[(&str, &str)])] = &[
        (
            TOOL_JOB_OUTPUT,
            &[("job_id", "jobId"), ("timeout_ms", "timeoutMs")],
        ),
        (TOOL_JOB_KILL, &[("job_id", "jobId")]),
        (TOOL_JOB_START, &[("timeout_ms", "timeoutMs")]),
    ];
    let Some(entries) = ALIASES
        .iter()
        .find(|(tool, _)| tool_call.name.as_str() == *tool)
        .map(|(_, entries)| *entries)
    else {
        return;
    };
    let Some(arguments) = tool_call.arguments.as_object_mut() else {
        return;
    };
    for (snake, camel) in entries {
        if arguments.contains_key(*camel) {
            continue;
        }
        if let Some(value) = arguments.remove(*snake) {
            arguments.insert((*camel).to_string(), value);
        }
    }
}

pub(super) fn normalize_tool_directive(
    mut tool_call: ToolCall,
    assistant_message: Option<Value>,
    output_text: &str,
    reasoning_content: Option<&str>,
    reasoning_content_value: Option<&Value>,
) -> Result<NormalizedToolDirective, String> {
    canonicalize_tool_argument_aliases(&mut tool_call);
    if !tool_call.name.trim().is_empty() {
        return Ok(NormalizedToolDirective {
            tool_call: Some(tool_call),
            assistant_message,
            output_text: output_text.to_string(),
        });
    }

    // 诊断：name 为空时，将完整的 assistant_message 原始数据和 tool_call 字段全部打印出来，
    // 用于排查 provider 返回的原始响应结构。
    if let Some(ref raw_msg) = assistant_message {
        runtime_log(format!(
            "turn:tool-call-empty-name call_id={:?} arguments={} raw_assistant_message={}",
            tool_call.call_id,
            tool_call.arguments,
            preview_text(&raw_msg.to_string(), 800),
        ));
    } else {
        runtime_log(format!(
            "turn:tool-call-empty-name call_id={:?} arguments={} raw_assistant_message=none",
            tool_call.call_id, tool_call.arguments,
        ));
    }

    // 兜底：name 为空且 arguments 也是空对象（上游空洞 tool call，如
    // PonyLlm/fledge-alpha-free 回 name="" arguments={}）时，直接丢弃该调用、
    // 本轮按纯文本继续，而不是整轮 failed。非空参数仍走原有修复/报错路径。
    if is_empty_tool_arguments(&tool_call.arguments) {
        runtime_log(format!(
            "turn:tool-call-empty-dropped call_id={:?} output_chars={}",
            tool_call.call_id,
            output_text.chars().count(),
        ));
        let dropped_message = match reasoning_content_value {
            Some(raw_reasoning) => provider_native_assistant_message_with_reasoning_value(
                &fallback_output_text(output_text),
                Some(raw_reasoning),
            ),
            None => provider_native_assistant_message_with_reasoning(
                &fallback_output_text(output_text),
                reasoning_content,
            ),
        };
        return Ok(NormalizedToolDirective {
            tool_call: None,
            assistant_message: Some(dropped_message),
            output_text: fallback_output_text(output_text),
        });
    }

    let repaired_name = infer_tool_name_from_arguments(&tool_call.arguments).ok_or_else(|| {
        format!(
            "provider 返回了缺少工具名的 tool call，且当前无法根据参数自动修复；arguments={}",
            preview_text(&tool_call.arguments.to_string(), 200)
        )
    })?;
    runtime_log(format!(
        "turn:tool-call-repaired repaired_name={} args={}",
        repaired_name, tool_call.arguments
    ));
    tool_call.name = repaired_name;

    let rebuilt_message = match reasoning_content_value {
        Some(raw_reasoning) => provider_native_assistant_tool_call_message_with_reasoning_value(
            non_empty_text(output_text),
            Some(raw_reasoning),
            &tool_call,
        ),
        None => provider_native_assistant_tool_call_message(
            non_empty_text(output_text),
            reasoning_content,
            &tool_call,
        ),
    };

    Ok(NormalizedToolDirective {
        assistant_message: Some(rebuilt_message),
        tool_call: Some(tool_call),
        output_text: output_text.to_string(),
    })
}

pub(super) fn is_empty_tool_arguments(arguments: &Value) -> bool {
    match arguments {
        Value::Null => true,
        Value::Object(map) => map.is_empty(),
        Value::String(raw) => {
            let trimmed = raw.trim();
            trimmed.is_empty() || trimmed == "{}" || trimmed == "null"
        }
        _ => false,
    }
}

fn fallback_output_text(output_text: &str) -> String {
    if output_text.trim().is_empty() {
        "已忽略上游返回的空工具调用（缺少工具名且无参数），请基于当前上下文继续作答或重试工具调用。".to_string()
    } else {
        output_text.to_string()
    }
}

pub(super) fn infer_tool_name_from_arguments(arguments: &Value) -> Option<String> {
    let object = arguments.as_object()?;

    // Plan 类型：参数包含 calls 数组（用于批量并行执行子调用）
    if object
        .get("calls")
        .and_then(Value::as_array)
        .map_or(false, |calls| !calls.is_empty())
    {
        return Some("BatchExecute".to_string());
    }

    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_query = object.contains_key("query");
    let has_limit = object.contains_key("limit");
    let has_line_count = object.contains_key("lineCount");
    let has_start_line = object.contains_key("startLine");

    if object
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Some("echo_input".to_string());
    }

    let path = path?;
    // PA-100：gather 信号（query/lineCount）优先于 startLine 判定——F1 之后
    // `{path, query, startLine}` 是合法的 gather 搜索+翻页组合；若仍先命中
    // startLine→segment，segment 的 schema 无 query 且 additionalProperties:false，
    // 会换一个门复现 invalid_arguments。纯 `{path, startLine}` 分页仍路由到 segment。
    if has_query || has_line_count {
        return Some("workspace_gather_context".to_string());
    }
    if has_start_line {
        return Some("workspace_read_file_segment".to_string());
    }
    if has_limit {
        if looks_like_file_path(path) {
            return Some("workspace_read_file".to_string());
        }
        return Some("workspace_list_files".to_string());
    }
    if looks_like_file_path(path) {
        return Some("workspace_read_file".to_string());
    }

    Some("workspace_path_info".to_string())
}

pub(super) fn looks_like_file_path(path: &str) -> bool {
    Path::new(path).extension().is_some()
}

pub(super) fn non_empty_text(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

pub(super) fn emit_lightweight_delta(
    sink: &impl TurnEventSink,
    turn_id: &str,
    text: Option<String>,
    reasoning_content: Option<String>,
    first_token_latency_ms: Option<u64>,
    session_id: Option<String>,
) {
    emit_stream_event(
        sink,
        "turn:delta",
        turn_id.to_string(),
        "delta",
        Some("calling_model"),
        text,
        reasoning_content,
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
        first_token_latency_ms,
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

pub(super) fn build_provider_call_cache_record(
    request_kind: ProviderRequestKind,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    protocol_label: Option<&str>,
    token_usage: Option<&TokenUsage>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    latency_kind: ProviderLatencyKind,
    build_context_observation: Option<&BuildContextObservation>,
) -> ProviderCallCacheRecord {
    let (input_tokens, cache_hit_input_tokens, reasoning_tokens, output_tokens, total_tokens) =
        token_usage_parts(token_usage);
    let cache_hit_source = token_usage.and_then(|usage| usage.cache_hit_source.clone());
    if cache_hit_input_tokens.is_some() && cache_hit_source.is_none() {
        panic!(
            "cache hit tokens require provider usage source; request_kind={:?} provider_source={:?} provider_mode={:?} cache_hit_input_tokens={:?}",
            request_kind, provider_source, provider_mode, cache_hit_input_tokens
        );
    }

    ProviderCallCacheRecord {
        request_kind,
        provider_source: provider_source.map(str::to_string),
        provider_mode: provider_mode.map(str::to_string),
        input_tokens,
        cache_hit_input_tokens,
        cache_hit_source,
        cache_miss_input_tokens: derive_cache_miss_input_tokens(token_usage),
        reasoning_tokens,
        output_tokens,
        total_tokens,
        first_token_latency_ms,
        turn_duration_ms,
        // wave-54 C3：usage 来源随记录透传（`None` ⇒ 未知，不得冒充真值）。
        usage_origin: token_usage.and_then(|usage| usage.origin).map(str::to_string),
        // wave-54 C4：上下文输入按协议归一（anthropic ⇒ input + cache_read）。
        context_input_tokens: context_input_tokens_for(protocol_label, token_usage),
        latency_kind,
        prefix_mutation_reasons: build_context_observation
            .map(|observation| observation.prefix_mutation_reasons.clone())
            .unwrap_or_default(),
    }
}

/// wave-54 C4：上下文输入口径（协议归一）。
///
/// `protocol_label` 以 `anthropic` 开头 ⇒ `input + cache_read`（anthropic 的 `input`
/// 不含缓存命中）；其余协议 ⇒ `input`（上游已含缓存命中）。缺 input ⇒ `None`
/// （UI 回退到上下文解析路径，禁止 0 占位）。
pub(crate) fn context_input_tokens_for(
    protocol_label: Option<&str>,
    token_usage: Option<&TokenUsage>,
) -> Option<u64> {
    let usage = token_usage?;
    let input_tokens = usage.input_tokens?;
    let anthropic = protocol_label
        .map(|label| label.starts_with("anthropic"))
        .unwrap_or(false);
    if anthropic {
        // anthropic：`input` 既不含 cache_read 也不含 cache_creation ⇒ 三者相加。
        Some(
            input_tokens
                .saturating_add(usage.cache_hit_input_tokens.unwrap_or(0))
                .saturating_add(usage.cache_creation_input_tokens.unwrap_or(0)),
        )
    } else {
        // 其余协议：`input` 已含缓存命中（semconv 口径）。
        Some(input_tokens)
    }
}

/// wave-54 C3：合并累积 usage 的来源标记——`provider` 真值优先于估算值；
/// 两者一致则保持，均缺失 ⇒ `None`（未知）。
pub(super) fn merge_usage_origin(
    left: Option<&'static str>,
    right: Option<&'static str>,
) -> Option<&'static str> {
    use crate::agent::provider::{USAGE_ORIGIN_ESTIMATED_CHARS, USAGE_ORIGIN_PROVIDER};
    // wave-54 C3（L3 收口）：与 `TurnTraceRecord::derive_usage_provenance_from_calls`
    // 及前端同一口径 —— **任一** `estimated_chars` ⇒ 合并结果 `estimated_chars`
    // （只要掺了估算值，整轮数字就必须可辨识）；全 `provider` ⇒ `provider`；
    // 其余（含未知 `None`）⇒ `None`，不臆造。
    let any_estimated = matches!(left, Some(USAGE_ORIGIN_ESTIMATED_CHARS))
        || matches!(right, Some(USAGE_ORIGIN_ESTIMATED_CHARS));
    if any_estimated {
        return Some(USAGE_ORIGIN_ESTIMATED_CHARS);
    }
    let all_provider = (left.is_none() || left == Some(USAGE_ORIGIN_PROVIDER))
        && (right.is_none() || right == Some(USAGE_ORIGIN_PROVIDER))
        && (left == Some(USAGE_ORIGIN_PROVIDER) || right == Some(USAGE_ORIGIN_PROVIDER));
    if all_provider {
        Some(USAGE_ORIGIN_PROVIDER)
    } else {
        None
    }
}

pub(super) fn derive_cache_miss_input_tokens(token_usage: Option<&TokenUsage>) -> Option<u64> {
    let usage = token_usage?;
    let input_tokens = usage.input_tokens?;
    let cache_hit_input_tokens = usage.cache_hit_input_tokens?;
    Some(input_tokens.saturating_sub(cache_hit_input_tokens))
}

/// 把一次 follow-up 响应的 usage / fallback 归并进轮级累加器。
///
/// 与内联的 `merge_token_usage` + `merge_fallback_reason` 两段**逐字等价**
/// （同一顺序、同一借用形态），供 `turn_sync` 的多条恢复分支共用。
pub(super) fn merge_response_usage_into_accumulators(
    accumulated_usage: Option<TokenUsage>,
    accumulated_fallback_reason: Option<String>,
    response: &ProviderResponse,
) -> (Option<TokenUsage>, Option<String>) {
    let usage = merge_token_usage(accumulated_usage, response.token_usage.as_ref());
    let fallback_reason =
        merge_fallback_reason(accumulated_fallback_reason, response.fallback_reason.clone());
    (usage, fallback_reason)
}

/// 同步（buffered）follow-up 的 per-call 记录装配并追加进 `records`。
///
/// 与内联 `records.push(build_provider_call_cache_record(...))` **逐字等价**：
/// 固定 `ProviderRequestKind::ToolFollowup` / `ProviderLatencyKind::BufferedResponse`，
/// 无首包观测（`first_token_latency_ms = None`）。
pub(super) fn push_buffered_followup_call_record(
    records: &mut Vec<ProviderCallCacheRecord>,
    provider: &ProviderManager,
    response: &ProviderResponse,
    duration_ms: u64,
    context_observation: &BuildContextObservation,
) {
    records.push(build_provider_call_cache_record(
        ProviderRequestKind::ToolFollowup,
        Some(response.provider_source.as_str()),
        Some(response.provider_mode.as_str()),
        // wave-54 C4：协议归一上下文输入口径（anthropic ⇒ input + cache_read）。
        Some(provider.protocol_label()),
        response.token_usage.as_ref(),
        None,
        Some(duration_ms),
        ProviderLatencyKind::BufferedResponse,
        Some(context_observation),
    ));
}

pub(super) fn merge_token_usage(
    existing: Option<TokenUsage>,
    next: Option<&TokenUsage>,
) -> Option<TokenUsage> {
    match (existing, next) {
        (Some(existing), Some(next)) => Some(TokenUsage {
            input_tokens: add_optional_u64(existing.input_tokens, next.input_tokens),
            cache_hit_input_tokens: add_optional_u64(
                existing.cache_hit_input_tokens,
                next.cache_hit_input_tokens,
            ),
            cache_hit_source: merge_cache_hit_source(
                existing.cache_hit_source,
                next.cache_hit_source.clone(),
            ),
            reasoning_tokens: add_optional_u64(existing.reasoning_tokens, next.reasoning_tokens),
            output_tokens: add_optional_u64(existing.output_tokens, next.output_tokens),
            total_tokens: add_optional_u64(existing.total_tokens, next.total_tokens),
            origin: merge_usage_origin(existing.origin, next.origin),
            cache_creation_input_tokens: add_optional_u64(
                existing.cache_creation_input_tokens,
                next.cache_creation_input_tokens,
            ),
        }),
        (Some(existing), None) => Some(existing),
        (None, Some(next)) => Some(next.clone()),
        (None, None) => None,
    }
}

pub(super) fn merge_cache_hit_source(
    left: Option<String>,
    right: Option<String>,
) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) if left == right => Some(left),
        (Some(left), Some(right)) => Some(format!("{}+{}", left, right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(super) fn add_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(super) fn running_tool_activities_with_history(
    completed: &[TurnToolActivity],
    running: Vec<TurnToolActivity>,
) -> Vec<TurnToolActivity> {
    let mut combined = completed.to_vec();
    combined.extend(running);
    combined
}

#[derive(Default)]
pub(super) struct StreamReasoningBatcher {
    pub(super) buffer: String,
}

pub(super) fn canonicalize_tool_argument_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(canonicalize_tool_argument_value)
                .collect::<Vec<_>>(),
        ),
        Value::Object(map) => {
            let mut normalized = Map::new();
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                // `description` 是系统提示要求模型在每次工具调用时附上的自由文本说明，
                // 措辞每次可能不同；它不属于任何工具 schema 的正式参数，若参与签名，
                // 模型只要换一种措辞就能绕过同一 turn 内的重复调用检测，导致重复检索。
                if key == "description" {
                    continue;
                }
                if let Some(entry) = map.get(&key) {
                    normalized.insert(key, canonicalize_tool_argument_value(entry));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

/// Run 类工具名（契约 P0-1）：只有这些工具的 `timeout` 失败才做指纹归一化。
fn is_run_class_tool_name(name: &str) -> bool {
    matches!(
        name,
        "Run" | "workspace_run_command" | "workspace.run_command"
    )
}

/// Parse a leading `NAME=value` assignment, returning the remainder after the value.
/// `NAME` must be a POSIX shell identifier; the value may be bare or double-quoted.
fn strip_inline_env_assignment(input: &str) -> Option<&str> {
    let eq = input.find('=')?;
    let name = &input[..eq];
    if name.is_empty()
        || !name.chars().enumerate().all(|(index, ch)| {
            ch == '_' || ch.is_ascii_alphabetic() || (index > 0 && ch.is_ascii_digit())
        })
    {
        return None;
    }
    let after_eq = &input[eq + 1..];
    if let Some(quoted) = after_eq.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some(&quoted[end + 1..]);
    }
    let end = after_eq
        .find(|ch: char| ch.is_whitespace() || ch == '&')
        .unwrap_or(after_eq.len());
    if end == 0 {
        return None;
    }
    Some(&after_eq[end..])
}

/// Strip one leading inline environment-assignment prefix: POSIX `NAME=value ` or Windows
/// `set NAME=value&&` / `set NAME=value &&` (case-insensitive). Returns the remainder.
fn strip_one_inline_env_prefix(input: &str) -> Option<&str> {
    if input
        .get(..4)
        .map_or(false, |head| head.eq_ignore_ascii_case("set "))
    {
        if let Some(after_value) = strip_inline_env_assignment(&input[4..]) {
            if let Some(rest) = after_value.trim_start().strip_prefix("&&") {
                let rest = rest.trim_start();
                if !rest.is_empty() {
                    return Some(rest);
                }
            }
        }
    }
    let after_value = strip_inline_env_assignment(input)?;
    let rest = after_value.strip_prefix(|ch: char| ch.is_whitespace())?;
    if rest.trim().is_empty() {
        return None;
    }
    Some(rest)
}

/// Repeatedly strip leading inline environment-assignment prefixes (契约 P0-1 归一化规则 b),
/// so that retries of the same core command share one fingerprint. Returns the trimmed core
/// command; it may be empty when the whole command consists of prefixes.
fn strip_leading_inline_env_prefixes(command: &str) -> &str {
    let mut rest = command.trim();
    while let Some(next) = strip_one_inline_env_prefix(rest) {
        rest = next.trim();
    }
    rest
}

/// Run 类工具 `timeout` 失败的参数指纹：删除 `timeoutMs`，并把 `command` 归一化为核心命令。
fn normalize_run_timeout_argument_value(value: &Value) -> Value {
    let mut normalized = canonicalize_tool_argument_value(value);
    if let Value::Object(map) = &mut normalized {
        // 契约 v8（B015）：`timeoutMs` 不再整体删除，而是按 2 的幂**分桶**后写回同名键。
        //
        // 语义：延长超时 = 换新策略（落进更大的桶 ⇒ 指纹改变），因此「同一命令 + 明显更长的超时」
        // 不会再被既有 timeout 指纹吞掉；但同一量级内的抖动（120s vs 121s）仍归一到同一桶，
        // 避免模型靠微调毫秒数绕过指纹。真正的防重试风暴由「同工具连续失败 4 次」闸兜底
        // ——那道闸**不看参数**，故分桶不会放宽它。
        //
        // bucket = floor(log2(ms))，写回 `1u64 << bucket`；`timeoutMs` 缺失/非 u64 时与既有行为
        // 一致：不写该键（归化结果里不出现）。
        match value.get("timeoutMs").and_then(Value::as_u64) {
            Some(ms) => {
                // `ilog2(0)` 未定义：0 与 1 同归 0 号桶（1ms），避免下溢 panic
                // （真实生效值由 `resolve_run_timeout_ms` 夹紧到 ≥1）。
                let bucket = if ms == 0 { 0 } else { ms.ilog2() };
                map.insert("timeoutMs".to_string(), Value::from(1u64 << bucket));
            }
            None => {
                map.remove("timeoutMs");
            }
        }
        if let Some(Value::String(command)) = map.get("command") {
            let stripped = strip_leading_inline_env_prefixes(command);
            if !stripped.is_empty() {
                map.insert("command".to_string(), Value::String(stripped.to_string()));
            }
        }
    }
    normalized
}

pub(super) fn tool_call_signature(tool_call: &ToolCall) -> String {
    let normalized = canonicalize_tool_argument_value(&tool_call.arguments);
    format!(
        "{}:{}",
        tool_call.name,
        serde_json::to_string(&normalized).unwrap_or_else(|_| "{}".to_string())
    )
}

/// Detect a `control_outcome_pending` legacy tool result: the governed executor surfaces a
/// pending control outcome as `status == "error"` with a structured `error.code ==
/// "control_outcome_pending"` (design.md Decision 5). Such a result is never provider-consumable
/// and must pause the run instead of being fed back as an ordinary tool error.
pub(super) fn tool_result_control_outcome_pending(tool_result: &ToolResult) -> bool {
    if tool_result.status != "error" {
        return false;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(&tool_result.output) else {
        return false;
    };
    parsed
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        == Some("control_outcome_pending")
}

/// Extract the consecutive-failure signal for a tool result:
/// `(tool name, argument fingerprint, error code)`.
/// `ok`/`partial` results, aborted executions, and pending control outcomes (`Ask` waits) never
/// count as failures, so they reset the consecutive counter.
/// Using the argument fingerprint ensures that trying different arguments (e.g. `pnpm test` then `pnpm build`)
/// does not trigger a false-positive consecutive failure loop break.
pub fn tool_failure_signal(
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Option<(String, String, String)> {
    if tool_result_control_outcome_pending(tool_result) {
        return None;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(&tool_result.output) else {
        return None;
    };
    // 契约 B010（CT-B010-1）：Run 到期 promote 成后台作业后，结果是 `status="ok"` 且**没有**
    // `error.code=timeout` —— 若不在这里补一条 timeout 类信号，连续失败熔断、超时指纹归一、
    // 非幂等盲重试防护会同时失去输入。判定口径只看载荷根级事实：`timedOut == true`。
    // 指纹与 error 路径**共用同一归一化面**（Run 类工具去掉/归一 timeoutMs），保证
    // 「同命令同超时稳定、不同命令可区分」。
    if tool_result.status == "ok" {
        if parsed.get("timedOut").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let normalized_args = if is_run_class_tool_name(&tool_call.name) {
            normalize_run_timeout_argument_value(&tool_call.arguments)
        } else {
            canonicalize_tool_argument_value(&tool_call.arguments)
        };
        return Some((
            tool_call.name.clone(),
            normalized_args.to_string(),
            "timeout".to_string(),
        ));
    }
    if tool_result.status != "error" {
        return None;
    }
    let error = parsed.get("error")?;
    let code = error
        .get("kind")
        .or_else(|| error.get("code"))
        .and_then(Value::as_str)?;
    let normalized_args = if code == "timeout" && is_run_class_tool_name(&tool_call.name) {
        normalize_run_timeout_argument_value(&tool_call.arguments)
    } else {
        canonicalize_tool_argument_value(&tool_call.arguments)
    };
    let arg_fingerprint = normalized_args.to_string();
    Some((tool_call.name.clone(), arg_fingerprint, code.to_string()))
}

/// Tracks how many consecutive failures share the same `(tool name, argument fingerprint, error code)` signal.
/// A success, an unclassifiable result, or a different signal resets the run.
#[derive(Clone, Debug, Default)]
pub(super) struct ConsecutiveFailureTracker {
    pub(super) signal: Option<(String, String, String)>,
    pub(super) count: usize,
    /// 工具级连续失败：只看工具名，忽略入参与错误码（P1-3 第二道闸）。
    pub(super) tool_name: Option<String>,
    pub(super) tool_count: usize,
}

/// 同一 turn 内的「同签名执行次数」计数器（契约矩阵 C2）：把既有的 `BTreeSet` 判重升级为
/// 计数判重，使同一工具 + 同一归一化参数可以**有限次**执行，而不是第 2 次即被拒。
///
/// 关键不变量：
/// - **闸点只读、执行点才记账**：`is_exhausted` / `occurrence_count` 纯读，`record_executed`
///   是唯一记账点；被闸拦下的调用不得记账，否则恢复式收尾后的重试会被自己推高计数。
/// - **不预置首批计数**：turn 起始一律用 `new()` 建空计数。若按首批调用预置（note 最初的
///   `from_batch`），预置 1 次 + 执行点再记 1 次构成双重计数，默认 limit=2 时第 2 次即熔断。
/// - **写类成功只清空其它签名计数、保留自身**：`record_executed(.., changed_world = true)` 时，
///   若该工具确实改变了世界，则证明模型已推进任务，其它签名的历史计数不再代表打转；自身计数
///   必须保留，否则「同一读法反复写同一处」会被自己清空而无限放行。
#[derive(Default, Clone, Debug)]
pub(super) struct SameSignatureTracker {
    pub(super) counts: BTreeMap<String, usize>,
}

impl SameSignatureTracker {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// 只读：该签名在本 turn 内已执行成功放行的次数。
    pub(super) fn occurrence_count(&self, signature: &str) -> usize {
        self.counts.get(signature).copied().unwrap_or(0)
    }

    /// 只读闸点：`count >= limit` 即视为耗尽（不记账、不改状态）。
    pub(super) fn is_exhausted(&self, signature: &str, limit: usize) -> bool {
        self.occurrence_count(signature) >= limit
    }

    /// 唯一记账点：仅在工具调用真正被放行执行时调用。
    /// `changed_world` = 本次执行是否改变了世界（见 `tool_execution_changed_world`）。
    pub(super) fn record_executed(&mut self, signature: String, changed_world: bool) {
        // 写类成功 ⇒ 任务已被推进，其它签名的历史计数不再代表「打转」，整体清空；
        // 但**保留自身**（`retain` 只留 `signature`），否则「反复写同一处」会自我清空而无限放行。
        if changed_world {
            self.counts.retain(|key, _| key == &signature);
        }
        *self.counts.entry(signature).or_insert(0) += 1;
    }
}

/// 判定一次工具执行是否「改变了世界」（写类成功清空其它签名计数的输入事实）。
///
/// 语义（AMEND-2）：`tool_result.status ∉ {"error", "aborted", "cancelled"}` **且**
/// `tool_kind_for_name(&tool_call.name) == ToolKind::Write`。失败/中止/取消、只读/搜索/执行类
/// 一律视为未改变世界（不得据此清空别人的计数）。
///
/// 为什么成功判据是**负向列表**而不是 `status != "error"`：`aborted`（用户中止）与 `cancelled`
/// 的写类调用并没有落地任何变更，却会被 `!= "error"` 误判为成功并清空他人计数 —— 等于把
/// 「被中止的写」当成任务推进。
///
/// 为什么**只认 `Write`、移除 `Execute`**（Gate-1 BLOCKING）：`Execute` 覆盖
/// `workspace_run_command` 与全部 `Job*`（`JobOutput`/`JobList`/`JobKill`，
/// `crates/pony-agent-core/src/agent/tools.rs:7980`），静态分类无法区分「写域 Run」「纯查询（如
/// `ls`）」「job 轮询」。若 Execute 也清空，`JobOutput(A)` 与 `Run(X)` 的成功**交替**会互相清空，
/// 两签名计数恒为 1、永不达上限 —— 旧 `BTreeSet`（第 2 次即拦）反而更严，本闸就成了新的绕闸面。
/// 且 `retain` 恒保留自身，Execute 清空对「同参重试」**零收益**；「改完重跑同一条命令」的真实
/// 受益路径是中间的 `Edit`（Write）触发清空，不依赖 `Run` 自身清空。
pub(super) fn tool_execution_changed_world(tool_call: &ToolCall, tool_result: &ToolResult) -> bool {
    use crate::agent::tools::{tool_kind_for_name, ToolKind};

    if matches!(
        tool_result.status.as_str(),
        "error" | "aborted" | "cancelled"
    ) {
        return false;
    }
    matches!(tool_kind_for_name(&tool_call.name), ToolKind::Write)
}

/// Match the dispatcher's persisted `PendingControlRequest` to the originating tool call. The
/// request's `call_id` is the dispatch call id the executor persisted, which equals the original
/// assistant tool-call id when one was supplied; a missing call id falls back to the most recent
/// pending request, **Approval and Interaction alike**.
///
/// 契约 C6（W1）：旧实现无 `call_id` 时只回退匹配 `Interaction`（Ask 路径），需要审批的工具调用
/// 在恢复回合时永远拿不到待审批请求（`pending_request=None` ⇒ 用户看到一条无法恢复的挂起）。
/// 现在回退集合 = 所有 `Pending` 状态的 `Approval` / `Interaction`，多条时取“最新”一条：
/// `expires_at_ms` 最大者（= 最晚登记/最晚过期），仍并列则取数组中靠后的一条。
pub(super) fn match_pending_control_request<'a>(
    requests: &'a [PendingControlRequest],
    tool_call: &ToolCall,
) -> Option<&'a PendingControlRequest> {
    if let Some(call_id) = tool_call.call_id.as_deref() {
        if let Some(found) = requests.iter().find(|request| {
            request.state == PendingControlRequestState::Pending && request.call_id == call_id
        }) {
            return Some(found);
        }
    }
    requests
        .iter()
        .filter(|request| {
            request.state == PendingControlRequestState::Pending
                && matches!(
                    request.request_kind,
                    PendingControlRequestKind::Approval | PendingControlRequestKind::Interaction
                )
        })
        // `max_by_key` 在并列时返回**最后**一个最大值 ⇒ 同 `expires_at_ms` 时按数组顺序取最新一条。
        .max_by_key(|request| request.expires_at_ms)
}

impl StreamReasoningBatcher {
    pub(super) fn push(&mut self, reasoning: String) -> Option<String> {
        self.buffer.push_str(&reasoning);
        if self.buffer.chars().count() >= STREAM_REASONING_BATCH_CHARS {
            return self.flush();
        }
        None
    }

    pub(super) fn flush(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.buffer))
    }
}

impl ConsecutiveFailureTracker {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Record one tool outcome signal; returns the updated consecutive-failure count.
    ///
    /// 同时维护「同工具连续失败」计数（`same_tool_count`）：既有指纹闸要求
    /// `(工具, 入参指纹, 错误码)` 三元组完全一致，模型换参数/换错误码即可绕过；工具级
    /// 计数只看工具名，用于拦住「反复锤同一工具」的卡死循环。
    pub(super) fn record(&mut self, signal: Option<(String, String, String)>) -> usize {
        match signal {
            Some(next) => {
                if self.tool_name.as_deref() == Some(next.0.as_str()) {
                    self.tool_count += 1;
                } else {
                    self.tool_name = Some(next.0.clone());
                    self.tool_count = 1;
                }
                if self.signal.as_ref() == Some(&next) {
                    self.count += 1;
                    self.count
                } else {
                    self.signal = Some(next);
                    self.count = 1;
                    1
                }
            }
            None => {
                self.signal = None;
                self.count = 0;
                self.tool_name = None;
                self.tool_count = 0;
                0
            }
        }
    }

    /// 同一工具连续失败次数（成功或换工具即归零）。
    pub(super) fn same_tool_count(&self) -> usize {
        self.tool_count
    }
}

// ── F3：非阻塞 delta 聚合（原 `turn_stream.rs`，按读线程聚合语义归位本模块） ──

// ─────────────────── F3：非阻塞 delta 聚合（SSE 读线程零阻塞） ───────────────────
//
// 取证：`on_delta` 在**阻塞 SSE 读循环内联执行**——`provider::openai_sse::push_payload`
// 每解析到一个 chunk 就同步调 `on_delta`（`openai_sse.rs:133/139/149`）。旧实现曾在 6 处
// `on_delta` 里做「阻塞睡眠到 16ms 间隔」的客户端节流（`turn_stream.rs` 的
// followup/initial 两条流 + `tool_recovery.rs` 的恢复流，均已删除）：睡眠期间停止读
// socket ⇒ TCP 背压 ⇒ 突发路由（1 token/chunk，间隔中位 0ms）下 token 率被硬顶在 ~62.5/s。
//
// 现在改为**非阻塞时间窗聚合**：窗口内的 text/reasoning 合并进 pending，窗口外的
// 第一个 delta 把 pending 一次性上屏。读线程每处理一个 delta 只做一次字符串拼接 +
// 一次时间戳比较，耗时**与客户端消费速度无关**（I3）；内容只合并、不丢弃，收尾
// 必须调 `flush()`（I2：禁止丢最终帧；终态派发本就不经此路径）。
//
// `sink` 契约（F3 修订 v2，Lead 裁决）：`TurnEventSink::emit` 必须是**非阻塞投递**
// （Tauri `emit` fire-and-forget / TUI 无界 channel / headless 无消费者）。
// 已知限制：`emit` 仍**内联**在读循环里，若将来出现阻塞式 sink（例如有界 channel
// 满后等待），慢消费者会重新串进读线程——届时必须在 sink 实现侧解决，而不是在本
// 聚合器里加等待（本聚合器**永不**阻塞读线程）。

/// 聚合后的单帧内容（`None` 字段表示本次没有该内容，禁止用空串冒充）。
pub(super) struct CoalescedFrame {
    pub(super) text: Option<String>,
    pub(super) reasoning: Option<String>,
    pub(super) latency_ms: Option<u64>,
}

/// 一个节流窗口里攒下的内容（reasoning 与 text 各至多一段）。
///
/// 为什么分两槽而不是揉成一帧：既有语义要求「思考帧」与「正文帧」的载荷形状分开
/// （旧实现也是先 flush 思考、再 flush 正文，两帧同刻发出、中间不睡眠）。合并只发生在
/// **同类型**之内，帧形状与顺序因此与旧实现逐字一致。
pub(super) struct CoalescedBatch {
    reasoning: Option<String>,
    text: Option<String>,
    latency_ms: Option<u64>,
}

impl CoalescedBatch {
    /// 拆成至多两帧：**先 reasoning 后 text**（与旧实现「先 flush 思考再 flush 正文」同序）。
    /// 两帧携带同一个首字延迟值（旧实现同样把同一个 `latency` 传给两次 flush）。
    pub(super) fn into_frames(self) -> [Option<CoalescedFrame>; 2] {
        let latency_ms = self.latency_ms;
        [
            self.reasoning.map(|reasoning| CoalescedFrame {
                text: None,
                reasoning: Some(reasoning),
                latency_ms,
            }),
            self.text.map(|text| CoalescedFrame {
                text: Some(text),
                reasoning: None,
                latency_ms,
            }),
        ]
    }
}

/// F3：读线程侧的 delta 聚合器（禁止任何阻塞等待，契约见上方注释）。
pub(super) struct StreamDeltaCoalescer {
    window_ms: u64,
    /// 上一次真正上屏的时刻（毫秒，调用方提供的单调时钟）。
    last_emit_ms: Option<u64>,
    pending_text: String,
    pending_reasoning: String,
    pending_latency_ms: Option<u64>,
}

impl StreamDeltaCoalescer {
    /// 客户端节流窗口：沿用旧节流口径（16ms ⇒ 上屏帧率上限不变），
    /// 变的只是「窗口内不再阻塞读线程」。
    const CLIENT_WINDOW_MS: u64 = 16;

    pub(super) fn new() -> Self {
        Self {
            window_ms: Self::CLIENT_WINDOW_MS,
            last_emit_ms: None,
            pending_text: String::new(),
            pending_reasoning: String::new(),
            pending_latency_ms: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.pending_text.is_empty() && self.pending_reasoning.is_empty()
    }

    /// 入队一段增量（`text` / `reasoning` 至多一个非空）。**只拼接，绝不 sleep**。
    ///
    /// `latency_ms` 取**第一个**非空值：即「首个内容增量」的首字延迟，随第一个
    /// 真正上屏的帧一起下发（合并不会把首字延迟挪后或丢失）。
    pub(super) fn push(
        &mut self,
        text: Option<String>,
        reasoning: Option<String>,
        latency_ms: Option<u64>,
    ) {
        if let Some(text) = text {
            self.pending_text.push_str(&text);
        }
        if let Some(reasoning) = reasoning {
            self.pending_reasoning.push_str(&reasoning);
        }
        if self.pending_latency_ms.is_none() {
            self.pending_latency_ms = latency_ms;
        }
    }

    /// 到窗口 ⇒ 取走本窗口攒下的内容；仍在窗口内 ⇒ `None`（内容留在 pending，
    /// 等下一次 delta 或收尾 `flush`）。
    pub(super) fn take_if_due(&mut self, now_ms: u64) -> Option<CoalescedBatch> {
        if self.is_empty() {
            return None;
        }
        match self.last_emit_ms {
            Some(last) if now_ms.saturating_sub(last) < self.window_ms => None,
            _ => Some(self.take(now_ms)),
        }
    }

    /// 收尾 flush：**无视窗口**把 pending 全部发完（禁止丢最终帧）。
    pub(super) fn flush(&mut self, now_ms: u64) -> Option<CoalescedBatch> {
        if self.is_empty() {
            return None;
        }
        Some(self.take(now_ms))
    }

    fn take(&mut self, now_ms: u64) -> CoalescedBatch {
        self.last_emit_ms = Some(now_ms);
        let text = (!self.pending_text.is_empty()).then(|| std::mem::take(&mut self.pending_text));
        let reasoning = (!self.pending_reasoning.is_empty())
            .then(|| std::mem::take(&mut self.pending_reasoning));
        let latency_ms = self.pending_latency_ms.take();
        CoalescedBatch {
            reasoning,
            text,
            latency_ms,
        }
    }
}

#[cfg(test)]
mod b010_timeout_signal_tests {
    use super::*;
    use serde_json::json;

    fn run_call(command: &str, timeout_ms: u64) -> ToolCall {
        ToolCall {
            name: "workspace_run_command".to_string(),
            arguments: json!({
                "command": command,
                "cwd": "/repo",
                "timeoutMs": timeout_ms,
                "description": "b010",
            }),
            call_id: Some("call-b010".to_string()),
            plan: None,
        }
    }

    /// promote 成功载荷（status=ok + 根部 timedOut=true + promoted/jobId）。
    fn promoted_result(timeout_ms: u64) -> ToolResult {
        ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "ok".to_string(),
            duration_ms: 0,
            output: json!({
                "ok": true,
                "command": "pnpm test",
                "timeoutMs": timeout_ms,
                "timedOut": true,
                "promoted": true,
                "jobId": "job-b010",
                "exitCode": serde_json::Value::Null,
                "error": serde_json::Value::Null,
            })
            .to_string(),
        }
    }

    /// 波次一的超时错误载荷（status=error + error.code=timeout）。
    fn timeout_error_result(timeout_ms: u64) -> ToolResult {
        ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "error".to_string(),
            duration_ms: 0,
            output: json!({
                "ok": false,
                "command": "pnpm test",
                "timeoutMs": timeout_ms,
                "timedOut": true,
                "error": { "code": "timeout", "kind": "timeout", "message": "超时" },
            })
            .to_string(),
        }
    }

    /// 普通成功载荷（不得产生失败信号）。
    fn plain_ok_result() -> ToolResult {
        ToolResult {
            tool_name: "workspace_run_command".to_string(),
            status: "ok".to_string(),
            duration_ms: 0,
            output: json!({ "ok": true, "exitCode": 0, "timedOut": false }).to_string(),
        }
    }

    /// 契约 v8 / B015：`timeoutMs` 按 2 的幂分桶后参与指纹。
    ///
    /// 等价类：同桶 ⇒ 同指纹；跨桶 ⇒ 不同指纹；缺失 ⇒ 不写该键（与既有归一化一致）。
    #[test]
    fn timeout_ms_buckets_follow_power_of_two() {
        fn normalized_timeout(ms: u64) -> u64 {
            let normalized = normalize_run_timeout_argument_value(&json!({
                "command": "pnpm test",
                "timeoutMs": ms,
            }));
            normalized
                .get("timeoutMs")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_else(|| panic!("timeoutMs={ms} 归一化后必须写回同名键：{normalized}"))
        }

        let cases = [
            (120_000u64, 65_536u64, 16u32),
            (121_000, 65_536, 16),
            (300_000, 262_144, 18),
            (600_000, 524_288, 19),
            (60_000, 32_768, 15),
        ];
        for (requested, expected, bucket) in cases {
            let actual = normalized_timeout(requested);
            eprintln!(
                "[b015] timeoutMs={requested} ⇒ bucket={bucket} ⇒ 归一值={actual}（期望 {expected}）"
            );
            assert_eq!(actual, expected, "B015：timeoutMs={requested} 必须分桶到 {expected}");
            assert_eq!(
                bucket,
                if requested == 0 { 0 } else { requested.ilog2() },
                "B015：桶号必须是 floor(log2)"
            );
        }

        // 同桶等价、跨桶不等价（指纹由同一函数产生）。
        let same_bucket = normalize_run_timeout_argument_value(&json!({
            "command": "pnpm test", "timeoutMs": 120_000u64,
        }));
        let same_bucket_2 = normalize_run_timeout_argument_value(&json!({
            "command": "pnpm test", "timeoutMs": 121_000u64,
        }));
        let bigger_bucket = normalize_run_timeout_argument_value(&json!({
            "command": "pnpm test", "timeoutMs": 600_000u64,
        }));
        assert_eq!(same_bucket, same_bucket_2, "B015：同桶必须归一到同一指纹");
        assert_ne!(same_bucket, bigger_bucket, "B015：跨桶必须产生不同指纹");

        // 缺失/非 u64：不写该键（与既有归一化结果一致）。
        let missing = normalize_run_timeout_argument_value(&json!({ "command": "pnpm test" }));
        eprintln!("[b015] 缺失 timeoutMs ⇒ 归一化结果={missing}");
        assert!(
            missing.get("timeoutMs").is_none(),
            "B015：timeoutMs 缺失时不得凭空写入该键"
        );
        let non_integer =
            normalize_run_timeout_argument_value(&json!({ "command": "pnpm test", "timeoutMs": 1.5 }));
        assert!(
            non_integer.get("timeoutMs").is_none(),
            "B015：非 u64 的 timeoutMs 同样不写该键"
        );
    }

    /// 契约 B010：promote 载荷必须产出与 timeout 错误路径**完全一致**的三元组，
    /// 且指纹对命令敏感；普通成功载荷不得产出信号。
    #[test]
    fn promote_result_yields_same_timeout_signal_as_error_path() {
        let call = run_call("pnpm test", 120_000);
        let promote = tool_failure_signal(&call, &promoted_result(120_000));
        let error = tool_failure_signal(&call, &timeout_error_result(120_000));
        eprintln!("[b010] promote signal      = {promote:?}");
        eprintln!("[b010] timeout-error signal = {error:?}");
        assert_eq!(
            promote, error,
            "B010：promote 与 timeout 错误路径必须产出同一 (tool, fingerprint, code) 三元组"
        );

        let other_call = run_call("pnpm build", 120_000);
        let other = tool_failure_signal(&other_call, &promoted_result(120_000));
        eprintln!("[b010] other-command signal = {other:?}");
        let promote_fingerprint = promote.expect("promote 必须产生信号").1;
        let other_fingerprint = other.expect("promote 必须产生信号").1;
        assert_ne!(
            promote_fingerprint, other_fingerprint,
            "B010：不同 command 必须产生不同指纹"
        );

        assert!(
            tool_failure_signal(&call, &plain_ok_result()).is_none(),
            "B010：非 timeout 的成功载荷不得产生失败信号（不得放宽为「所有 ok 都算失败」）"
        );
    }
}

/// W1 RED（task-4）：`match_pending_control_request` 的证据。
///
/// 现状缺陷：当 `tool_call.call_id` 为 `None` 时，函数只回退匹配 `Interaction`（Ask 路径），
/// 永远不命中 `Approval`，导致需要审批的工具调用在恢复流式回合时拿不到待审批请求。
/// 契约 C6 要求：无 call_id 时必须能命中 Pending Approval，同时保留 Interaction 兜底。
#[cfg(test)]
mod w1_pending_control_match_tests {
    use super::*;
    use serde_json::json;

    fn pending_request(
        request_id: &str,
        request_kind: PendingControlRequestKind,
        call_id: &str,
        state: PendingControlRequestState,
    ) -> PendingControlRequest {
        PendingControlRequest {
            request_id: request_id.to_string(),
            request_kind,
            session_id: Some("session-w1".to_string()),
            run_id: Some("run-w1".to_string()),
            turn_id: "turn-w1".to_string(),
            call_id: call_id.to_string(),
            descriptor_snapshot_id: "snapshot-w1".to_string(),
            descriptor_id: "descriptor-w1".to_string(),
            final_args_digest: "digest-w1".to_string(),
            policy_digest: "policy-w1".to_string(),
            nonce: "nonce-w1".to_string(),
            version: 1,
            expires_at_ms: u64::MAX,
            state,
            prompt: Some("需要确认".to_string()),
            options: None,
            multi_select: None,
        }
    }

    fn approval(request_id: &str, call_id: &str) -> PendingControlRequest {
        pending_request(
            request_id,
            PendingControlRequestKind::Approval,
            call_id,
            PendingControlRequestState::Pending,
        )
    }

    fn interaction(request_id: &str, call_id: &str) -> PendingControlRequest {
        pending_request(
            request_id,
            PendingControlRequestKind::Interaction,
            call_id,
            PendingControlRequestState::Pending,
        )
    }

    fn tool_call(call_id: Option<&str>) -> ToolCall {
        ToolCall {
            call_id: call_id.map(str::to_string),
            name: "workspace_run_command".to_string(),
            arguments: json!({ "command": "pnpm test", "description": "w1" }),
            plan: None,
        }
    }

    fn observed(request: Option<&PendingControlRequest>) -> Option<(String, PendingControlRequestKind)> {
        request.map(|request| (request.request_id.clone(), request.request_kind.clone()))
    }

    /// 契约 C6 主断言：call_id=None 且存在 Pending Approval ⇒ 必须命中该 Approval。
    #[test]
    fn w1_missing_call_id_matches_pending_approval() {
        let requests = vec![approval("req-approval", "dispatch-call-1")];

        let matched = match_pending_control_request(&requests, &tool_call(None));

        assert!(
            matched.is_some(),
            "契约 C6：tool_call.call_id=None 且存在 Pending Approval 时必须返回 Some(该 Approval)，实际 = {:?}",
            observed(matched)
        );
        assert_eq!(
            matched.map(|request| request.request_id.as_str()),
            Some("req-approval"),
            "契约 C6：必须命中唯一的 Pending Approval"
        );
        assert_eq!(
            matched.map(|request| request.request_kind.clone()),
            Some(PendingControlRequestKind::Approval),
            "契约 C6：命中的请求类型必须是 Approval"
        );
    }

    /// 契约 C6 回归保护：Ask/Interaction 兜底不得被削弱。
    #[test]
    fn w1_missing_call_id_still_falls_back_to_pending_interaction() {
        let requests = vec![interaction("req-ask", "dispatch-call-2")];

        let matched = match_pending_control_request(&requests, &tool_call(None));

        assert_eq!(
            matched.map(|request| request.request_id.as_str()),
            Some("req-ask"),
            "契约 C6：无 Approval 时 Ask/Interaction 兜底必须保留，实际 = {:?}",
            observed(matched)
        );
    }

    /// 契约 C6 回归保护：有 call_id 时按 call_id 精确命中 Approval。
    #[test]
    fn w1_call_id_still_matches_approval_by_id() {
        let requests = vec![
            approval("req-other", "call-zzz"),
            approval("req-target", "call-abc"),
        ];

        let matched = match_pending_control_request(&requests, &tool_call(Some("call-abc")));

        assert_eq!(
            matched.map(|request| request.request_id.as_str()),
            Some("req-target"),
            "契约 C6：call_id 精确匹配必须优先，实际 = {:?}",
            observed(matched)
        );
    }

    /// 契约 C6 边界：非 Pending 的 Approval 不得被“无 call_id 回退”命中。
    #[test]
    fn w1_non_pending_approval_is_never_matched() {
        let requests = vec![pending_request(
            "req-consumed",
            PendingControlRequestKind::Approval,
            "dispatch-call-3",
            PendingControlRequestState::Consumed,
        )];

        let matched = match_pending_control_request(&requests, &tool_call(None));

        assert!(
            matched.is_none(),
            "契约 C6：state != Pending 的 Approval 不得命中，实际 = {:?}",
            observed(matched)
        );
    }

    /// 契约 C6 边界：Approval 与 Interaction 同时待定 ⇒ 必须命中其中之一（不得 None）。
    #[test]
    fn w1_mixed_pending_requests_hit_a_pending_one() {
        let requests = vec![
            approval("req-approval", "dispatch-call-4"),
            interaction("req-ask", "dispatch-call-5"),
        ];

        let matched = match_pending_control_request(&requests, &tool_call(None));

        let matched_id = matched.map(|request| request.request_id.as_str());
        assert!(
            matched_id == Some("req-approval") || matched_id == Some("req-ask"),
            "契约 C6：同时存在 Pending Approval / Interaction 时必须命中其中一个，实际 = {matched_id:?}"
        );
        assert_eq!(
            matched.map(|request| request.state.clone()),
            Some(PendingControlRequestState::Pending),
            "契约 C6：命中的请求必须是 Pending 状态"
        );
    }

    /// 契约 C6 边界：空列表始终返回 None。
    #[test]
    fn w1_empty_requests_yield_none() {
        assert!(
            match_pending_control_request(&[], &tool_call(None)).is_none(),
            "契约 C6：无待定请求时必须返回 None"
        );
    }
}
