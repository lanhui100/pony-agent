use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub filename: Option<String>,
    pub function: Option<String>,
    pub lineno: Option<u32>,
    pub in_app: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exception {
    pub error_type: String,
    pub value: Option<String>,
    pub stacktrace: Option<Vec<Frame>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    pub category: String,
    pub message: String,
    pub data: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPayload {
    pub platform: String,
    pub release: String,
    pub environment: String,
    pub message: Option<String>,
    pub exception: Option<Exception>,
    pub tags: Option<HashMap<String, String>>,
    pub extra: Option<serde_json::Value>,
    pub breadcrumbs: Option<Vec<Breadcrumb>>,
}

/// Agent Trace 评估标注与生命周期状态机
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EvalStatus {
    #[default]
    Unreviewed,
    TriageGood,
    TriageBad,
    EvalDataset,
    Optimized,
    Wontfix,
}

impl std::fmt::Display for EvalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreviewed => write!(f, "unreviewed"),
            Self::TriageGood => write!(f, "triage_good"),
            Self::TriageBad => write!(f, "triage_bad"),
            Self::EvalDataset => write!(f, "eval_dataset"),
            Self::Optimized => write!(f, "optimized"),
            Self::Wontfix => write!(f, "wontfix"),
        }
    }
}

/// 单次工具调用遥测摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallTraceItem {
    pub call_id: Option<String>,
    pub tool_name: String,
    pub arguments_summary: Option<String>,
    pub status: String,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

/// Turn 级 Trace 明细条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnTraceItem {
    pub turn_id: String,
    pub sequence: Option<u64>,
    pub phase: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallTraceItem>,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
}

/// 上报至 PonySentry 的 Agent 运行完整链路 Trace Payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTracePayload {
    pub session_id: String,
    pub run_id: Option<String>,
    pub turn_id: Option<String>,
    pub environment: String,
    pub release: String,
    pub eval_status: EvalStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<TurnTraceItem>,
    pub tags: Option<HashMap<String, String>>,
    pub extra: Option<serde_json::Value>,
    pub total_input_tokens: Option<u64>,
    pub total_output_tokens: Option<u64>,
    pub total_duration_ms: Option<u64>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub stats_incomplete: bool,
    #[serde(default)]
    pub wall_clock_ms: Option<u64>,
    #[serde(default)]
    pub inter_turn_pause_ms: Option<u64>,
    pub reported_at_ms: u64,
}

