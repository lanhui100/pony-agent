use crate::agent::graph::GraphEngine;
use crate::agent::config::ProviderRegistryStore;
use crate::agent::provider::{ProviderDecision, ProviderManager, ProviderMessage, ProviderRequest, TokenUsage};
use crate::agent::session::SessionState;
use crate::agent::tools::{builtin_tools, ToolCall, ToolRouter};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::rc::Rc;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub message: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    #[serde(default)]
    pub history: Vec<TurnHistoryMessage>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHistoryMessage {
    pub role: String,
    pub content: String,
}

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
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    pub phase: String,
    pub provider_requested_name: String,
    pub provider_name: String,
    pub provider_protocol: String,
    pub provider_model: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub user_message: String,
    pub assistant_message: String,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub session_summary: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStreamEvent {
    pub turn_id: String,
    pub kind: String,
    pub phase: Option<String>,
    pub text: Option<String>,
    pub error: Option<String>,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub trace_steps: Option<Vec<TurnTraceStep>>,
    pub tool_activities: Option<Vec<TurnToolActivity>>,
    pub session_summary: Option<String>,
}

pub struct AgentRuntime {
    graph: GraphEngine,
    session: SessionState,
}

fn emit_turn_failed(
    app: &AppHandle,
    turn_id: String,
    provider_requested_name: Option<String>,
    provider_name: Option<String>,
    provider_protocol: Option<String>,
    provider_model: Option<String>,
    error: String,
) {
    let _ = app.emit(
        "turn:failed",
        TurnStreamEvent {
            turn_id,
            kind: "failed".to_string(),
            phase: Some("failed".to_string()),
            text: Some("本轮执行失败，请查看右侧状态信息。".to_string()),
            error: Some(error),
            provider_requested_name,
            provider_name,
            provider_protocol,
            provider_model,
            provider_mode: None,
            fallback_reason: None,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            trace_steps: Some(vec![
                TurnTraceStep {
                    id: "step-plan".to_string(),
                    label: "接收输入".to_string(),
                    state: "completed".to_string(),
                },
                TurnTraceStep {
                    id: "step-context".to_string(),
                    label: "组织上下文".to_string(),
                    state: "completed".to_string(),
                },
                TurnTraceStep {
                    id: "step-call-model".to_string(),
                    label: "调用模型".to_string(),
                    state: "completed".to_string(),
                },
                TurnTraceStep {
                    id: "step-call-tool".to_string(),
                    label: "调用工具".to_string(),
                    state: "completed".to_string(),
                },
                TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "返回结果".to_string(),
                    state: "completed".to_string(),
                },
            ]),
            tool_activities: None,
            session_summary: None,
        },
    );
}

impl AgentRuntime {
    pub fn new() -> Self {
        Self {
            graph: GraphEngine::new("state-machine-v1"),
            session: SessionState {
                conversation_id: "local-dev-session".to_string(),
                summary: "Pony Agent 本地开发会话".to_string(),
            },
        }
    }

