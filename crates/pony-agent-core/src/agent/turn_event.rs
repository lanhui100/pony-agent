// turn_event: turn 内过程事实的事件类型定义（事件溯源演进阶段 1，PA-091）。
// 事件是 append-only 事实层，快照/投影是派生视图。设计见
// openspec/changes/turn-event-log/design.md（对抗审核后定稿）。
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::provider::TokenUsage;
use crate::agent::session::AttachmentReference;
use crate::agent::telemetry::{
    CapabilityInvocationRecord, ProviderLatencyKind, ProviderRequestKind,
};

/// 事件 schema 版本：结构变更才 bump，新增事件类型不 bump（借鉴 dsh SESSION_FORMAT_VERSION）。
pub const EVENT_SCHEMA_VERSION: u64 = 1;

/// turn 结束原因（turn/end 事件的 reason）。
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndReason {
    #[default]
    Completed,
    Error,
    Aborted,
    Cancelled,
}

/// 一条 turn 内过程事件（append-only，落盘后不可变）。
///
/// 事件携带 `branch_id`（阶段 3 起分支可见性推导的基石，阶段 1 默认 "main"）。
/// 构造/反序列化失败 fail loud（数据完整性错误）；持久化写入失败 contained
/// （IO 错误，只记日志不阻断流）——两个失败面由调用方分层处理。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    TurnStart {
        turn_id: String,
    },
    TurnEnd {
        turn_id: String,
        reason: TurnEndReason,
        turn_duration_ms: Option<u64>,
    },
    StepStart {
        turn_id: String,
        step: u32,
        first_token_latency_ms: Option<u64>,
    },
    StepEnd {
        turn_id: String,
        step: u32,
    },
    UserMessage {
        turn_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<AttachmentReference>,
    },
    AssistantChunk {
        turn_id: String,
        step: u32,
        text: String,
    },
    AssistantMessage {
        turn_id: String,
        step: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        /// 仅消息元数据展示，不参与 MetricsProjection totals（防双计数）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
        /// 回填/降级标记：过程 chunk 不可恢复时置 true。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chunk_missing: Option<bool>,
    },
    ToolCall {
        turn_id: String,
        step: u32,
        call_id: String,
        name: String,
        arguments: String,
        /// TurnToolActivity.duration_seconds 重建所需。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        started_at_ms: Option<u64>,
    },
    ToolResult {
        turn_id: String,
        step: u32,
        call_id: String,
        /// 32KB 截断在投影层做，事件层存引用或截断标记。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifacts: Option<Vec<Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_invocation: Option<CapabilityInvocationRecord>,
    },
    PlanUpdate {
        plan_state: Value,
    },
    /// 每次 provider 调用结算（ProviderCallCacheRecord 的事件化）。
    /// 缓存命中（cache_hit/cache_miss）只有 provider 结算时才知道，必须进事件，
    /// 否则 ADR 0007 的缓存命中一等指标无法从事件重建。
    ProviderUsage {
        turn_id: String,
        /// 初始请求=0，每次 tool followup +1（MetricsProjection addReplacing 的唯一键）。
        step: u32,
        request_kind: ProviderRequestKind,
        usage: TokenUsage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_hit_input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_miss_input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prefix_mutation_reasons: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first_token_latency_ms: Option<u64>,
        /// 本次调用耗时（非 turn 耗时；turn 耗时以 turn/end 为准）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_duration_ms: Option<u64>,
        latency_kind: ProviderLatencyKind,
        provider: String,
        model: String,
    },
    /// 上下文压缩：投影丢弃 base_seq 之前的消息事件并注入摘要
    /// （折叠不复活被压缩历史）。
    HistorySquash {
        base_seq: u64,
        summary_message: String,
    },
    CheckpointCreated {
        node_id: String,
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_node_id: Option<String>,
    },
    CheckpointCheckout {
        node_id: String,
        mode: String,
    },
    ForkCreated {
        branch_id: String,
        from_node_id: String,
    },
}

