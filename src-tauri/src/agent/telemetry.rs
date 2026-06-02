use crate::agent::provider::PrefixMutationReason;
use crate::agent::tools::{ToolCall, ToolPlan, ToolPlanStep, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolActivity {
    pub id: String,
    pub name: String,
    pub status: String,
    pub summary: String,
    pub arguments_text: Option<String>,
    pub result_text: Option<String>,
    pub duration_seconds: Option<f64>,
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
#[serde(rename_all = "camelCase")]
pub struct ProviderCallCacheRecord {
    pub request_kind: ProviderRequestKind,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub cache_miss_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_mutation_reasons: Vec<PrefixMutationReason>,
}

pub trait TurnTelemetryBuilder: Send {
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

fn completed_trace_without_tool() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step("step-call-tool", "Call tool", "pending"),
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
        id: tool_activity_id(&active_call.name, None),
        name: active_call.name.to_string(),
        status: "running".to_string(),
        summary: running_summary(active_call),
        arguments_text: Some(pretty_json(&active_call.arguments)),
        result_text: None,
        duration_seconds: None,
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
        id: tool_activity_id(&active_call.name, None),
        name: active_call.name.to_string(),
        status: activity_status(result.status.as_str(), composite_result_status(&parsed)),
        summary: completed_summary(active_call, result.status.as_str(), &parsed),
        arguments_text: Some(pretty_json(&active_call.arguments)),
        result_text: Some(parent_result_text(&parsed, &result.output)),
        duration_seconds: Some(result.duration_ms as f64 / 1000.0),
        capability_invocation: None,
    }];

    activities.extend(nested_child_activities(active_call, &parsed));
    activities
}

fn tool_activity_id(tool_name: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(suffix) => format!("tool-{}-{}", tool_name.replace('.', "-"), suffix),
        None => format!("tool-{}", tool_name.replace('.', "-")),
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn parse_tool_output(output: &str) -> Value {
    serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()))
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
    if parsed.is_object() {
        pretty_json(parsed)
    } else {
        fallback.to_string()
    }
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
            id: tool_activity_id(&active_call.name, Some(&format!("planned-{}", index + 1))),
            name: step.name,
            status: "planned".to_string(),
            summary: step.summary,
            arguments_text: Some(pretty_json(&step.arguments)),
            result_text: None,
            duration_seconds: None,
            capability_invocation: None,
        })
        .collect()
}

fn nested_child_activities(active_call: &ToolCall, parsed: &Value) -> Vec<TurnToolActivity> {
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
        id: tool_activity_id(&active_call.name, Some(&format!("child-{}", position + 1))),
        name: tool_name.to_string(),
        status: match aggregate_status {
            "ok" => "done".to_string(),
            "partial" | "error" | "aborted" => "error".to_string(),
            _ => "done".to_string(),
        },
        summary: nested_summary(position, tool_name, aggregate_status, error_message),
        arguments_text: Some(pretty_json(&arguments)),
        result_text: Some(nested_result_text(&output, error_message)),
        duration_seconds,
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
    if let Some(summary_text) = output
        .get("summary")
        .and_then(|summary| summary.get("text"))
        .and_then(Value::as_str)
    {
        return summary_text.to_string();
    }

    if let Some(message) = error_message {
        return message.to_string();
    }

    pretty_json(output)
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
        assert_eq!(activities[1].name, "workspace.read_file");
        assert_eq!(activities[2].name, "workspace.search_text");
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
        assert_eq!(activities[0].name, "workspace_batch");
        assert_eq!(activities[0].status, "done");
        assert_eq!(activities[1].name, "workspace_read_file");
        assert_eq!(activities[1].status, "done");
        assert_eq!(activities[2].name, "workspace_search_text");
        assert_eq!(activities[2].status, "error");
        assert!(activities[2]
            .result_text
            .as_deref()
            .unwrap_or_default()
            .contains("search failed"));
    }
}