    pub fn name(&self) -> &'static str {
        "rust-core"
    }

    pub fn graph_engine(&self) -> &str {
        self.graph.name()
    }

    pub fn run_turn(&self, input: TurnInput) -> TurnResult {
        let normalized = input.message.trim();
        let user_message = if normalized.is_empty() {
            "请先输入一个想观察的 agent 问题。".to_string()
        } else {
            normalized.to_string()
        };

        let provider_store = ProviderRegistryStore::new();
        let provider = ProviderManager::new(provider_store.resolve_selection(
            input.provider_id.as_deref(),
            input.model_id.as_deref(),
        ));
        let tools = builtin_tools();
        let planning_request = self.build_planning_request(&provider, &user_message, &input.history);
        runtime_log(format!(
            "turn:run requested={} provider={} protocol={} model={} message_preview={}",
            provider.requested_name(),
            provider.name(),
            provider.protocol_label(),
            provider.model(),
            preview_text(&user_message, 120)
        ));
        let provider_decision = preflight_tool_decision(&user_message, &input.history)
            .unwrap_or_else(|| provider.decide_with_tools(&planning_request, &tools));
        runtime_log(format!(
            "turn:decision mode={} fallback_reason={} tool_call={}",
            provider_decision.provider_mode,
            provider_decision.fallback_reason.as_deref().unwrap_or("none"),
            provider_decision
                .tool_call
                .as_ref()
                .map(|call| call.name.as_str())
                .unwrap_or("none")
        ));
        let inferred_tool_call =
            self.resolve_tool_call(
                &user_message,
                &input.history,
                provider_decision.tool_call,
                &provider_decision.provider_mode,
            );
        let tool_router = ToolRouter::new();
        let mut assistant_message = provider_decision.output_text.clone();
        let mut provider_mode = provider_decision.provider_mode.clone();
        let mut fallback_reason = provider_decision.fallback_reason.clone();
        let mut token_usage = provider_decision.token_usage.clone();
        let (trace_steps, tool_activities) = if let Some(tool_call) = inferred_tool_call {
            runtime_log(format!("turn:tool-execute name={} args={}", tool_call.name, tool_call.arguments));
            let tool_result = tool_router.execute(&tool_call);
            runtime_log(format!(
                "turn:tool-result name={} status={} output_preview={}",
                tool_result.tool_name,
                tool_result.status,
                preview_text(&tool_result.output, 160)
            ));
            let final_response = provider.continue_with_tool_result(
                &planning_request,
                &tools,
                &tool_call,
                &tool_result,
            );
            assistant_message = final_response.output_text;
            provider_mode = final_response.provider_mode;
            fallback_reason = final_response.fallback_reason;
            token_usage = final_response.token_usage;
            (
                if tool_result.status == "ok" {
                    completed_trace_with_tool()
                } else {
                    completed_trace_with_tool_error()
                },
                tool_activities_after_result(&tools, &tool_call, &tool_result),
            )
        } else {
            (
                completed_trace_without_tool(),
                tool_activities_without_call(&tools),
            )
        };

        let session_summary = format!(
            "{} / graph={} / session={} / provider={} / mode={}",
            self.session.summary,
            self.graph.name(),
            self.session.conversation_id,
            provider.name(),
            provider_mode
        );
        let (input_tokens, output_tokens, total_tokens) = token_usage_parts(token_usage.as_ref());

        TurnResult {
            phase: "ready".to_string(),
            provider_requested_name: provider.requested_name().to_string(),
            provider_name: provider.name().to_string(),
            provider_protocol: provider.protocol_label().to_string(),
            provider_model: provider.model().to_string(),
            provider_mode,
            fallback_reason,
            input_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms: None,
            user_message,
            assistant_message,
            trace_steps,
            tool_activities,
            session_summary,
        }
    }

    pub fn start_turn_stream(&self, app: AppHandle, turn_id: String, input: TurnInput) {
        let user_message = input.message.trim().to_string();
        if user_message.is_empty() {
            emit_turn_failed(
                &app,
                turn_id,
                None,
                None,
                None,
                None,
                "消息为空，无法启动本轮执行。".to_string(),
            );
            return;
        }

        let provider_store = ProviderRegistryStore::new();
        let provider = ProviderManager::new(provider_store.resolve_selection(
            input.provider_id.as_deref(),
            input.model_id.as_deref(),
        ));
        let tools = builtin_tools();
        let turn_started_at = Instant::now();
        runtime_log(format!(
            "turn:start id={} requested={} provider={} protocol={} model={} message_preview={}",
            turn_id,
            provider.requested_name(),
            provider.name(),
            provider.protocol_label(),
            provider.model(),
            preview_text(&user_message, 120)
        ));

        let _ = app.emit(
            "turn:started",
            TurnStreamEvent {
                turn_id: turn_id.clone(),
                kind: "started".to_string(),
                phase: Some("calling_model".to_string()),
                text: Some(user_message.clone()),
                error: None,
                provider_requested_name: Some(provider.requested_name().to_string()),
                provider_name: Some(provider.name().to_string()),
                provider_protocol: Some(provider.protocol_label().to_string()),
                provider_model: Some(provider.model().to_string()),
                provider_mode: None,
                fallback_reason: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                trace_steps: Some(start_trace_steps()),
                tool_activities: Some(tool_catalog(&tools)),
                session_summary: None,
            },
        );

        let request = self.build_planning_request(&provider, &user_message, &input.history);
        let first_decision = preflight_tool_decision(&user_message, &input.history)
            .unwrap_or_else(|| provider.decide_with_tools(&request, &tools));
        runtime_log(format!(
            "turn:decision-stream id={} mode={} fallback_reason={} tool_call={}",
            turn_id,
            first_decision.provider_mode,
            first_decision.fallback_reason.as_deref().unwrap_or("none"),
            first_decision
                .tool_call
                .as_ref()
                .map(|call| call.name.as_str())
                .unwrap_or("none")
        ));
        let resolved_tool_call =
            self.resolve_tool_call(
                &user_message,
                &input.history,
                first_decision.tool_call,
                &first_decision.provider_mode,
            );

        if let Some(tool_call) = resolved_tool_call {
            runtime_log(format!("turn:tool-execute-stream id={} name={} args={}", turn_id, tool_call.name, tool_call.arguments));
            let _ = app.emit(
                "turn:trace",
                TurnStreamEvent {
                    turn_id: turn_id.clone(),
                    kind: "trace".to_string(),
                    phase: Some("calling_tool".to_string()),
                    text: None,
                    error: None,
                    provider_requested_name: None,
                    provider_name: None,
                    provider_protocol: None,
                    provider_model: None,
                    provider_mode: None,
                    fallback_reason: None,
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms: None,
                    trace_steps: Some(trace_tool_active()),
                    tool_activities: None,
                    session_summary: None,
                },
            );

            let _ = app.emit(
                "turn:tool",
                TurnStreamEvent {
                    turn_id: turn_id.clone(),
                    kind: "tool".to_string(),
                    phase: Some("calling_tool".to_string()),
                    text: None,
                    error: None,
                    provider_requested_name: None,
                    provider_name: None,
                    provider_protocol: None,
                    provider_model: None,
                    provider_mode: None,
                    fallback_reason: None,
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms: None,
                    trace_steps: None,
                    tool_activities: Some(tool_activities_running(&tools, &tool_call)),
                    session_summary: None,
                },
            );

            let tool_router = ToolRouter::new();
            let tool_result = tool_router.execute(&tool_call);
            runtime_log(format!(
                "turn:tool-result-stream id={} name={} status={} output_preview={}",
                turn_id,
                tool_result.tool_name,
                tool_result.status,
                preview_text(&tool_result.output, 160)
            ));

            let _ = app.emit(
                "turn:tool",
                TurnStreamEvent {
                    turn_id: turn_id.clone(),
                    kind: "tool".to_string(),
                    phase: Some("calling_model".to_string()),
                    text: None,
                    error: None,
                    provider_requested_name: None,
                    provider_name: None,
                    provider_protocol: None,
                    provider_model: None,
                    provider_mode: None,
                    fallback_reason: None,
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms: None,
                    trace_steps: None,
                    tool_activities: Some(tool_activities_after_result(&tools, &tool_call, &tool_result)),
                    session_summary: None,
                },
            );

            let _ = app.emit(
                "turn:trace",
                TurnStreamEvent {
                    turn_id: turn_id.clone(),
                    kind: "trace".to_string(),
                    phase: Some("calling_model".to_string()),
                    text: None,
                    error: None,
                    provider_requested_name: None,
                    provider_name: None,
                    provider_protocol: None,
                    provider_model: None,
                    provider_mode: None,
                    fallback_reason: None,
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms: None,
                    trace_steps: Some(if tool_result.status == "ok" {
                        trace_return_active()
                    } else {
                        trace_return_active_after_tool_error()
                    }),
                    tool_activities: None,
                    session_summary: None,
                },
            );

            let delta_app = app.clone();
            let delta_turn_id = turn_id.clone();
            let first_token_latency = Rc::new(Cell::new(None));
            let first_token_latency_for_emit = first_token_latency.clone();
            let final_response = provider.continue_with_tool_result_stream(
                &request,
                &tools,
                &tool_call,
                &tool_result,
                move |delta| {
                    let latency = if first_token_latency_for_emit.get().is_none() {
                        let latency = turn_started_at.elapsed().as_millis() as u64;
                        first_token_latency_for_emit.set(Some(latency));
                        Some(latency)
                    } else {
                        None
                    };
                    let _ = delta_app.emit(
                        "turn:delta",
                        TurnStreamEvent {
                            turn_id: delta_turn_id.clone(),
                            kind: "delta".to_string(),
                            phase: Some("calling_model".to_string()),
                            text: Some(delta),
                            error: None,
                            provider_requested_name: None,
                            provider_name: None,
                            provider_protocol: None,
                            provider_model: None,
                            provider_mode: None,
                            fallback_reason: None,
                            input_tokens: None,
                            output_tokens: None,
                            total_tokens: None,
                            first_token_latency_ms: latency,
                            trace_steps: None,
                            tool_activities: None,
                            session_summary: None,
                        },
                    );
                },
            );

            let session_summary = format!(
                "{} / graph={} / session={} / provider={} / mode={}",
                self.session.summary,
                self.graph.name(),
                self.session.conversation_id,
                provider.name(),
                final_response.provider_mode
            );
            runtime_log(format!(
                "turn:completed-stream id={} mode={} fallback_reason={}",
                turn_id,
                final_response.provider_mode,
                final_response
                    .fallback_reason
                    .as_deref()
                    .or(first_decision.fallback_reason.as_deref())
                    .unwrap_or("none")
            ));
            let (input_tokens, output_tokens, total_tokens) =
                token_usage_parts(final_response.token_usage.as_ref());

            let _ = app.emit(
                "turn:completed",
                TurnStreamEvent {
                    turn_id,
                    kind: "completed".to_string(),
                    phase: Some("ready".to_string()),
                    text: Some(final_response.output_text),
                    error: None,
                    provider_requested_name: Some(provider.requested_name().to_string()),
                    provider_name: Some(provider.name().to_string()),
                    provider_protocol: Some(provider.protocol_label().to_string()),
                    provider_model: Some(provider.model().to_string()),
                    provider_mode: Some(final_response.provider_mode),
                    fallback_reason: final_response.fallback_reason.or(first_decision.fallback_reason),
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    first_token_latency_ms: first_token_latency.get(),
                    trace_steps: Some(if tool_result.status == "ok" {
                        completed_trace_with_tool()
                    } else {
                        completed_trace_with_tool_error()
                    }),
                    tool_activities: Some(tool_activities_after_result(&tools, &tool_call, &tool_result)),
                    session_summary: Some(session_summary),
                },
            );
            return;
        }
        runtime_log(format!("turn:no-tool-stream id={} mode={}", turn_id, first_decision.provider_mode));

        let _ = app.emit(
            "turn:trace",
            TurnStreamEvent {
                turn_id: turn_id.clone(),
                kind: "trace".to_string(),
                phase: Some("calling_model".to_string()),
                text: None,
                error: None,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_mode: None,
                fallback_reason: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                trace_steps: Some(trace_return_active_without_tool()),
                tool_activities: None,
                session_summary: None,
            },
        );

        let no_tool_activities = tool_activities_without_call(&tools);
        let _ = app.emit(
            "turn:tool",
            TurnStreamEvent {
                turn_id: turn_id.clone(),
                kind: "tool".to_string(),
                phase: Some("calling_model".to_string()),
                text: None,
                error: None,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_mode: None,
                fallback_reason: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                trace_steps: None,
                tool_activities: Some(no_tool_activities.clone()),
                session_summary: None,
            },
        );

        let first_token_latency_ms =
            stream_text_chunks(&app, &turn_id, "calling_model", &first_decision.output_text, &turn_started_at);

        let session_summary = format!(
            "{} / graph={} / session={} / provider={} / mode={}",
            self.session.summary,
            self.graph.name(),
            self.session.conversation_id,
            provider.name(),
            first_decision.provider_mode
        );
        let (input_tokens, output_tokens, total_tokens) =
            token_usage_parts(first_decision.token_usage.as_ref());

        let _ = app.emit(
            "turn:completed",
            TurnStreamEvent {
                turn_id,
                kind: "completed".to_string(),
                phase: Some("ready".to_string()),
                text: Some(first_decision.output_text),
                error: None,
                provider_requested_name: Some(provider.requested_name().to_string()),
                provider_name: Some(provider.name().to_string()),
                provider_protocol: Some(provider.protocol_label().to_string()),
                provider_model: Some(provider.model().to_string()),
                provider_mode: Some(first_decision.provider_mode),
                fallback_reason: first_decision.fallback_reason,
                input_tokens,
                output_tokens,
                total_tokens,
                first_token_latency_ms,
                trace_steps: Some(completed_trace_without_tool()),
                tool_activities: Some(no_tool_activities),
                session_summary: Some(session_summary),
            },
        );
    }

    fn build_planning_request(
        &self,
        provider: &ProviderManager,
        user_message: &str,
        history: &[TurnHistoryMessage],
    ) -> ProviderRequest {
        let mut input = vec![
            ProviderMessage::system(
                "你是 Pony Agent 的学习模式助手。请始终使用中文回答，并在需要时通过 provider 的原生工具调用能力请求工具。",
            ),
            ProviderMessage::developer(
                "当前阶段是 planning/decision 阶段，不是最终回答阶段。你的职责只有两种：1. 如果需要访问工作区、目录、文件内容、代码配置，请优先调用工具；2. 如果确实不需要工具，只返回一句非常简短的中文结论，不要展开解释，不要写长段落，不要给示例。凡是用户提到具体文件名、配置项、文件内容、目录内容，默认优先调用 workspace.* 工具。".to_string(),
            ),
            ProviderMessage::developer(format!(
                "当前会话摘要：{} / graph={} / session={}",
                self.session.summary,
                self.graph.name(),
                self.session.conversation_id
            )),
        ];

        for message in history.iter().filter_map(to_provider_history_message) {
            input.push(message);
        }

        input.push(ProviderMessage::user(user_message.to_string()));

        ProviderRequest {
            model: provider.model().to_string(),
            input,
            temperature: provider.temperature(),
            max_output_tokens: provider.max_output_tokens(),
        }
    }

    fn resolve_tool_call(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        provider_tool_call: Option<ToolCall>,
        provider_mode: &str,
    ) -> Option<ToolCall> {
        let inferred = infer_local_tool_call(user_message, history, provider_mode);
        match (provider_tool_call, inferred) {
            (Some(provider_call), Some(inferred_call))
                if should_prefer_inferred_tool_call(&provider_call, &inferred_call) =>
            {
                Some(inferred_call)
            }
            (Some(provider_call), _) => Some(provider_call),
            (None, inferred_call) => inferred_call,
        }
    }
}

