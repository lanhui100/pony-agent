use crate::agent::provider::PrefixMutationReason;
use crate::agent::tools::{
    model_visible_tool_name, product_canonical_tool_name, tool_display_metadata_for_name,
    tool_error_from_output, ToolCall, ToolError, ToolPermissionFacts, ToolPlan, ToolPlanStep, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 工具结果文本的落库上限（32KB）。
/// 防止 WebSearch 等工具把完整搜索结果全文写入 trace，导致 session blob 膨胀
/// （实测单个 timeline 条目 222KB、会话 46MB 的根因之一）。
/// 前端预览足够；完整结果如需保留，由后续规范化存储阶段外置到文件。
const TOOL_RESULT_TEXT_MAX_CHARS: usize = 32 * 1024;

fn truncate_tool_result_text(text: String) -> String {
    if text.len() <= TOOL_RESULT_TEXT_MAX_CHARS {
        return text;
    }
    let mut truncated = text;
    // `String::truncate` 要求切线落在 UTF-8 字符边界上，否则 panic
    // （assertion failed: self.is_char_boundary(new_len)）。工具结果里中文/emoji 很常见，
    // 按字节直切必然会在 3 字节字符中间翻车，并把整轮 turn 线程带崩
    // （用户侧只看到“本轮请求未能完成”）。故先回退到最近的字界再切。
    let mut cut = TOOL_RESULT_TEXT_MAX_CHARS;
    while cut > 0 && !truncated.is_char_boundary(cut) {
        cut -= 1;
    }
    truncated.truncate(cut);
    truncated.push_str("\n...[truncated by pony-agent]");
    truncated
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTraceStep {
    pub id: String,
    pub label: String,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityInvocationRecord {
    pub tool_name: String,
    pub capability_id: Option<String>,
    pub source_id: Option<String>,
    pub source_kind: Option<String>,
    pub capability_kind: Option<String>,
    pub invocation_mode: Option<String>,
    pub failure_kind: Option<String>,
    pub requires_approval: Option<bool>,
    pub host_mediated: Option<bool>,
    pub permission_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_facts: Option<ToolPermissionFacts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composed_capability_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composed_capability_kinds: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_layer: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolActivity {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name_zh: Option<String>,
    pub status: String,
    #[serde(alias = "summary")]
    pub description: String,
    pub arguments_text: Option<String>,
    pub result_text: Option<String>,
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_activity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_invocation: Option<CapabilityInvocationRecord>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRequestKind {
    #[default]
    InitialRequest,
    ToolFollowup,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLatencyKind {
    ProviderStream,
    BufferedResponse,
    #[default]
    Unknown,
}

thread_local! {
    /// B139：最近一次 **失败** 的 provider HTTP 状态码。
    ///
    /// 为什么不改 74 处 `Result<_, String>` 签名：provider 调用与 per-call 记录构建跑在同一
    /// turn 线程上（与 `agent::retry` 的 RetryNotice 观察者同一模式），状态码作为「取走即清空」
    /// 的旁路传递，既不动错误类型，也不会让两次 call 的记录串味。
    static LAST_UPSTREAM_STATUS: std::cell::Cell<Option<u16>> = const { std::cell::Cell::new(None) };
}

/// 记录一次失败的 provider HTTP 响应状态码。
///
/// **唯一写入点**是 `provider::provider_status_error`（所有非 2xx 出口都经过它）；
/// 任何其它地方直接写会让「状态码 ↔ 错误」的对应关系不可审计。
pub fn record_upstream_status(status: u16) {
    LAST_UPSTREAM_STATUS.with(|slot| slot.set(Some(status)));
}

/// 取走并清空最近的状态码（per-call 记录结算时调用一次）。
///
/// 「取走即清空」是刻意的：一次 call 的状态码不得泄漏给下一次 call 的记录。
pub fn take_upstream_status() -> Option<u16> {
    LAST_UPSTREAM_STATUS.with(|slot| slot.replace(None))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCallCacheRecord {
    pub request_kind: ProviderRequestKind,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_source: Option<String>,
    pub cache_miss_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    /// wave-54 C3：usage 来源（`provider` | `estimated_chars`）。`None` ⇒ 键缺席
    /// （「未知」不得被渲染/上报成真值）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_origin: Option<String>,
    /// wave-54 C4：上下文输入口径（协议归一）——`anthropic*` ⇒ `input + cache_read`，
    /// 其余协议 ⇒ `input`（上游已含缓存命中）。`None` ⇒ UI 回退到上下文解析路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_input_tokens: Option<u64>,
    #[serde(default)]
    pub latency_kind: ProviderLatencyKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_mutation_reasons: Vec<PrefixMutationReason>,
    /// wave-acp-delegate C5.1：本 call 的**尝试次数**（恒 `Some(≥1)`：1 = 无重试；
    /// `None` 仅表示旧记录/未知，不表示「0 次」）。
    ///
    /// additive：旧记录反序列化缺省 `None`，旧消费者不读即行为不变；值由 W2b 在成功结算处
    /// 填入（本波 W1 只冻结字段，不赋业务值）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    /// wave-acp-delegate C5.1：失败尝试的累计耗时（毫秒）。**仅重试时** `Some(>0)`，
    /// 无重试 ⇒ `None` ⇒ 键缺席（禁止 0 占位冒充真值）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_attempt_ms: Option<u64>,
    /// wave-acp-delegate C5.1：首个**成功** attempt 之前的等待/预热时长（毫秒）。
    /// **仅重试时** `Some(>0)`，无重试 ⇒ `None` ⇒ 键缺席（禁止 0 占位冒充真值）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prelude_ms: Option<u64>,
    /// B139：上游 HTTP 响应状态码。**仅非 2xx** 时 `Some`（成功调用不写 200，键缺席）。
    ///
    /// 用途：把「配额抖动期的 429」从速率样本里筛出去——正文匹配（`is_rate_limit_error`）
    /// 只能给一个布尔，无法区分 429 / 503 / 业务配额耗尽。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
}

/// wave-acp-delegate C5.1：一次 provider 调用的**重试观测口径**——上面三个 additive 字段的
/// 唯一赋值真源（避免六处 `build_provider_call_cache_record` 调用点各自重算口径而漂移）。
///
/// 口径（契约矩阵 C5.1）：
/// - `attempts` 恒 `Some(≥1)`：1 = 无重试（含「无账本可读」的诚实下限，不得省略成 `None`）；
/// - `failed_attempt_ms` / `prelude_ms` **仅当真的发生过失败重试**时 `Some(>0)`，
///   否则 `None`（键缺席）——禁止 0 占位冒充真值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderCallRetryMeta {
    pub attempts: Option<u32>,
    pub failed_attempt_ms: Option<u64>,
    pub prelude_ms: Option<u64>,
}

impl Default for ProviderCallRetryMeta {
    fn default() -> Self {
        Self::single_attempt()
    }
}

impl ProviderCallRetryMeta {
    /// 无重试账本（第三方直调 / 测试桩 / 恢复路径）⇒ 只知「至少尝试过 1 次」。
    pub fn single_attempt() -> Self {
        Self {
            attempts: Some(1),
            failed_attempt_ms: None,
            prelude_ms: None,
        }
    }

    /// 由 [`crate::agent::retry::take_pure_call_window`] 的重试账本结算三键。
    ///
    /// `window == None` ⇒ 未开启账本（未观测到重试循环）⇒ 退化为 [`Self::single_attempt`]。
    /// 毫秒取整可能让极快的失败尝试落到 0：此时宁可让键缺席（诚实），也不伪造正数。
    pub fn from_window(window: Option<&crate::agent::retry::ProviderAttemptWindow>) -> Self {
        let Some(window) = window else {
            return Self::single_attempt();
        };
        // `attempts == 0` 只会出现在「账本开启但一次 attempt 都没记账」的异常路径上，
        // 但本函数只在调用**成功结算**处被调用 ⇒ 至少发生过 1 次尝试（恒 ≥1）。
        let attempts = window.attempts.max(1);
        if attempts <= 1 {
            return Self {
                attempts: Some(attempts),
                failed_attempt_ms: None,
                prelude_ms: None,
            };
        }
        Self {
            attempts: Some(attempts),
            failed_attempt_ms: (window.failed_attempt_ms > 0).then_some(window.failed_attempt_ms),
            prelude_ms: (window.prelude_ms() > 0).then_some(window.prelude_ms()),
        }
    }
}

pub trait TurnTelemetryBuilder: Send + Sync {
    fn start_trace_steps(&self) -> Vec<TurnTraceStep>;
    fn trace_tool_active(&self) -> Vec<TurnTraceStep>;
    fn trace_return_active(&self, tool_ok: bool) -> Vec<TurnTraceStep>;
    fn trace_return_active_without_tool(&self) -> Vec<TurnTraceStep>;
    fn completed_trace_with_tool(&self, tool_ok: bool) -> Vec<TurnTraceStep>;
    fn completed_trace_without_tool(&self) -> Vec<TurnTraceStep>;
    fn failed_trace_empty_input(&self) -> Vec<TurnTraceStep>;
    fn failed_trace_before_tool(&self) -> Vec<TurnTraceStep>;
    fn failed_trace_after_tool(&self, tool_ok: bool) -> Vec<TurnTraceStep>;
    fn tool_activities_running(&self, active_call: &ToolCall) -> Vec<TurnToolActivity>;
    fn tool_activities_after_result(
        &self,
        active_call: &ToolCall,
        result: &ToolResult,
    ) -> Vec<TurnToolActivity>;
}

pub struct DefaultTurnTelemetryBuilder;

impl TurnTelemetryBuilder for DefaultTurnTelemetryBuilder {
    fn start_trace_steps(&self) -> Vec<TurnTraceStep> {
        start_trace_steps()
    }

    fn trace_tool_active(&self) -> Vec<TurnTraceStep> {
        trace_tool_active()
    }

    fn trace_return_active(&self, tool_ok: bool) -> Vec<TurnTraceStep> {
        trace_return_active(tool_ok)
    }

    fn trace_return_active_without_tool(&self) -> Vec<TurnTraceStep> {
        trace_return_active_without_tool()
    }

    fn completed_trace_with_tool(&self, tool_ok: bool) -> Vec<TurnTraceStep> {
        completed_trace_with_tool(tool_ok)
    }

    fn completed_trace_without_tool(&self) -> Vec<TurnTraceStep> {
        completed_trace_without_tool()
    }

    fn failed_trace_empty_input(&self) -> Vec<TurnTraceStep> {
        failed_trace_empty_input()
    }

    fn failed_trace_before_tool(&self) -> Vec<TurnTraceStep> {
        failed_trace_before_tool()
    }

    fn failed_trace_after_tool(&self, tool_ok: bool) -> Vec<TurnTraceStep> {
        failed_trace_after_tool(tool_ok)
    }

    fn tool_activities_running(&self, active_call: &ToolCall) -> Vec<TurnToolActivity> {
        tool_activities_running(active_call)
    }

    fn tool_activities_after_result(
        &self,
        active_call: &ToolCall,
        result: &ToolResult,
    ) -> Vec<TurnToolActivity> {
        tool_activities_after_result(active_call, result)
    }
}

fn start_trace_steps() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "active"),
        trace_step("step-call-tool", "Call tool", "pending"),
    ]
}

fn trace_tool_active() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step("step-call-tool", "Call tool", "active"),
    ]
}

fn trace_return_active(tool_ok: bool) -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step(
            "step-call-tool",
            "Call tool",
            if tool_ok { "completed" } else { "error" },
        ),
    ]
}