impl TurnEvent {
    /// 事件类型名（`domain/action` 格式，与 `event_type` 列一致）。
    pub fn type_name(&self) -> &'static str {
        match self {
            TurnEvent::TurnStart { .. } => "turn/start",
            TurnEvent::TurnEnd { .. } => "turn/end",
            TurnEvent::StepStart { .. } => "step/start",
            TurnEvent::StepEnd { .. } => "step/end",
            TurnEvent::UserMessage { .. } => "user/message",
            TurnEvent::AssistantChunk { .. } => "assistant/chunk",
            TurnEvent::AssistantMessage { .. } => "assistant/message",
            TurnEvent::ToolCall { .. } => "tool/call",
            TurnEvent::ToolResult { .. } => "tool/result",
            TurnEvent::PlanUpdate { .. } => "plan/update",
            TurnEvent::ProviderUsage { .. } => "provider/usage",
            TurnEvent::HistorySquash { .. } => "history/squash",
            TurnEvent::CheckpointCreated { .. } => "checkpoint/created",
            TurnEvent::CheckpointCheckout { .. } => "checkpoint/checkout",
            TurnEvent::ForkCreated { .. } => "fork/created",
        }
    }

    /// 事件归属的 turn（无 turn 语义的事件返回 None）。
    pub fn turn_id(&self) -> Option<&str> {
        match self {
            TurnEvent::TurnStart { turn_id }
            | TurnEvent::TurnEnd { turn_id, .. }
            | TurnEvent::StepStart { turn_id, .. }
            | TurnEvent::StepEnd { turn_id, .. }
            | TurnEvent::UserMessage { turn_id, .. }
            | TurnEvent::AssistantChunk { turn_id, .. }
            | TurnEvent::AssistantMessage { turn_id, .. }
            | TurnEvent::ToolCall { turn_id, .. }
            | TurnEvent::ToolResult { turn_id, .. }
            | TurnEvent::ProviderUsage { turn_id, .. } => Some(turn_id),
            TurnEvent::PlanUpdate { .. }
            | TurnEvent::HistorySquash { .. }
            | TurnEvent::CheckpointCreated { .. }
            | TurnEvent::CheckpointCheckout { .. }
            | TurnEvent::ForkCreated { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_usage() -> TokenUsage {
        TokenUsage {
            input_tokens: Some(100),
            cache_hit_input_tokens: Some(40),
            cache_hit_source: None,
            reasoning_tokens: Some(10),
            output_tokens: Some(50),
            total_tokens: Some(160),
        }
    }

    #[test]
    fn serde_round_trip_semantic_equivalence() {
        let events: Vec<TurnEvent> = vec![
            TurnEvent::TurnStart {
                turn_id: "t1".into(),
            },
            TurnEvent::TurnEnd {
                turn_id: "t1".into(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: Some(1234),
            },
            TurnEvent::StepStart {
                turn_id: "t1".into(),
                step: 0,
                first_token_latency_ms: Some(88),
            },
            TurnEvent::StepEnd {
                turn_id: "t1".into(),
                step: 0,
            },
            TurnEvent::UserMessage {
                turn_id: "t1".into(),
                text: "hello".into(),
                attachments: Vec::new(),
            },
            TurnEvent::AssistantChunk {
                turn_id: "t1".into(),
                step: 0,
                text: "hi".into(),
            },
            TurnEvent::AssistantMessage {
                turn_id: "t1".into(),
                step: 0,
                text: "hi".into(),
                reasoning_content: None,
                usage: Some(sample_usage()),
                chunk_missing: None,
            },
            TurnEvent::ToolCall {
                turn_id: "t1".into(),
                step: 1,
                call_id: "c1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
                started_at_ms: Some(1000),
            },
            TurnEvent::ToolResult {
                turn_id: "t1".into(),
                step: 1,
                call_id: "c1".into(),
                result: Some("ok".into()),
                error: None,
                status: Some("done".into()),
                duration_ms: Some(50),
                artifacts: None,
                capability_invocation: None,
            },
            TurnEvent::PlanUpdate {
                plan_state: serde_json::json!({"steps": ["a"]}),
            },
            TurnEvent::ProviderUsage {
                turn_id: "t1".into(),
                step: 0,
                request_kind: ProviderRequestKind::InitialRequest,
                usage: sample_usage(),
                cache_hit_input_tokens: Some(40),
                cache_miss_input_tokens: Some(60),
                prefix_mutation_reasons: Vec::new(),
                first_token_latency_ms: Some(88),
                turn_duration_ms: Some(500),
                latency_kind: ProviderLatencyKind::ProviderStream,
                provider: "deepseek".into(),
                model: "deepseek-chat".into(),
            },
            TurnEvent::HistorySquash {
                base_seq: 10,
                summary_message: "summary".into(),
            },
            TurnEvent::CheckpointCreated {
                node_id: "n1".into(),
                kind: "turn_committed".into(),
                parent_node_id: None,
            },
            TurnEvent::CheckpointCheckout {
                node_id: "n1".into(),
                mode: "transcript_only".into(),
            },
            TurnEvent::ForkCreated {
                branch_id: "b2".into(),
                from_node_id: "n1".into(),
            },
        ];
        for event in &events {
            let json = serde_json::to_string(event).expect("serialize");
            let decoded: TurnEvent = serde_json::from_str(&json).expect("deserialize");
            // 语义等价断言：反序列化后重序列化与原始序列化一致（serde 确定性）。
            assert_eq!(
                serde_json::to_string(&decoded).expect("re-serialize"),
                json,
                "round-trip mismatch for {}",
                event.type_name()
            );
            let re_json = serde_json::to_string(&decoded).expect("re-serialize");
            let re_decoded: TurnEvent = serde_json::from_str(&re_json).expect("re-deserialize");
            assert_eq!(
                serde_json::to_string(&re_decoded).expect("re-re-serialize"),
                re_json,
                "double round-trip mismatch"
            );
        }
    }

    #[test]
    fn type_name_and_turn_id() {
        let start = TurnEvent::TurnStart {
            turn_id: "t1".into(),
        };
        assert_eq!(start.type_name(), "turn/start");
        assert_eq!(start.turn_id(), Some("t1"));
        let squash = TurnEvent::HistorySquash {
            base_seq: 1,
            summary_message: "s".into(),
        };
        assert_eq!(squash.type_name(), "history/squash");
        assert_eq!(squash.turn_id(), None);
    }

    #[test]
    fn unknown_type_fails_loud() {
        let bad = r#"{"type":"alien/event","turn_id":"t1"}"#;
        let result: Result<TurnEvent, _> = serde_json::from_str(bad);
        assert!(result.is_err(), "unknown event type must fail loud");
    }

    #[test]
    fn schema_version_is_stable() {
        assert_eq!(EVENT_SCHEMA_VERSION, 1);
    }
}