fn infer_local_tool_call(
    user_message: &str,
    history: &[TurnHistoryMessage],
    provider_mode: &str,
) -> Option<ToolCall> {
    if let Some(tool_call) = infer_workspace_tool_call(user_message, history) {
        return Some(tool_call);
    }

    if provider_mode != "mock" {
        return None;
    }

    let lowered = user_message.to_lowercase();
    if lowered.contains("时间") || lowered.contains("几点") || lowered.contains("timestamp") || lowered.contains("time") {
        return Some(ToolCall {
            call_id: None,
            name: "time.now".to_string(),
            arguments: serde_json::json!({}),
        });
    }

    if lowered.contains("回显") || lowered.contains("echo") {
        return Some(ToolCall {
            call_id: None,
            name: "echo.input".to_string(),
            arguments: serde_json::json!({ "text": user_message }),
        });
    }

    None
}

fn preflight_tool_decision(user_message: &str, history: &[TurnHistoryMessage]) -> Option<ProviderDecision> {
    let tool_call = infer_workspace_tool_call(user_message, history)?;
    Some(ProviderDecision {
        output_text: String::new(),
        tool_call: Some(tool_call),
        provider_mode: "local-planner".to_string(),
        fallback_reason: None,
        token_usage: None,
    })
}

fn infer_workspace_tool_call(user_message: &str, history: &[TurnHistoryMessage]) -> Option<ToolCall> {
    let lowered = user_message.to_lowercase();

    if lowered.contains("当前文件夹")
        || lowered.contains("当前目录")
        || lowered.contains("有哪些文件")
        || lowered.contains("列出文件")
        || lowered.contains("目录里")
    {
        return Some(ToolCall {
            call_id: None,
            name: "workspace.list_files".to_string(),
            arguments: serde_json::json!({
                "path": ".",
                "limit": 80
            }),
        });
    }

    let file_name = extract_explicit_file_name(user_message)
        .or_else(|| infer_last_referenced_file(history))?;
    let resolved_path = find_workspace_file(&file_name)
        .unwrap_or_else(|| file_name.replace('\\', "/"));

    let line_request = extract_line_request(user_message);
    let asks_for_segment = line_request.is_some()
        || lowered.contains("哪几行")
        || lowered.contains("片段")
        || lowered.contains("一段")
        || lowered.contains("局部");

    if asks_for_segment {
        let (start_line, line_count) = line_request.unwrap_or((1, 120));
        return Some(ToolCall {
            call_id: None,
            name: "workspace.read_file_segment".to_string(),
            arguments: serde_json::json!({
                "path": resolved_path,
                "startLine": start_line,
                "lineCount": line_count
            }),
        });
    }

    if lowered.contains("配置")
        || lowered.contains("内容")
        || lowered.contains("是什么文件")
        || lowered.contains("这是什么文件")
        || lowered.contains("查看")
        || lowered.contains("读取")
        || lowered.contains("打开")
        || lowered.contains("讲解")
        || lowered.contains("解释")
    {
        return Some(ToolCall {
            call_id: None,
            name: "workspace.read_file".to_string(),
            arguments: serde_json::json!({
                "path": resolved_path
            }),
        });
    }

    None
}

