//! B240 回归：**事件 → refold → 重启修复**后，子活动归因与 C5.1 重试三键必须保留。
//!
//! 缺陷（wave-acp-delegate L3-B ADVISORY / Gate-1 ADVISORY）：
//! `agent::projection` 重建时恒把 `TurnToolActivity.parent_activity_id` 置 `None`、
//! `ProviderCallCacheRecord.attempts/failed_attempt_ms/prelude_ms` 置 `None`；
//! 而 `session::startup_repair::refold_missing_trace_cache` 会把 refold 结果写回 trace 缓存
//! ⇒ 重启修复后子活动**静默退化为顶层活动**、重试观测**静默归零**。
//!
//! 本用例走真实持久化链路：事件 `serde` 序列化（写盘形态）→ 反序列化（重启读取）→
//! `fold_all`（与 startup_repair 同一条折叠路径）。

use pony_agent_core::agent::projection::{fold_all, MetricsProjectionState, TraceProjectionState};
use pony_agent_core::agent::telemetry::{ProviderLatencyKind, ProviderRequestKind};
use pony_agent_core::agent::turn_event::TurnEvent;

fn tool_call(call_id: &str, parent: Option<&str>) -> TurnEvent {
    TurnEvent::ToolCall {
        turn_id: "t1".into(),
        step: 0,
        call_id: call_id.into(),
        name: "acp_delegate".into(),
        arguments: "{}".into(),
        started_at_ms: None,
        description: Some("委派给外部 agent".into()),
        parent_activity_id: parent.map(str::to_string),
    }
}

fn provider_usage() -> TurnEvent {
    TurnEvent::ProviderUsage {
        turn_id: "t1".into(),
        step: 0,
        request_kind: ProviderRequestKind::ToolFollowup,
        usage: pony_agent_core::agent::provider::TokenUsage {
            input_tokens: Some(100),
            cache_hit_input_tokens: Some(40),
            cache_hit_source: Some("provider".into()),
            reasoning_tokens: None,
            output_tokens: Some(20),
            total_tokens: Some(120),
            origin: None,
            cache_creation_input_tokens: None,
        },
        cache_hit_input_tokens: Some(40),
        cache_miss_input_tokens: Some(60),
        prefix_mutation_reasons: Vec::new(),
        first_token_latency_ms: Some(120),
        turn_duration_ms: Some(2_400),
        latency_kind: ProviderLatencyKind::ProviderStream,
        provider: "deepseek".into(),
        model: "deepseek-chat".into(),
        attempts: Some(3),
        failed_attempt_ms: Some(300),
        prelude_ms: Some(900),
    }
}

/// 事件 schema 必须自带这些键（否则「从载荷取回」无从谈起）。
#[test]
fn event_schema_carries_attribution_and_retry_keys() {
    let tool_call_json = serde_json::to_string(&tool_call("acp-tool-1", Some("call-acp-delegate")))
        .expect("serialize tool/call");
    assert!(
        tool_call_json.contains("\"parent_activity_id\":\"call-acp-delegate\""),
        "tool/call 事件必须携带子活动归因，实际 = {tool_call_json}"
    );
    let usage_json = serde_json::to_string(&provider_usage()).expect("serialize provider/usage");
    for key in [
        "\"attempts\":3",
        "\"failed_attempt_ms\":300",
        "\"prelude_ms\":900",
    ] {
        assert!(
            usage_json.contains(key),
            "provider/usage 事件必须携带重试键 {key}，实际 = {usage_json}"
        );
    }
}

/// 写盘 → 重启读取 → refold：子活动归因与重试三键必须原样保留。
#[test]
fn refold_keeps_child_attribution_and_retry_meta() {
    let events: Vec<(u64, TurnEvent)> = vec![
        (0, tool_call("call-acp-delegate", None)),
        (1, tool_call("acp-tool-1", Some("call-acp-delegate"))),
        (2, provider_usage()),
    ];
    // 写盘 → 重启读取（真实持久化形态：JSON；旧日志缺键由 serde default 兜底）。
    let persisted: Vec<(u64, TurnEvent)> = events
        .iter()
        .map(|(seq, event)| {
            let json = serde_json::to_string(event).expect("serialize");
            (
                *seq,
                serde_json::from_str::<TurnEvent>(&json).expect("deserialize"),
            )
        })
        .collect();

    let trace_state = fold_all::<_, TraceProjectionState>(&persisted);
    let trace = trace_state.traces().into_iter().next().expect("trace rebuilt");
    let child = trace
        .tool_activities
        .iter()
        .find(|activity| activity.id == "acp-tool-1")
        .expect("child activity must survive refold");
    assert_eq!(
        child.parent_activity_id.as_deref(),
        Some("call-acp-delegate"),
        "B240：refold 后子活动归因不得丢失（此前恒 None）"
    );
    let top_level = trace
        .tool_activities
        .iter()
        .find(|activity| activity.id == "call-acp-delegate")
        .expect("top-level activity must survive refold");
    assert_eq!(
        top_level.parent_activity_id, None,
        "顶层活动归因必须仍为 None（不得被臆造）"
    );

    let metrics_state = fold_all::<_, MetricsProjectionState>(&persisted);
    let record = metrics_state.by_turn_records["t1"]
        .first()
        .expect("per-call record rebuilt");
    assert_eq!(record.attempts, Some(3), "B240：refold 后 attempts 不得丢失");
    assert_eq!(record.failed_attempt_ms, Some(300));
    assert_eq!(record.prelude_ms, Some(900));
}

/// 旧日志（无这些键）必须仍可折叠：归因为顶层、attempts 未知（不得伪造 1 次）。
#[test]
fn legacy_events_without_keys_still_fold_additively() {
    let legacy: Vec<(u64, TurnEvent)> = vec![
        (
            0,
            serde_json::from_str(
                r#"{"type":"tool_call","turn_id":"t2","step":0,"call_id":"legacy-call","name":"bash","arguments":"{}"}"#,
            )
            .expect("legacy tool/call must deserialize"),
        ),
        (
            1,
            serde_json::from_str(
                r#"{"type":"provider_usage","turn_id":"t2","step":0,"request_kind":"tool_followup","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2},"latency_kind":"provider_stream","provider":"p","model":"m"}"#,
            )
            .expect("legacy provider/usage must deserialize"),
        ),
    ];
    let trace_state = fold_all::<_, TraceProjectionState>(&legacy);
    let trace = trace_state.traces().into_iter().next().expect("legacy trace");
    assert_eq!(
        trace.tool_activities[0].parent_activity_id, None,
        "旧日志缺键 ⇒ 顶层语义（additive 兼容）"
    );
    let metrics_state = fold_all::<_, MetricsProjectionState>(&legacy);
    assert_eq!(
        metrics_state.by_turn_records["t2"][0].attempts, None,
        "旧日志缺键 ⇒ attempts 未知（不得伪造 1 次）"
    );
}
