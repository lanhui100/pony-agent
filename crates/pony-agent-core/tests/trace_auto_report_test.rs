//! PonySentry Trace 自动上报纯函数与触发策略契约测试 (Test as Contract)
//!
//! 覆盖目标：
//! 1. 终态判定契约：is_trace_reportable_terminal("turn:completed" / "turn:failed" / "turn:cancelled") == true, ("turn:suspended" / 其他) == false
//! 2. 映射契约：build_agent_trace_from_event 从构造的 TurnStreamEvent 完整映射各字段（session_id, turn_id, tokens, tool_activities → tool_calls）
//! 3. 缺失关键字段容错：缺失 session_id 或 turn_id 时返回 None
//! 4. 自动上报门控：maybe_report_turn_trace 与 is_enabled 接口期望

use pony_agent_core::agent::ponysentry::{
    build_agent_trace_from_event, is_enabled, is_trace_reportable_terminal,
    maybe_report_turn_trace, AgentTracePayload, EvalStatus,
};
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::telemetry::TurnToolActivity;
use serde_json::json;

// ==========================================
// 1. 终态判定策略契约
// ==========================================

#[test]
fn test_is_trace_reportable_terminal_contract() {
    // Act & Assert: 允许上报的终态事件
    assert!(
        is_trace_reportable_terminal("turn:completed"),
        "turn:completed must be reportable"
    );
    assert!(
        is_trace_reportable_terminal("turn:failed"),
        "turn:failed must be reportable"
    );
    assert!(
        is_trace_reportable_terminal("turn:cancelled"),
        "turn:cancelled must be reportable"
    );

    // Act & Assert: 非终态或暂挂事件（严禁上报，避免重复或未结束链路泄露）
    assert!(
        !is_trace_reportable_terminal("turn:suspended"),
        "turn:suspended must NOT be reported"
    );
    assert!(
        !is_trace_reportable_terminal("turn:started"),
        "turn:started must NOT be reported"
    );
    assert!(
        !is_trace_reportable_terminal("turn:chunk"),
        "turn:chunk must NOT be reported"
    );
    assert!(
        !is_trace_reportable_terminal("turn:tool_call"),
        "turn:tool_call must NOT be reported"
    );
}

// ==========================================
// 2. TurnStreamEvent -> AgentTracePayload 纯函数映射契约
// ==========================================

#[test]
fn test_build_agent_trace_from_event_complete_mapping() {
    // Arrange
    let tool_activity = TurnToolActivity {
        id: "act-call-001".to_string(),
        name: "bash".to_string(),
        canonical_tool_name: Some("bash".to_string()),
        display_name_zh: Some("执行命令".to_string()),
        status: "success".to_string(),
        description: "run cargo test".to_string(),
        arguments_text: Some("cargo test -p pony-agent-core".to_string()),
        result_text: Some("test result ok".to_string()),
        duration_seconds: Some(1.25),
        parent_activity_id: None,
        artifacts: None,
        error: None,
        capability_invocation: None,
    };

    let event = TurnStreamEvent {
        event_id: Some("evt-001".to_string()),
        session_id: Some("sess-contract-123".to_string()),
        turn_id: "turn-contract-456".to_string(),
        kind: "turn:completed".to_string(),
        event_type: None,
        event_version: None,
        sequence: Some(42),
        emitted_at_ms: Some(1710000000000),
        phase: Some("act".to_string()),
        text: Some("Task finished".to_string()),
        reasoning_content: None,
        error: None,
        provider_requested_name: None,
        provider_name: Some("anthropic".to_string()),
        provider_protocol: None,
        provider_model: Some("claude-3-7-sonnet".to_string()),
        provider_source: None,
        provider_mode: None,
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: Some(1500),
        cache_hit_input_tokens: Some(300),
        reasoning_tokens: None,
        output_tokens: Some(250),
        total_tokens: Some(1750),
        first_token_latency_ms: None,
        turn_duration_ms: Some(3200),
        trace_steps: None,
        trace_timeline: None,
        tool_activities: Some(vec![tool_activity]),
        provider_call_records: None,
        hook_trace_records: None,
        session_summary: Some("Session about building trace feature".to_string()),
        step: Some(0),
    };

    // Act
    let trace_opt = build_agent_trace_from_event(&event);

    // Assert
    assert!(trace_opt.is_some(), "Trace mapping must succeed for valid terminal event");
    let trace = trace_opt.unwrap();

    assert_eq!(trace.session_id, "sess-contract-123");
    assert_eq!(trace.turn_id, Some("turn-contract-456".to_string()));
    assert_eq!(trace.eval_status, EvalStatus::Unreviewed);
    assert_eq!(trace.total_input_tokens, Some(1500));
    assert_eq!(trace.total_output_tokens, Some(250));
    assert_eq!(trace.total_duration_ms, Some(3200));

    // Turn 级别明细断言
    assert_eq!(trace.turns.len(), 1);
    let turn = &trace.turns[0];
    assert_eq!(turn.turn_id, "turn-contract-456");
    assert_eq!(turn.sequence, Some(42));
    assert_eq!(turn.phase, Some("act".to_string()));
    assert_eq!(turn.provider, Some("anthropic".to_string()));
    assert_eq!(turn.model, Some("claude-3-7-sonnet".to_string()));
    assert_eq!(turn.input_tokens, Some(1500));
    assert_eq!(turn.cache_hit_tokens, Some(300));
    assert_eq!(turn.output_tokens, Some(250));
    assert_eq!(turn.duration_ms, Some(3200));

    // Tool activity 转换为 tool calls 断言
    assert_eq!(turn.tool_calls.len(), 1);
    let tool_call = &turn.tool_calls[0];
    assert_eq!(tool_call.call_id, Some("act-call-001".to_string()));
    assert_eq!(tool_call.tool_name, "bash");
    assert_eq!(tool_call.status, "success");
    assert_eq!(tool_call.duration_ms, Some(1250)); // 1.25s -> 1250ms
    assert_eq!(
        tool_call.arguments_summary,
        Some("cargo test -p pony-agent-core".to_string())
    );
}