fn to_provider_history_message(message: &TurnHistoryMessage) -> Option<ProviderMessage> {
    let content = message.content.trim();
    if content.is_empty() {
        return None;
    }

    match message.role.as_str() {
        "user" => Some(ProviderMessage::user(content.to_string())),
        "assistant" => Some(ProviderMessage {
            role: crate::agent::provider::ProviderRole::Developer,
            content: format!("上一轮 assistant 回复：{}", content),
        }),
        _ => None,
    }
}

fn extract_explicit_file_name(user_message: &str) -> Option<String> {
    let mut candidates = Vec::new();
    let mut current = String::new();

    for ch in user_message.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\') {
            current.push(ch);
        } else if !current.is_empty() {
            candidates.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        candidates.push(current);
    }

    candidates
        .into_iter()
        .map(|segment| segment.trim_matches(|ch: char| ch == '`' || ch == '.' || ch == '!').to_string())
        .find(|segment| {
            !segment.is_empty()
                && segment.contains('.')
                && !segment.starts_with("http://")
                && !segment.starts_with("https://")
                && segment
                    .rsplit('.')
                    .next()
                    .map(|ext| !ext.is_empty() && ext.chars().all(|ch| ch.is_ascii_alphanumeric()))
                    .unwrap_or(false)
        })
}

