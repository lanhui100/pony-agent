use super::*;

pub(super) fn is_out_of_scope_tool_result(tool_result: &crate::agent::tools::ToolResult) -> bool {
    let parsed = serde_json::from_str::<Value>(&tool_result.output).unwrap_or(Value::Null);
    if let Some(error) = tool_error_from_output(tool_result.status.as_str(), &parsed) {
        if error.kind == "out_of_scope" {
            return true;
        }
        if error.message.contains("只允许访问当前工作区内的相对路径") {
            return true;
        }
    }
    false
}


pub(super) fn tool_result_failure_kind(
    tool_result: &crate::agent::tools::ToolResult,
) -> Option<CapabilityFailureKind> {
    let parsed = serde_json::from_str::<Value>(&tool_result.output).unwrap_or(Value::Null);
    if let Some(error) = parsed.get("error") {
        let kind = error
            .get("kind")
            .or_else(|| error.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "out_of_scope" {
            return Some(CapabilityFailureKind::OutOfScope);
        }
        return Some(CapabilityFailureKind::InvocationFailed);
    }

    if tool_result.status == "ok" {
        if parsed.get("status").and_then(Value::as_str) == Some("partial") {
            if let Some(first_error) = parsed
                .get("summary")
                .and_then(|summary| summary.get("firstError"))
                .and_then(Value::as_object)
            {
                let kind = first_error
                    .get("kind")
                    .or_else(|| first_error.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if kind == "out_of_scope" {
                    return Some(CapabilityFailureKind::OutOfScope);
                }
                return Some(CapabilityFailureKind::InvocationFailed);
            }
        }
        return None;
    }

    Some(CapabilityFailureKind::InvocationFailed)
}


pub(super) fn is_registry_resource_tool_name(name: &str) -> bool {
    matches!(canonical_tool_name(name), Some("mcp_resource_read"))
}


pub(super) fn is_registry_tool_search_name(name: &str) -> bool {
    matches!(canonical_tool_name(name), Some("tool_search"))
}


#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub message: String,
    pub display_message: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    pub workspace_mode: Option<String>,
    pub session_id: Option<String>,
    pub node_id: Option<String>,
    #[serde(default)]
    pub history: Vec<TurnHistoryMessage>,
    #[serde(default)]
    pub images: Vec<TurnInputImage>,
    /// Workspace 归属（PA-079）：携带时在会话首次持久化时盖章。
    #[serde(default)]
    pub workspace_id: Option<String>,
}


#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub event_version: Option<String>,
    pub sequence: Option<u64>,
    pub emitted_at_ms: Option<u64>,
    pub phase: String,
    pub provider_requested_name: String,
    pub provider_name: String,
    pub provider_protocol: String,
    pub provider_model: String,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub user_message: String,
    pub assistant_message: String,
    pub trace_steps: Vec<TurnTraceStep>,
    pub trace_timeline: Vec<TraceTimelineEntry>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub provider_call_records: Vec<ProviderCallCacheRecord>,
    pub hook_trace_records: Vec<HookTraceRecord>,
    pub session_summary: String,
}


#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStreamEvent {
    pub event_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: String,
    pub kind: String,
    pub event_type: Option<String>,
    pub event_version: Option<String>,
    pub sequence: Option<u64>,
    pub emitted_at_ms: Option<u64>,
    pub phase: Option<String>,
    pub text: Option<String>,
    pub reasoning_content: Option<String>,
    pub error: Option<String>,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub trace_steps: Option<Vec<TurnTraceStep>>,
    pub trace_timeline: Option<Vec<TraceTimelineEntry>>,
    pub tool_activities: Option<Vec<TurnToolActivity>>,
    pub provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    pub hook_trace_records: Option<Vec<HookTraceRecord>>,
    pub session_summary: Option<String>,
    /// PA-095 #4：逻辑 hop 索引（0-based；None = 缺省归 step 0，wire 兼容）。
    /// delta/chunk 携带所属 hop；turn:trace(calling_model) 携带即将发起的
    /// followup hop 索引（≥1，初始 call 的 step/start 由 turn:started 承担）。
    #[serde(default)]
    pub step: Option<u32>,
}