#[test]
fn test_build_agent_trace_from_event_missing_required_fields() {
    // Arrange: 缺少 session_id 时，不应构造不完整的 Trace
    let event_no_session = TurnStreamEvent {
        event_id: None,
        session_id: None,
        turn_id: "turn-contract-999".to_string(),
        kind: "turn:completed".to_string(),
        event_type: None,
        event_version: None,
        sequence: None,
        emitted_at_ms: None,
        phase: None,
        text: None,
        reasoning_content: None,
        error: None,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
        trace_steps: None,
        trace_timeline: None,
        tool_activities: None,
        provider_call_records: None,
        hook_trace_records: None,
        session_summary: None,
        step: None,
    };

    // Act & Assert
    assert!(
        build_agent_trace_from_event(&event_no_session).is_none(),
        "Must return None when session_id is missing"
    );
}

// ==========================================
// 3. 自动上报入口与门控存在性断言
// ==========================================

#[test]
fn test_maybe_report_turn_trace_entrypoint() {
    // Arrange
    let event = TurnStreamEvent {
        event_id: Some("evt-002".to_string()),
        session_id: Some("sess-report-001".to_string()),
        turn_id: "turn-report-001".to_string(),
        kind: "turn:completed".to_string(),
        event_type: None,
        event_version: None,
        sequence: Some(1),
        emitted_at_ms: Some(1710000000000),
        phase: Some("act".to_string()),
        text: Some("ok".to_string()),
        reasoning_content: None,
        error: None,
        provider_requested_name: None,
        provider_name: Some("anthropic".to_string()),
        provider_protocol: None,
        provider_model: Some("claude-3-7-sonnet".to_string()),
        provider_source: None,
        provider_mode: None,
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: Some(100),
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: Some(50),
        total_tokens: Some(150),
        first_token_latency_ms: None,
        turn_duration_ms: Some(500),
        trace_steps: None,
        trace_timeline: None,
        tool_activities: None,
        provider_call_records: None,
        hook_trace_records: None,
        session_summary: None,
        step: None,
    };

    // Act & Assert:
    // 门控检查与非阻塞调用，无论客户端是否启用，均不可 panic
    let _ = is_enabled();
    maybe_report_turn_trace(&event);
}

// ==========================================
// 4. L3 FAIL 修复固化：短名 kind 与 error 编码契约（防回归）
// ==========================================

#[test]
fn test_is_trace_reportable_terminal_short_kind_contract() {
    // 运行时真实 TurnStreamEvent.payload.kind 是短名（turn_stream.rs:1182 kind="completed",
    // turn_flow.rs:571 kind="cancelled"），L3 审查确认此前门控对真实终态恒 false 导致
    // 生产零上报；修复后短名必须全部判定为可上报。
    assert!(is_trace_reportable_terminal("completed"));
    assert!(is_trace_reportable_terminal("failed"));
    assert!(is_trace_reportable_terminal("cancelled"));
    // 短名非终态仍不可上报
    assert!(!is_trace_reportable_terminal("suspended"));
    assert!(!is_trace_reportable_terminal("delta"));
}