fn find_workspace_file(file_name: &str) -> Option<String> {
    let root = std::env::current_dir().ok()?;
    let normalized_target = file_name.replace('\\', "/");
    let mut stack = vec![(root.clone(), 0usize)];
    let mut visited = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        if visited >= 2_000 || depth > 4 {
            break;
        }
        visited += 1;

        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }

            let matches = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case(&normalized_target))
                .unwrap_or(false);

            if matches {
                return path
                    .strip_prefix(&root)
                    .ok()
                    .map(path_to_unix_string);
            }
        }
    }

    None
}

fn path_to_unix_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn infer_last_referenced_file(history: &[TurnHistoryMessage]) -> Option<String> {
    history
        .iter()
        .rev()
        .filter(|message| message.role == "user")
        .filter_map(|message| extract_explicit_file_name(&message.content))
        .next()
}

fn extract_line_request(user_message: &str) -> Option<(u64, u64)> {
    let marker = "第";
    let suffix = "行";
    let start = user_message.find(marker)?;
    let rest = &user_message[start + marker.len()..];
    let end = rest.find(suffix)?;
    let digits: String = rest[..end].chars().filter(|ch| ch.is_ascii_digit()).collect();
    let line = digits.parse::<u64>().ok()?;
    let start_line = line.saturating_sub(2).max(1);
    Some((start_line, 5))
}