pub(super) const CANCELLED_TURN_MESSAGE: &str = "用户终止，发送消息可继续。";

pub(super) const HOOK_FAILTURN_HANDLED_SENTINEL: &str = "__hook_failturn_handled__";


#[derive(Clone)]
pub(super) struct ToolTurnHopRecord {
    pub(super) assistant_message: Option<Value>,
    pub(super) assistant_output_text: String,
    pub(super) assistant_reasoning_content: Option<String>,
    pub(super) assistant_reasoning_content_value: Option<Value>,
    pub(super) tool_call: ToolCall,
    pub(super) tool_result: crate::agent::tools::ToolResult,
}


pub(super) fn model_hop_trace_contents(hop_records: &[ToolTurnHopRecord]) -> Vec<ModelHopTraceContent> {
    hop_records
        .iter()
        .map(|hop| ModelHopTraceContent {
            text: hop.assistant_output_text.clone(),
            reasoning_content: hop.assistant_reasoning_content.clone(),
        })
        .collect()
}


pub(super) fn model_hop_trace_contents_with_current(
    hop_records: &[ToolTurnHopRecord],
    current_text: &str,
    current_reasoning: Option<&str>,
) -> Vec<ModelHopTraceContent> {
    let mut contents = model_hop_trace_contents(hop_records);
    contents.push(ModelHopTraceContent {
        text: current_text.to_string(),
        reasoning_content: current_reasoning.map(str::to_string),
    });
    contents
}


pub(super) struct RecoveredToolFollowup {
    pub(super) response: ProviderResponse,
    pub(super) provider_call_record: ProviderCallCacheRecord,
}


pub(crate) struct HookDispatchOutcome {
    pub(super) trace_records: Vec<HookTraceRecord>,
    pub(super) fail_turn_error: Option<String>,
}


pub(crate) struct CapabilityMediationDispatchOutcome {
    pub(super) arguments: Value,
    pub(super) trace_records: Vec<HookTraceRecord>,
    pub(super) blocked_error: Option<String>,
    pub(super) fail_turn_error: Option<String>,
}


pub(crate) struct PlannerDispatchOutcome {
    pub(super) decision: Option<ProviderDecision>,
    pub(super) selected_tool_call: Option<ToolCall>,
    pub(super) trace_records: Vec<HookTraceRecord>,
    pub(super) blocked_error: Option<String>,
    pub(super) fail_turn_error: Option<String>,
}


pub struct PlannerGraphDecisionDispatchOutcome {
    pub decision: GraphDecision,
    pub trace_records: Vec<HookTraceRecord>,
}


pub(crate) struct GraphDecisionDispatchOutcome {
    pub(super) decision: GraphDecision,
    pub(super) trace_records: Vec<HookTraceRecord>,
    pub(super) blocked_error: Option<String>,
    pub(super) fail_turn_error: Option<String>,
}


pub(super) struct NormalizedToolDirective {
    pub(super) tool_call: ToolCall,
    pub(super) assistant_message: Option<Value>,
}


/// Turn-level invocation facts for Ask control-request persistence (design.md Decision 5,
/// PA-076 P1-1). The control plane sets these before each graph-run turn so a persisted
/// `PendingControlRequest` is bound to the real session/run/turn; plain `run_turn` calls apply
/// the input's session with `None` run/turn facts.
#[derive(Clone, Debug, Default)]
pub struct RunTurnFacts {
    pub run_id: Option<String>,
    pub turn_id: Option<String>,
    pub workspace_root: Option<String>,
}


/// `TurnResult.phase` value for a turn paused on a control outcome (Ask pending). The host must
/// answer the persisted `PendingControlRequest` before the run resumes; the runtime never feeds a
/// `control_outcome_pending` result to the provider as a follow-up.
pub const SUSPENDED_TURN_PHASE: &str = "suspended";


/// Runtime-internal snapshot of a suspended Ask turn: the persisted pending request the host must
/// answer and whether the graph run was bound to `WaitingUser`.
pub(super) struct ToolTurnSuspension {
    pub(super) pending_request: Option<PendingControlRequest>,
    pub(super) bound_run_id: Option<String>,
}