fn trace_return_active_without_tool() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step("step-call-tool", "Call tool", "pending"),
    ]
}

fn completed_trace_with_tool(tool_ok: bool) -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step(
            "step-call-tool",
            "Call tool",
            if tool_ok { "completed" } else { "error" },
        ),
    ]
}

/// P5：本轮从未派发工具 ⇒ `step-call-tool` 是 `skipped`（不是 `pending`）。
///
/// 为什么不是 `pending`：`pending` 的语义是「排队等待执行」，会让轨迹面板看起来像卡死；
/// `completed` 又会谎报「调用过工具」。`skipped` 是唯一不撒谎且可区分「没调用」的状态。
/// 流式进行中的 `trace_return_active_without_tool` 仍为 `pending`（此刻确实可能再调工具）。
fn completed_trace_without_tool() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step("step-call-tool", "Call tool", "skipped"),
    ]
}

fn failed_trace_empty_input() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "error"),
        trace_step("step-context", "Build context", "pending"),
        trace_step("step-call-model", "Call model", "pending"),
        trace_step("step-call-tool", "Call tool", "pending"),
    ]
}

fn failed_trace_before_tool() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "error"),
        trace_step("step-call-tool", "Call tool", "pending"),
    ]
}

fn failed_trace_after_tool(tool_ok: bool) -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step(
            "step-call-tool",
            "Call tool",
            if tool_ok { "completed" } else { "error" },
        ),
    ]
}