fn should_prefer_inferred_tool_call(provider_call: &ToolCall, inferred_call: &ToolCall) -> bool {
    provider_call.name == "workspace.list_files"
        && matches!(
            inferred_call.name.as_str(),
            "workspace.read_file" | "workspace.read_file_segment"
        )
}

fn start_trace_steps() -> Vec<TurnTraceStep> {
    vec![
        TurnTraceStep {
            id: "step-plan".to_string(),
            label: "接收输入".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-context".to_string(),
            label: "组织上下文".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-model".to_string(),
            label: "调用模型".to_string(),
            state: "active".to_string(),
        },
        TurnTraceStep {
            id: "step-call-tool".to_string(),
            label: "调用工具".to_string(),
            state: "pending".to_string(),
        },
        TurnTraceStep {
            id: "step-return".to_string(),
            label: "返回结果".to_string(),
            state: "pending".to_string(),
        },
    ]
}

fn trace_tool_active() -> Vec<TurnTraceStep> {
    vec![
        TurnTraceStep {
            id: "step-plan".to_string(),
            label: "接收输入".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-context".to_string(),
            label: "组织上下文".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-model".to_string(),
            label: "调用模型".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-tool".to_string(),
            label: "调用工具".to_string(),
            state: "active".to_string(),
        },
        TurnTraceStep {
            id: "step-return".to_string(),
            label: "返回结果".to_string(),
            state: "pending".to_string(),
        },
    ]
}