#[test]
fn test_build_agent_trace_from_event_short_kind_mapping() {
    // 用真实运行时形态（kind="completed" 短名）构造终态事件，映射必须成功
    let event = TurnStreamEvent {
        event_id: Some("evt-short-001".to_string()),
        session_id: Some("sess-short-001".to_string()),
        turn_id: "turn-short-001".to_string(),
        kind: "completed".to_string(),
        event_type: Some("completed".into()),
        event_version: None,
        sequence: Some(7),
        emitted_at_ms: Some(1710000000000),
        phase: Some("act".to_string()),
        text: Some("done".to_string()),
        reasoning_content: None,
        error: None,
        provider_requested_name: None,
        provider_name: Some("deepseek".to_string()),
        provider_protocol: None,
        provider_model: Some("deepseek-chat".to_string()),
        provider_source: None,
        provider_mode: None,
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: Some(200),
        cache_hit_input_tokens: Some(50),
        reasoning_tokens: None,
        output_tokens: Some(80),
        total_tokens: Some(280),
        first_token_latency_ms: None,
        turn_duration_ms: Some(1500),
        trace_steps: None,
        trace_timeline: None,
        tool_activities: Some(vec![TurnToolActivity {
            id: "act-short-1".into(),
            name: "bash".into(),
            canonical_tool_name: Some("bash".into()),
            display_name_zh: None,
            status: "success".into(),
            description: "run test".into(),
            arguments_text: Some("cargo test".into()),
            result_text: None,
            duration_seconds: Some(0.5),
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }]),
        provider_call_records: None,
        hook_trace_records: None,
        session_summary: Some("short".into()),
        step: Some(0),
    };

    let trace = build_agent_trace_from_event(&event).expect("短名 completed 终态必须可构造");
    assert_eq!(trace.session_id, "sess-short-001");
    assert_eq!(trace.turns.len(), 1);
    assert_eq!(trace.turns[0].model.as_deref(), Some("deepseek-chat"));
    assert_eq!(trace.turns[0].tool_calls.len(), 1);
    assert_eq!(trace.turns[0].tool_calls[0].tool_name, "bash");
    assert_eq!(trace.turns[0].tool_calls[0].duration_ms, Some(500));
}

#[test]
fn test_tool_error_string_is_bare_text_not_json_encoded() {
    // L3 审查 finding F3：act.error 为 Value::String 时 to_string() 会输出带引号/转义的
    // JSON（"boom" -> "\"boom\""）；修复后字符串必须取裸文本。
    let event = TurnStreamEvent {
        event_id: Some("evt-err-001".to_string()),
        session_id: Some("sess-err-001".to_string()),
        turn_id: "turn-err-001".to_string(),
        kind: "completed".to_string(),
        event_type: Some("completed".into()),
        event_version: None,
        sequence: Some(1),
        emitted_at_ms: Some(1710000000000),
        phase: Some("act".to_string()),
        text: Some("boom".to_string()),
        reasoning_content: None,
        error: None,
        provider_requested_name: None,
        provider_name: Some("deepseek".to_string()),
        provider_protocol: None,
        provider_model: Some("deepseek-chat".to_string()),
        provider_source: None,
        provider_mode: None,
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
        trace_steps: None,
        trace_timeline: None,
        tool_activities: Some(vec![TurnToolActivity {
            id: "act-err-1".into(),
            name: "bash".into(),
            canonical_tool_name: Some("bash".into()),
            display_name_zh: None,
            status: "error".into(),
            description: "boom".into(),
            arguments_text: None,
            result_text: None,
            duration_seconds: None,
            parent_activity_id: None,
            artifacts: None,
            error: Some(serde_json::Value::String("boom".to_string())),
            capability_invocation: None,
        }]),
        provider_call_records: None,
        hook_trace_records: None,
        session_summary: None,
        step: None,
    };

    let trace = build_agent_trace_from_event(&event).expect("error 事件必须可构造");
    let tool_error = trace.turns[0].tool_calls[0].error.clone();
    assert_eq!(
        tool_error.as_deref(),
        Some("boom"),
        "字符串 error 必须是裸文本，不得带 JSON 引号转义"
    );
    assert!(!tool_error.unwrap_or_default().contains('"'));
}