fn trace_step(id: &str, label: &str, state: &str) -> TurnTraceStep {
    TurnTraceStep {
        id: id.to_string(),
        label: label.to_string(),
        state: state.to_string(),
    }
}

fn tool_activities_running(active_call: &ToolCall) -> Vec<TurnToolActivity> {
    let mut activities = vec![TurnToolActivity {
        id: tool_activity_id(active_call, None),
        name: model_visible_tool_name(&active_call.name).to_string(),
        canonical_tool_name: Some(product_canonical_tool_name(&active_call.name).to_string()),
        display_name_zh: tool_display_metadata_for_name(&active_call.name).display_name_zh,
        status: "running".to_string(),
        description: extract_llm_description(&active_call.arguments)
            .unwrap_or_else(|| running_summary(active_call)),
        arguments_text: Some(pretty_json(&active_call.arguments)),
        result_text: None,
        duration_seconds: None,
        parent_activity_id: None,
        artifacts: None,
        error: None,
        capability_invocation: None,
    }];

    activities.extend(planned_child_activities(active_call));
    activities
}

fn tool_activities_after_result(
    active_call: &ToolCall,
    result: &ToolResult,
) -> Vec<TurnToolActivity> {
    let parsed = parse_tool_output(&result.output);
    let mut activities = vec![TurnToolActivity {
        id: tool_activity_id(active_call, None),
        name: model_visible_tool_name(&active_call.name).to_string(),
        canonical_tool_name: Some(product_canonical_tool_name(&active_call.name).to_string()),
        display_name_zh: tool_display_metadata_for_name(&active_call.name).display_name_zh,
        status: activity_status(result.status.as_str(), composite_result_status(&parsed)),
        description: extract_llm_description(&active_call.arguments)
            .or_else(|| extract_description_from_result(&parsed))
            .unwrap_or_else(|| completed_summary(active_call, result.status.as_str(), &parsed)),
        arguments_text: Some(pretty_json(&active_call.arguments)),
        result_text: Some(parent_result_text(&parsed, &result.output)),
        duration_seconds: Some(result.duration_ms as f64 / 1000.0),
        parent_activity_id: None,
        artifacts: Some(extract_activity_artifacts(&parsed)),
        error: tool_error_from_output(result.status.as_str(), &parsed)
            .or_else(|| composite_error_from_children(&parsed))
            .map(|error| serde_json::to_value(error).unwrap_or(Value::Null)),
        capability_invocation: None,
    }];

    activities.extend(nested_child_activities(active_call, &parsed));
    activities
}

