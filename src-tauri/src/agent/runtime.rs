use crate::agent::graph::GraphEngine;
use crate::agent::config::ProviderRegistryStore;
use crate::agent::provider::{ProviderManager, ProviderMessage, ProviderRequest};
use crate::agent::session::SessionState;
use crate::agent::tools::builtin_tools;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub message: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
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

        let trace_steps = vec![
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
                id: "step-return".to_string(),
                label: "返回结果".to_string(),
                state: "completed".to_string(),
            },
        ];

        let tool_activities = vec![
            TurnToolActivity {
                id: "tool-terminal".to_string(),
                name: tools[0].name.to_string(),
                status: "planned".to_string(),
                summary: format!("已注册工具：{}。下一阶段会把真实工具执行接回运行时。", tools[0].description),
            },
            TurnToolActivity {
                id: "tool-mcp".to_string(),
                name: tools[2].name.to_string(),
                status: "planned".to_string(),
                summary: format!("已预留能力：{}。当前这次迁移先聚焦真实模型与 mock 诊断。", tools[2].description),
            },
        ];

        let provider_response = provider.send(&ProviderRequest {
            model: provider.model().to_string(),
            input: vec![
                ProviderMessage::system(
                    "你是 Pony Agent 的学习模式助手。请使用中文，解释清楚当前这轮执行结果。",
                ),
                ProviderMessage::developer(format!(
                    "当前会话摘要：{} / graph={} / session={}",
                    self.session.summary,
                    self.graph.name(),
                    self.session.conversation_id
                )),
                ProviderMessage::user(user_message.clone()),
            ],
            temperature: provider.temperature(),
            max_output_tokens: provider.max_output_tokens(),
        });

        let session_summary = format!(
            "{} / graph={} / session={} / provider={} / mode={}",
            self.session.summary,
            self.graph.name(),
            self.session.conversation_id,
            provider.name(),
            provider_response.provider_mode
        );

        TurnResult {
            phase: "ready".to_string(),
            provider_requested_name: provider.requested_name().to_string(),
            provider_name: provider.name().to_string(),
            provider_protocol: provider.protocol_label().to_string(),
            provider_model: provider.model().to_string(),
            provider_mode: provider_response.provider_mode,
            fallback_reason: provider_response.fallback_reason,
            user_message,
            assistant_message: provider_response.output_text,
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
        let terminal_tool_name = tools[0].name.to_string();
        let terminal_tool_description = tools[0].description.to_string();
        let mcp_tool_name = tools[2].name.to_string();
        let mcp_tool_description = tools[2].description.to_string();

        let start_trace_steps = vec![
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
                id: "step-return".to_string(),
                label: "返回结果".to_string(),
                state: "pending".to_string(),
            },
        ];

        let tool_activities = vec![
            TurnToolActivity {
                id: "tool-terminal".to_string(),
                name: terminal_tool_name.clone(),
                status: "planned".to_string(),
                summary: format!("已注册工具：{}。当前回合尚未触发本地工具调用。", terminal_tool_description),
            },
            TurnToolActivity {
                id: "tool-mcp".to_string(),
                name: mcp_tool_name.clone(),
                status: "planned".to_string(),
                summary: format!("已预留能力：{}。当前先聚焦模型回包链路。", mcp_tool_description),
            },
        ];

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
                trace_steps: Some(start_trace_steps),
                tool_activities: Some(tool_activities),
                session_summary: None,
            },
        );

        let request = ProviderRequest {
            model: provider.model().to_string(),
            input: vec![
                ProviderMessage::system(
                    "你是 Pony Agent 的学习模式助手。请使用中文，解释清楚当前这轮执行结果。",
                ),
                ProviderMessage::developer(format!(
                    "当前会话摘要：{} / graph={} / session={}",
                    self.session.summary,
                    self.graph.name(),
                    self.session.conversation_id
                )),
                ProviderMessage::user(user_message.clone()),
            ],
            temperature: provider.temperature(),
            max_output_tokens: provider.max_output_tokens(),
        };

        let delta_app = app.clone();
        let delta_turn_id = turn_id.clone();
        let delta_terminal_tool_name = terminal_tool_name.clone();
        let delta_terminal_tool_description = terminal_tool_description.clone();
        let delta_mcp_tool_name = mcp_tool_name.clone();
        let first_delta = Arc::new(AtomicBool::new(false));
        let first_delta_flag = first_delta.clone();
        let response = provider.send_stream(&request, move |delta| {
            if !first_delta_flag.swap(true, Ordering::SeqCst) {
                let _ = delta_app.emit(
                    "turn:trace",
                    TurnStreamEvent {
                        turn_id: delta_turn_id.clone(),
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
                                id: "step-return".to_string(),
                                label: "返回结果".to_string(),
                                state: "active".to_string(),
                            },
                        ]),
                        tool_activities: None,
                        session_summary: None,
                    },
                );

                let _ = delta_app.emit(
                    "turn:tool",
                    TurnStreamEvent {
                        turn_id: delta_turn_id.clone(),
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
                        trace_steps: None,
                        tool_activities: Some(vec![
                            TurnToolActivity {
                                id: "tool-terminal".to_string(),
                                name: delta_terminal_tool_name.clone(),
                                status: "planned".to_string(),
                                summary: format!("已注册工具：{}。当前回合尚未触发本地工具调用。", delta_terminal_tool_description),
                            },
                            TurnToolActivity {
                                id: "tool-mcp".to_string(),
                                name: delta_mcp_tool_name.clone(),
                                status: "running".to_string(),
                                summary: "模型正在流式返回内容，当前回合处于增量输出中。".to_string(),
                            },
                        ]),
                        session_summary: None,
                    },
                );
            }

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
                    trace_steps: None,
                    tool_activities: None,
                    session_summary: None,
                },
            );
        });

        let completed_trace_steps = vec![
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
                id: "step-return".to_string(),
                label: "返回结果".to_string(),
                state: "completed".to_string(),
            },
        ];

        let session_summary = format!(
            "{} / graph={} / session={} / provider={} / mode={}",
            self.session.summary,
            self.graph.name(),
            self.session.conversation_id,
            provider.name(),
            response.provider_mode
        );

        let _ = app.emit(
            "turn:completed",
            TurnStreamEvent {
                turn_id,
                kind: "completed".to_string(),
                phase: Some("ready".to_string()),
                text: Some(response.output_text),
                error: None,
                provider_requested_name: Some(provider.requested_name().to_string()),
                provider_name: Some(provider.name().to_string()),
                provider_protocol: Some(provider.protocol_label().to_string()),
                provider_model: Some(provider.model().to_string()),
                provider_mode: Some(response.provider_mode),
                fallback_reason: response.fallback_reason,
                trace_steps: Some(completed_trace_steps),
                tool_activities: Some(vec![
                    TurnToolActivity {
                        id: "tool-terminal".to_string(),
                        name: terminal_tool_name,
                        status: "planned".to_string(),
                        summary: format!("已注册工具：{}。下一阶段会把真实工具执行接回运行时。", terminal_tool_description),
                    },
                    TurnToolActivity {
                        id: "tool-mcp".to_string(),
                        name: mcp_tool_name,
                        status: "done".to_string(),
                        summary: "本轮模型输出已完成，当前流式回包链路已结束。".to_string(),
                    },
                ]),
                session_summary: Some(session_summary),
            },
        );
    }
}