fn trace_return_active() -> Vec<TurnTraceStep> {
    vec![
        TurnTraceStep {
            id: "step-plan".to_string(),
            label: "接收输入".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-context".to_string(),
            label: "组织上下文".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-model".to_string(),
            label: "调用模型".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-tool".to_string(),
            label: "调用工具".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-return".to_string(),
            label: "返回结果".to_string(),
            state: "active".to_string(),
        },
    ]
}

fn trace_return_active_after_tool_error() -> Vec<TurnTraceStep> {
    vec![
        TurnTraceStep {
            id: "step-plan".to_string(),
            label: "接收输入".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-context".to_string(),
            label: "组织上下文".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-model".to_string(),
            label: "调用模型".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-tool".to_string(),
            label: "调用工具".to_string(),
            state: "error".to_string(),
        },
        TurnTraceStep {
            id: "step-return".to_string(),
            label: "返回结果".to_string(),
            state: "active".to_string(),
        },
    ]
}

fn trace_return_active_without_tool() -> Vec<TurnTraceStep> {
    vec![
        TurnTraceStep {
            id: "step-plan".to_string(),
            label: "接收输入".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-context".to_string(),
            label: "组织上下文".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-model".to_string(),
            label: "调用模型".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-tool".to_string(),
            label: "调用工具".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-return".to_string(),
            label: "返回结果".to_string(),
            state: "active".to_string(),
        },
    ]
}

fn completed_trace_with_tool() -> Vec<TurnTraceStep> {
    trace_completed()
}

fn completed_trace_with_tool_error() -> Vec<TurnTraceStep> {
    vec![
        TurnTraceStep {
            id: "step-plan".to_string(),
            label: "接收输入".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-context".to_string(),
            label: "组织上下文".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-model".to_string(),
            label: "调用模型".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-tool".to_string(),
            label: "调用工具".to_string(),
            state: "error".to_string(),
        },
        TurnTraceStep {
            id: "step-return".to_string(),
            label: "返回结果".to_string(),
            state: "completed".to_string(),
        },
    ]
}

fn completed_trace_without_tool() -> Vec<TurnTraceStep> {
    trace_completed()
}

fn trace_completed() -> Vec<TurnTraceStep> {
    vec![
        TurnTraceStep {
            id: "step-plan".to_string(),
            label: "接收输入".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-context".to_string(),
            label: "组织上下文".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-model".to_string(),
            label: "调用模型".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-call-tool".to_string(),
            label: "调用工具".to_string(),
            state: "completed".to_string(),
        },
        TurnTraceStep {
            id: "step-return".to_string(),
            label: "返回结果".to_string(),
            state: "completed".to_string(),
        },
    ]
}

