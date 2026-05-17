use crate::agent::graph::GraphEngine;
use crate::agent::provider::{ProviderManager, ProviderMessage, ProviderRequest};
use crate::agent::session::SessionState;
use crate::agent::tools::builtin_tools;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub message: String,
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

pub struct AgentRuntime {
    graph: GraphEngine,
    session: SessionState,
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

        let provider = ProviderManager::from_env();
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
}