fn composite_error_from_children(parsed: &Value) -> Option<ToolError> {
    let results = parsed.get("results").and_then(Value::as_array)?;
    for entry in results {
        let is_ok = entry.get("ok").and_then(Value::as_bool).unwrap_or(true);
        let agg_status = entry.get("aggregateStatus").and_then(Value::as_str).unwrap_or("ok");
        if !is_ok || agg_status != "ok" {
            if let Some(err) = entry.get("error").filter(|e| !e.is_null()) {
                let code = err.get("code").or_else(|| err.get("kind")).and_then(Value::as_str).unwrap_or("child_failed");
                let message = err.get("message").and_then(Value::as_str).unwrap_or("子调用执行失败");
                return Some(ToolError {
                    kind: code.to_string(),
                    message: message.to_string(),
                    details: err.get("details").cloned(),
                    retryable: err.get("retryable").and_then(Value::as_bool),
                    source: err.get("source").and_then(Value::as_str).map(ToString::to_string),
                });
            } else if let Some(out_err) = entry.get("output").and_then(|o| o.get("error")).filter(|e| !e.is_null()) {
                let code = out_err.get("code").or_else(|| out_err.get("kind")).and_then(Value::as_str).unwrap_or("child_failed");
                let message = out_err.get("message").and_then(Value::as_str).unwrap_or("子调用执行失败");
                return Some(ToolError {
                    kind: code.to_string(),
                    message: message.to_string(),
                    details: out_err.get("details").cloned(),
                    retryable: out_err.get("retryable").and_then(Value::as_bool),
                    source: out_err.get("source").and_then(Value::as_str).map(ToString::to_string),
                });
            } else if let Some(summary_msg) = entry.get("output").and_then(|o| o.get("summary")).and_then(|s| s.get("text")).and_then(Value::as_str) {
                return Some(ToolError {
                    kind: "child_failed".to_string(),
                    message: summary_msg.to_string(),
                    details: None,
                    retryable: None,
                    source: None,
                });
            }
        }
    }
    None
}

