use super::*;

pub(super) const STREAM_REASONING_BATCH_CHARS: usize = 96;

pub(super) fn normalize_tool_directive(
    mut tool_call: ToolCall,
    assistant_message: Option<Value>,
    output_text: &str,
    reasoning_content: Option<&str>,
    reasoning_content_value: Option<&Value>,
) -> Result<NormalizedToolDirective, String> {
    if !tool_call.name.trim().is_empty() {
        return Ok(NormalizedToolDirective {
            tool_call,
            assistant_message,
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
        tool_call,
    })
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
    if has_start_line {
        return Some("workspace_read_file_segment".to_string());
    }
    if has_query || has_line_count {
        return Some("workspace_gather_context".to_string());
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
        latency_kind,
        prefix_mutation_reasons: build_context_observation
            .map(|observation| observation.prefix_mutation_reasons.clone())
            .unwrap_or_default(),
    }
}


pub(super) fn derive_cache_miss_input_tokens(token_usage: Option<&TokenUsage>) -> Option<u64> {
    let usage = token_usage?;
    let input_tokens = usage.input_tokens?;
    let cache_hit_input_tokens = usage.cache_hit_input_tokens?;
    Some(input_tokens.saturating_sub(cache_hit_input_tokens))
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
        }),
        (Some(existing), None) => Some(existing),
        (None, Some(next)) => Some(next.clone()),
        (None, None) => None,
    }
}


pub(super) fn merge_cache_hit_source(left: Option<String>, right: Option<String>) -> Option<String> {
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


/// Extract the consecutive-failure signal for a tool result: `(tool name, error code)`.
/// `ok`/`partial` results, aborted executions, and pending control outcomes (`Ask` waits) never
/// count as failures, so they reset the consecutive counter.
pub(super) fn tool_failure_signal(tool_call: &ToolCall, tool_result: &ToolResult) -> Option<(String, String)> {
    if tool_result.status != "error" || tool_result_control_outcome_pending(tool_result) {
        return None;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(&tool_result.output) else {
        return None;
    };
    let error = parsed.get("error")?;
    let code = error
        .get("kind")
        .or_else(|| error.get("code"))
        .and_then(Value::as_str)?;
    Some((tool_call.name.clone(), code.to_string()))
}


/// Tracks how many consecutive failures share the same `(tool name, error code)` signal.
/// A success, an unclassifiable result, or a different signal resets the run.
#[derive(Clone, Debug, Default)]
pub(super) struct ConsecutiveFailureTracker {
    pub(super) signal: Option<(String, String)>,
    pub(super) count: usize,
}


/// Match the dispatcher's persisted `PendingControlRequest` to the originating tool call. The
/// request's `call_id` is the dispatch call id the executor persisted, which equals the original
/// assistant tool-call id when one was supplied; a missing call id falls back to the most recent
/// `Interaction` request (the Ask path).
pub(super) fn match_pending_control_request<'a>(
    requests: &'a [PendingControlRequest],
    tool_call: &ToolCall,
) -> Option<&'a PendingControlRequest> {
    if let Some(call_id) = tool_call.call_id.as_deref() {
        if let Some(found) = requests
            .iter()
            .find(|request| request.state == PendingControlRequestState::Pending && request.call_id == call_id)
        {
            return Some(found);
        }
    }
    requests
        .iter()
        .filter(|request| {
            request.state == PendingControlRequestState::Pending
                && request.request_kind == PendingControlRequestKind::Interaction
        })
        .last()
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
    pub(super) fn record(&mut self, signal: Option<(String, String)>) -> usize {
        match signal {
            Some(next) if self.signal.as_ref() == Some(&next) => {
                self.count += 1;
                self.count
            }
            Some(next) => {
                self.signal = Some(next);
                self.count = 1;
                1
            }
            None => {
                self.signal = None;
                self.count = 0;
                0
            }
        }
    }
}
