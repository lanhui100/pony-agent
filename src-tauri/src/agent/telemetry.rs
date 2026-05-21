use crate::agent::tools::{ToolCall, ToolResult};
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTraceStep {
    pub id: String,
    pub label: String,
    pub state: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolActivity {
    pub id: String,
    pub name: String,
    pub status: String,
    pub summary: String,
    pub arguments_text: Option<String>,
    pub result_text: Option<String>,
    pub duration_seconds: Option<f64>,
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
        trace_step("step-return", "Return result", "pending"),
    ]
}

fn trace_tool_active() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step("step-call-tool", "Call tool", "active"),
        trace_step("step-return", "Return result", "pending"),
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
        trace_step("step-return", "Return result", "active"),
    ]
}

fn trace_return_active_without_tool() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step("step-call-tool", "Call tool", "pending"),
        trace_step("step-return", "Return result", "active"),
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
        trace_step("step-return", "Return result", "completed"),
    ]
}

fn completed_trace_without_tool() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "completed"),
        trace_step("step-call-tool", "Call tool", "pending"),
        trace_step("step-return", "Return result", "completed"),
    ]
}

fn failed_trace_empty_input() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "error"),
        trace_step("step-context", "Build context", "pending"),
        trace_step("step-call-model", "Call model", "pending"),
        trace_step("step-call-tool", "Call tool", "pending"),
        trace_step("step-return", "Return result", "pending"),
    ]
}

fn failed_trace_before_tool() -> Vec<TurnTraceStep> {
    vec![
        trace_step("step-plan", "Receive input", "completed"),
        trace_step("step-context", "Build context", "completed"),
        trace_step("step-call-model", "Call model", "error"),
        trace_step("step-call-tool", "Call tool", "pending"),
        trace_step("step-return", "Return result", "pending"),
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
        trace_step("step-return", "Return result", "error"),
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
    vec![TurnToolActivity {
        id: format!("tool-{}", active_call.name.replace('.', "-")),
        name: active_call.name.to_string(),
        status: "running".to_string(),
        summary: "Tool call in progress.".to_string(),
        arguments_text: Some(active_call.arguments.to_string()),
        result_text: None,
        duration_seconds: None,
    }]
}

fn tool_activities_after_result(
    active_call: &ToolCall,
    result: &ToolResult,
) -> Vec<TurnToolActivity> {
    vec![TurnToolActivity {
        id: format!("tool-{}", active_call.name.replace('.', "-")),
        name: active_call.name.to_string(),
        status: if result.status == "ok" {
            "done".to_string()
        } else {
            "error".to_string()
        },
        summary: format!("Tool call finished with status: {}.", result.status),
        arguments_text: Some(active_call.arguments.to_string()),
        result_text: Some(result.output.clone()),
        duration_seconds: Some(result.duration_ms as f64 / 1000.0),
    }]
}