fn tool_activity_id(active_call: &ToolCall, suffix: Option<&str>) -> String {
    if let Some(call_id) = active_call.call_id.as_deref().filter(|s| !s.trim().is_empty()) {
        match suffix {
            Some(suffix) => format!("{}-{}", call_id, suffix),
            None => call_id.to_string(),
        }
    } else {
        let tool_name = &active_call.name;
        match suffix {
            Some(suffix) => format!("tool-{}-{}", tool_name.replace('.', "-"), suffix),
            None => format!("tool-{}", tool_name.replace('.', "-")),
        }
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn parse_tool_output(output: &str) -> Value {
    serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()))
}

fn extract_llm_description(arguments: &Value) -> Option<String> {
    arguments
        .get("description")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

fn extract_description_from_result(parsed: &Value) -> Option<String> {
    parsed
        .get("description")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

fn running_summary(active_call: &ToolCall) -> String {
    match explicit_tool_plan_from_call(active_call).or_else(|| composite_child_plan(active_call)) {
        Some(plan) if !plan.steps.is_empty() => format!(
            "{} 当前待执行 {} 个子调用。",
            plan.summary,
            plan.steps.len()
        ),
        _ => "Tool call in progress.".to_string(),
    }
}

fn completed_summary(_active_call: &ToolCall, runtime_status: &str, parsed: &Value) -> String {
    if let Some(plan_summary) = parsed
        .get("plan")
        .and_then(explicit_tool_plan_from_value)
        .map(|plan| plan.summary)
    {
        let aggregate_status = parsed
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(runtime_status);
        return format!("{} (aggregate={}).", plan_summary, aggregate_status);
    }

    if let Some(summary_text) = parsed
        .get("summary")
        .and_then(|summary| summary.get("text"))
        .and_then(Value::as_str)
    {
        let aggregate_status = parsed
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(runtime_status);
        return format!("{} (aggregate={}).", summary_text, aggregate_status);
    }

    format!("Tool call finished with status: {}.", runtime_status)
}

fn parent_result_text(parsed: &Value, fallback: &str) -> String {
    let text = if parsed.is_object() {
        pretty_json(parsed)
    } else {
        fallback.to_string()
    };
    truncate_tool_result_text(text)
}

fn composite_result_status(parsed: &Value) -> Option<&str> {
    parsed.get("status").and_then(Value::as_str)
}

fn activity_status(runtime_status: &str, aggregate_status: Option<&str>) -> String {
    if runtime_status != "ok" {
        return "error".to_string();
    }

    match aggregate_status {
        Some("error") | Some("aborted") => "error".to_string(),
        _ => "done".to_string(),
    }
}

fn planned_child_activities(active_call: &ToolCall) -> Vec<TurnToolActivity> {
    let Some(plan) =
        explicit_tool_plan_from_call(active_call).or_else(|| composite_child_plan(active_call))
    else {
        return Vec::new();
    };

    plan.steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| TurnToolActivity {
            id: tool_activity_id(active_call, Some(&format!("planned-{}", index + 1))),
            name: model_visible_tool_name(&step.name).to_string(),
            canonical_tool_name: Some(product_canonical_tool_name(&step.name).to_string()),
            display_name_zh: tool_display_metadata_for_name(&step.name).display_name_zh,
            status: "planned".to_string(),
            description: extract_llm_description(&step.arguments)
                .unwrap_or_else(|| step.summary.clone()),
            arguments_text: Some(pretty_json(&step.arguments)),
            result_text: None,
            duration_seconds: None,
            parent_activity_id: Some(tool_activity_id(active_call, None)),
            artifacts: None,
            error: None,
            capability_invocation: None,
        })
        .collect()
}

fn nested_child_activities(active_call: &ToolCall, parsed: &Value) -> Vec<TurnToolActivity> {
    if !matches!(
        active_call.name.as_str(),
        "workspace_batch"
            | "workspace.batch"
            | "workspace_gather_context"
            | "workspace.gather_context"
    ) {
        return Vec::new();
    }

    parsed
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .enumerate()
                .map(|(position, entry)| nested_result_to_activity(active_call, position, entry))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn nested_result_to_activity(
    active_call: &ToolCall,
    position: usize,
    entry: &Value,
) -> TurnToolActivity {
    let aggregate_status = entry
        .get("aggregateStatus")
        .and_then(Value::as_str)
        .unwrap_or("error");
    let tool_name = entry
        .get("canonicalTool")
        .and_then(Value::as_str)
        .or_else(|| entry.get("tool").and_then(Value::as_str))
        .unwrap_or("unknown_tool");
    let arguments = entry.get("arguments").cloned().unwrap_or(Value::Null);
    let output = entry.get("output").cloned().unwrap_or(Value::Null);
    let duration_seconds = entry
        .get("durationMs")
        .and_then(Value::as_u64)
        .map(|ms| ms as f64 / 1000.0);
    let error_message = entry
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str);

    TurnToolActivity {
        id: tool_activity_id(active_call, Some(&format!("child-{}", position + 1))),
        name: model_visible_tool_name(tool_name).to_string(),
        canonical_tool_name: Some(product_canonical_tool_name(tool_name).to_string()),
        display_name_zh: tool_display_metadata_for_name(tool_name).display_name_zh,
        status: match aggregate_status {
            "ok" => "done".to_string(),
            "partial" | "error" | "aborted" => "error".to_string(),
            _ => "done".to_string(),
        },
        description: extract_llm_description(&arguments).unwrap_or_else(|| {
            nested_summary(position, tool_name, aggregate_status, error_message)
        }),
        arguments_text: Some(pretty_json(&arguments)),
        result_text: Some(nested_result_text(&output, error_message)),
        duration_seconds,
        parent_activity_id: Some(tool_activity_id(active_call, None)),
        artifacts: Some(extract_activity_artifacts(&output)),
        error: entry.get("error").cloned(),
        capability_invocation: None,
    }
}

fn nested_summary(
    position: usize,
    tool_name: &str,
    aggregate_status: &str,
    error_message: Option<&str>,
) -> String {
    match error_message {
        Some(message) if !message.trim().is_empty() => format!(
            "Subcall #{} `{}` finished with aggregate={} ({})",
            position + 1,
            tool_name,
            aggregate_status,
            message.trim()
        ),
        _ => format!(
            "Subcall #{} `{}` finished with aggregate={}.",
            position + 1,
            tool_name,
            aggregate_status
        ),
    }
}

fn nested_result_text(output: &Value, error_message: Option<&str>) -> String {
    let text = if let Some(summary_text) = output
        .get("summary")
        .and_then(|summary| summary.get("text"))
        .and_then(Value::as_str)
    {
        summary_text.to_string()
    } else if let Some(message) = error_message {
        message.to_string()
    } else {
        pretty_json(output)
    };
    truncate_tool_result_text(text)
}

fn extract_activity_artifacts(parsed: &Value) -> Vec<Value> {
    let mut artifacts = Vec::new();
    if let Some(path) = parsed.get("path") {
        artifacts.push(json!({
            "kind": "path",
            "value": path,
        }));
    }
    artifacts
}

fn composite_child_plan(active_call: &ToolCall) -> Option<ToolPlan> {
    match active_call.name.as_str() {
        "workspace_batch" | "workspace.batch" => Some(batch_child_plan(active_call)),
        "workspace_gather_context" | "workspace.gather_context" => gather_child_plan(active_call),
        _ => None,
    }
}

fn batch_child_plan(active_call: &ToolCall) -> ToolPlan {
    let steps = active_call
        .arguments
        .get("calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .enumerate()
                .filter_map(|(index, call)| {
                    let name = call.get("name").and_then(Value::as_str)?;
                    let arguments = call
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Default::default()));
                    Some(ToolPlanStep {
                        name: name.to_string(),
                        summary: format!("Planned subcall #{} for `{}`.", index + 1, name),
                        arguments,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    ToolPlan {
        kind: "batch".to_string(),
        summary: format!("Composite batch call for {} subcalls.", steps.len()),
        parallel: active_call
            .arguments
            .get("parallel")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        continue_on_error: active_call
            .arguments
            .get("continueOnError")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        steps,
    }
}

fn gather_child_plan(active_call: &ToolCall) -> Option<ToolPlan> {
    let paths = active_call
        .arguments
        .get("paths")
        .and_then(Value::as_array)?;
    let steps = paths
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let path = value.as_str()?.trim();
            if path.is_empty() {
                return None;
            }

            Some(ToolPlanStep {
                name: "workspace_gather_context".to_string(),
                summary: format!("Planned gather subcall #{} for path `{}`.", index + 1, path),
                arguments: serde_json::json!({
                    "path": path,
                }),
            })
        })
        .collect::<Vec<_>>();

    Some(ToolPlan {
        kind: "gather_context".to_string(),
        summary: format!("Composite gather call for {} paths.", steps.len()),
        parallel: false,
        continue_on_error: true,
        steps,
    })
}

fn explicit_tool_plan_from_call(active_call: &ToolCall) -> Option<ToolPlan> {
    active_call.plan.clone().or_else(|| {
        active_call
            .arguments
            .get("toolPlan")
            .and_then(explicit_tool_plan_from_value)
    })
}

fn explicit_tool_plan_from_value(value: &Value) -> Option<ToolPlan> {
    serde_json::from_value::<ToolPlan>(value.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn batch_running_exposes_planned_subcalls() {
        let call = ToolCall {
            call_id: None,
            name: "workspace.batch".to_string(),
            arguments: json!({
                "calls": [
                    {
                        "name": "workspace.read_file",
                        "arguments": { "path": "README.md" }
                    },
                    {
                        "name": "workspace.search_text",
                        "arguments": { "query": "ProviderManager" }
                    }
                ]
            }),
            plan: None,
        };

        let activities = tool_activities_running(&call);

        assert_eq!(activities.len(), 3);
        assert_eq!(activities[0].status, "running");
        assert_eq!(activities[1].status, "planned");
        assert_eq!(activities[1].name, "Read");
        assert_eq!(activities[2].name, "Search");
    }

    #[test]
    fn batch_result_expands_nested_results() {
        let call = ToolCall {
            call_id: None,
            name: "workspace_batch".to_string(),
            arguments: json!({
                "calls": [
                    {
                        "name": "workspace_read_file",
                        "arguments": { "path": "README.md" }
                    },
                    {
                        "name": "workspace_search_text",
                        "arguments": { "query": "AgentRuntime" }
                    }
                ]
            }),
            plan: None,
        };
        let result = ToolResult {
            tool_name: "workspace_batch".to_string(),
            status: "ok".to_string(),
            output: serde_json::to_string(&json!({
                "ok": true,
                "status": "partial",
                "summary": {
                    "text": "workspace_batch 已汇总 2 个子调用，整体状态为 partial。"
                },
                "results": [
                    {
                        "index": 0,
                        "tool": "workspace_read_file",
                        "canonicalTool": "workspace_read_file",
                        "arguments": { "path": "README.md" },
                        "status": "ok",
                        "aggregateStatus": "ok",
                        "durationMs": 12,
                        "output": { "ok": true, "text": "read ok" }
                    },
                    {
                        "index": 1,
                        "tool": "workspace_search_text",
                        "canonicalTool": "workspace_search_text",
                        "arguments": { "query": "AgentRuntime" },
                        "status": "error",
                        "aggregateStatus": "error",
                        "durationMs": 3,
                        "error": { "message": "search failed" },
                        "output": { "ok": false }
                    }
                ]
            }))
            .expect("result payload"),
            duration_ms: 48,
        };

        let activities = tool_activities_after_result(&call, &result);

        assert_eq!(activities.len(), 3);
        assert_eq!(activities[0].name, "BatchExecute");
        assert_eq!(activities[0].status, "done");
        assert_eq!(activities[1].name, "Read");
        assert_eq!(activities[1].status, "done");
        assert_eq!(activities[2].name, "Search");
        assert_eq!(activities[2].status, "error");
        assert!(activities[2]
            .result_text
            .as_deref()
            .unwrap_or_default()
            .contains("search failed"));
    }

    #[test]
    fn web_search_result_does_not_expand_search_hits_as_child_activities() {
        let call = ToolCall {
            call_id: None,
            name: "web_search_query".to_string(),
            arguments: json!({
                "query": "pony agent"
            }),
            plan: None,
        };
        let result = ToolResult {
            tool_name: "web_search_query".to_string(),
            status: "ok".to_string(),
            output: serde_json::to_string(&json!({
                "ok": true,
                "query": "pony agent",
                "resultCount": 2,
                "results": [
                    {
                        "title": "Pony Agent",
                        "url": "https://example.com/pony-agent",
                        "snippet": "Agent home page",
                        "score": 0.9
                    },
                    {
                        "title": "Docs",
                        "url": "https://example.com/docs",
                        "snippet": "Documentation",
                        "score": 0.7
                    }
                ],
                "summary": {
                    "text": "已完成搜索，返回 2 条结果。"
                }
            }))
            .expect("result payload"),
            duration_ms: 18,
        };

        let activities = tool_activities_after_result(&call, &result);

        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].name, "WebSearch");
        assert_eq!(activities[0].status, "done");
        assert!(activities[0]
            .result_text
            .as_deref()
            .unwrap_or_default()
            .contains("resultCount"));
    }

    #[test]
    fn multibyte_tool_result_text_truncates_on_char_boundary() {
        // 回归：旧实现按字节直切，多字节字符（中文/emoji）会让 String::truncate
        // 触发 is_char_boundary 断言 ⇒ 整轮 turn 线程 panic ⇒ 用户只看到“本轮请求未能完成”。
        let text = "中".repeat(TOOL_RESULT_TEXT_MAX_CHARS / 3 + 2);
        let truncated = truncate_tool_result_text(text);
        const MARKER: &str = "\n...[truncated by pony-agent]";
        assert!(truncated.ends_with(MARKER), "超限结果必须追加截断标记");
        let body_len = truncated.len() - MARKER.len();
        assert!(body_len <= TOOL_RESULT_TEXT_MAX_CHARS, "截断体不得超过预算");
        assert!(truncated.is_char_boundary(body_len), "切线必须落在字符边界");
        assert!(body_len > TOOL_RESULT_TEXT_MAX_CHARS - 4, "只允许为字界回退少量字节");
    }

    #[test]
    fn oversized_tool_result_text_is_truncated() {
        // 构造超过 32KB 的工具结果，验证 result_text 被截断并追加标记。
        let huge_output = json!({
            "ok": true,
            "query": "oversized",
            "results": [
                { "snippet": "x".repeat(TOOL_RESULT_TEXT_MAX_CHARS + 10_000) }
            ]
        });
        let call = ToolCall {
            call_id: None,
            name: "web_search_query".to_string(),
            arguments: json!({ "query": "oversized" }),
            plan: None,
        };
        let result = ToolResult {
            tool_name: "web_search_query".to_string(),
            status: "ok".to_string(),
            output: serde_json::to_string(&huge_output).expect("result payload"),
            duration_ms: 18,
        };

        let activities = tool_activities_after_result(&call, &result);
        let text = activities[0].result_text.as_deref().unwrap_or_default();

        assert!(text.len() <= TOOL_RESULT_TEXT_MAX_CHARS + 64, "截断后长度受控");
        assert!(text.ends_with("...[truncated by pony-agent]"), "追加截断标记");
    }

    #[test]
    fn small_tool_result_text_is_not_truncated() {
        let call = ToolCall {
            call_id: None,
            name: "web_search_query".to_string(),
            arguments: json!({ "query": "small" }),
            plan: None,
        };
        let result = ToolResult {
            tool_name: "web_search_query".to_string(),
            status: "ok".to_string(),
            output: serde_json::to_string(&json!({
                "ok": true,
                "query": "small",
                "resultCount": 1,
                "results": [{ "title": "t", "url": "u", "snippet": "s" }]
            }))
            .expect("result payload"),
            duration_ms: 18,
        };

        let activities = tool_activities_after_result(&call, &result);
        let text = activities[0].result_text.as_deref().unwrap_or_default();

        assert!(text.contains("resultCount"), "小结果保留完整内容");
        assert!(!text.contains("truncated by pony-agent"), "小结果不截断");
    }

    /// P5：无工具**完成**的 turn，`step-call-tool` 必须是 `skipped`。
    ///
    /// 旧行为是 `pending`（看起来像卡死在排队），`completed` 又会谎报「调用过工具」；
    /// 进行中的无工具轨迹仍保持 `pending`——此刻确实还可能再派发工具。
    #[test]
    fn completed_without_tool_marks_call_tool_skipped() {
        let step_state = |steps: &[TurnTraceStep]| {
            steps
                .iter()
                .find(|step| step.id == "step-call-tool")
                .expect("轨迹必须含 step-call-tool")
                .state
                .clone()
        };

        assert_eq!(
            step_state(&completed_trace_without_tool()),
            "skipped",
            "P5：无工具完成的 turn 不得把 step-call-tool 留在 pending"
        );
        assert_eq!(
            step_state(&trace_return_active_without_tool()),
            "pending",
            "P5：进行中的无工具轨迹仍应 pending（后续还可能调工具）"
        );
    }
}

#[cfg(test)]
mod upstream_status_tests {
    use super::{record_upstream_status, take_upstream_status};

    #[test]
    fn recorded_status_is_read_back_exactly_once() {
        assert_eq!(take_upstream_status(), None, "初始态不得凭空造出状态码");
        record_upstream_status(429);
        assert_eq!(take_upstream_status(), Some(429));
        assert_eq!(
            take_upstream_status(),
            None,
            "取走即清空：一次 call 的状态码不得泄漏给下一次 call"
        );
    }

    #[test]
    fn later_record_overwrites_earlier_one() {
        record_upstream_status(503);
        record_upstream_status(429);
        assert_eq!(
            take_upstream_status(),
            Some(429),
            "同一 call 内多次非 2xx 时，最后一次（真正上抛的那次）生效"
        );
    }
}