fn tool_catalog(tools: &[crate::agent::tools::ToolDefinition]) -> Vec<TurnToolActivity> {
    tools
        .iter()
        .map(|tool| TurnToolActivity {
            id: format!("tool-{}", tool.name.replace('.', "-")),
            name: tool.name.to_string(),
            status: "planned".to_string(),
            summary: format!("已注册工具：{}", tool.description),
        })
        .collect()
}

fn tool_activities_running(
    tools: &[crate::agent::tools::ToolDefinition],
    active_call: &ToolCall,
) -> Vec<TurnToolActivity> {
    tools
        .iter()
        .map(|tool| {
            if tool.name == active_call.name {
                TurnToolActivity {
                    id: format!("tool-{}", tool.name.replace('.', "-")),
                    name: tool.name.to_string(),
                    status: "running".to_string(),
                    summary: format!("当前回合正在执行该工具，参数：{}", active_call.arguments),
                }
            } else {
                TurnToolActivity {
                    id: format!("tool-{}", tool.name.replace('.', "-")),
                    name: tool.name.to_string(),
                    status: "planned".to_string(),
                    summary: format!("已注册工具：{}", tool.description),
                }
            }
        })
        .collect()
}

fn tool_activities_after_result(
    tools: &[crate::agent::tools::ToolDefinition],
    active_call: &ToolCall,
    result: &crate::agent::tools::ToolResult,
) -> Vec<TurnToolActivity> {
    tools
        .iter()
        .map(|tool| {
            if tool.name == active_call.name {
                TurnToolActivity {
                    id: format!("tool-{}", tool.name.replace('.', "-")),
                    name: tool.name.to_string(),
                    status: if result.status == "ok" {
                        "done".to_string()
                    } else {
                        "error".to_string()
                    },
                    summary: format!("工具状态：{}。结果：{}", result.status, result.output),
                }
            } else {
                TurnToolActivity {
                    id: format!("tool-{}", tool.name.replace('.', "-")),
                    name: tool.name.to_string(),
                    status: "planned".to_string(),
                    summary: format!("已注册工具：{}", tool.description),
                }
            }
        })
        .collect()
}

fn tool_activities_without_call(tools: &[crate::agent::tools::ToolDefinition]) -> Vec<TurnToolActivity> {
    tools
        .iter()
        .map(|tool| TurnToolActivity {
            id: format!("tool-{}", tool.name.replace('.', "-")),
            name: tool.name.to_string(),
            status: "planned".to_string(),
            summary: format!("本轮未触发该工具：{}", tool.description),
        })
        .collect()
}

fn token_usage_parts(token_usage: Option<&TokenUsage>) -> (Option<u64>, Option<u64>, Option<u64>) {
    match token_usage {
        Some(token_usage) => (
            token_usage.input_tokens,
            token_usage.output_tokens,
            token_usage.total_tokens,
        ),
        None => (None, None, None),
    }
}

fn stream_text_chunks(
    app: &AppHandle,
    turn_id: &str,
    phase: &str,
    text: &str,
    started_at: &Instant,
) -> Option<u64> {
    let mut first_token_latency_ms = None;

    for chunk in text.as_bytes().chunks(48) {
        let delta = String::from_utf8_lossy(chunk).to_string();
        let latency = if first_token_latency_ms.is_none() {
            let latency = started_at.elapsed().as_millis() as u64;
            first_token_latency_ms = Some(latency);
            Some(latency)
        } else {
            None
        };
        let _ = app.emit(
            "turn:delta",
            TurnStreamEvent {
                turn_id: turn_id.to_string(),
                kind: "delta".to_string(),
                phase: Some(phase.to_string()),
                text: Some(delta),
                error: None,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_mode: None,
                fallback_reason: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: latency,
                trace_steps: None,
                tool_activities: None,
                session_summary: None,
            },
        );
    }

    first_token_latency_ms
}

fn runtime_log(message: String) {
    eprintln!("[pony-runtime] {}", message);
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let count = normalized.chars().count();
    if count <= max_chars {
        normalized
    } else {
        let preview = normalized.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}
