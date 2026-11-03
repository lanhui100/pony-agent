use super::*;

pub(super) const STREAM_REASONING_BATCH_CHARS: usize = 96;

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

/// Match the dispatcher's persisted `PendingControlRequest` to the originating tool call. The
/// request's `call_id` is the dispatch call id the executor persisted, which equals the original
/// assistant tool-call id when one was supplied; a missing call id falls back to the most recent
/// `Interaction` request (the Ask path).
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
