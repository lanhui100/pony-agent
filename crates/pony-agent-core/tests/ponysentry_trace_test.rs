//! PonySentry Trace 链路遥测红相独立验收与对抗测试 (L2/L3 Red Phase Contract & Adversarial Test)
//!
//! 覆盖目标：
//! 1. 契约与序列化：AgentTracePayload 与 EvalStatus 状态机序列化/反序列化（默认 unreviewed、6 种状态枚举互转、缺失可选字段容错）
//! 2. 零信任脱敏安全边界：针对 AgentTracePayload 或 Extra/Summary 中的敏感词（sk-xxx、Authorization Header、User Home 路径、API Keys）
//! 3. 客户端链路与对抗防护：
//!    - PonySentryClient 提供 capture_agent_trace 接口
//!    - 有界队列（<= 1024）极限写入防雪崩保护
//!    - 网络超时 / 不可达沙盒隔离测试（非阻塞、不 panic）
//!    - 大 Payload 极限处理（超多 turns/tool_calls）

use pony_agent_core::agent::ponysentry::{
    capture_agent_trace, sanitize, sanitize_json, AgentTracePayload, EvalStatus,
    PonySentryClient, PonySentryConfig, ToolCallTraceItem, TurnTraceItem,
};
use serde_json::json;
use std::collections::HashMap;
use std::time::Instant;

// ==========================================
// 1. 数据契约与 EvalStatus 状态机测试
// ==========================================

#[test]
fn test_eval_status_state_machine_serialization() {
    let cases = vec![
        (EvalStatus::Unreviewed, "\"unreviewed\""),
        (EvalStatus::TriageGood, "\"triage_good\""),
        (EvalStatus::TriageBad, "\"triage_bad\""),
        (EvalStatus::EvalDataset, "\"eval_dataset\""),
        (EvalStatus::Optimized, "\"optimized\""),
        (EvalStatus::Wontfix, "\"wontfix\""),
    ];

    for (status, expected_json) in cases {
        // Arrange (status), Act (serialize/deserialize), Assert
        let serialized = serde_json::to_string(&status).expect("Serialization failed");
        assert_eq!(serialized, expected_json, "EvalStatus serialization contract");
        let deserialized: EvalStatus =
            serde_json::from_str(&serialized).expect("Deserialization failed");
        assert_eq!(deserialized, status, "EvalStatus round-trip contract");
    }

    // Default status must be unreviewed
    assert_eq!(EvalStatus::default(), EvalStatus::Unreviewed);
}

#[test]
fn test_agent_trace_payload_serialization_contract() {
    // Arrange
    let mut tags = HashMap::new();
    tags.insert("provider".to_string(), "anthropic".to_string());
    tags.insert("model".to_string(), "claude-3-7-sonnet".to_string());

    let payload = AgentTracePayload {
        session_id: "sess-abc-123".to_string(),
        run_id: Some("run-456".to_string()),
        turn_id: Some("turn-789".to_string()),
        environment: "production".to_string(),
        release: "0.1.109".to_string(),
        eval_status: EvalStatus::Unreviewed,
        turns: vec![TurnTraceItem {
            turn_id: "turn-789".to_string(),
            sequence: Some(1),
            phase: Some("act".to_string()),
            provider: Some("anthropic".to_string()),
            model: Some("claude-3-7-sonnet".to_string()),
            input_tokens: Some(1200),
            output_tokens: Some(300),
            cache_hit_tokens: Some(100),
            duration_ms: Some(850),
            error: None,
            tool_calls: vec![ToolCallTraceItem {
                call_id: Some("call-001".to_string()),
                tool_name: "bash".to_string(),
                arguments_summary: Some("cargo check".to_string()),
                status: "success".to_string(),
                duration_ms: Some(120),
                error: None,
            }],
            started_at_ms: Some(1700000000000),
            completed_at_ms: Some(1700000000850),
        }],
        tags: Some(tags),
        extra: Some(json!({"task_id": "task-2", "prompt_length": 42})),
        total_input_tokens: Some(1200),
        total_output_tokens: Some(300),
        total_duration_ms: Some(850),
        reported_at_ms: 1700000001000,
    };

    // Act
    let json_str = serde_json::to_string(&payload).expect("Serialization must succeed");
    let val: serde_json::Value =
        serde_json::from_str(&json_str).expect("Deserialization to JSON Value must succeed");

    // Assert
    assert_eq!(val["session_id"], "sess-abc-123");
    assert_eq!(val["run_id"], "run-456");
    assert_eq!(val["turn_id"], "turn-789");
    assert_eq!(val["eval_status"], "unreviewed");
    assert_eq!(val["turns"][0]["tool_calls"][0]["tool_name"], "bash");
    assert_eq!(val["extra"]["task_id"], "task-2");

    // Round-trip back to struct
    let back: AgentTracePayload =
        serde_json::from_str(&json_str).expect("Round-trip deserialization must succeed");
    assert_eq!(back.session_id, payload.session_id);
    assert_eq!(back.eval_status, EvalStatus::Unreviewed);
    assert_eq!(back.turns.len(), 1);
    assert_eq!(back.turns[0].tool_calls.len(), 1);
}

#[test]
fn test_agent_trace_payload_optional_fields_fault_tolerance() {
    // Arrange: Minimal JSON payload without optional fields
    let minimal_json = r#"{
        "session_id": "sess-min-999",
        "environment": "test",
        "release": "0.1.0",
        "eval_status": "triage_bad",
        "reported_at_ms": 1700000000000
    }"#;

    // Act
    let parsed: Result<AgentTracePayload, _> = serde_json::from_str(minimal_json);

    // Assert
    assert!(parsed.is_ok(), "Minimal JSON must deserialize successfully");
    let p = parsed.unwrap();
    assert_eq!(p.session_id, "sess-min-999");
    assert_eq!(p.eval_status, EvalStatus::TriageBad);
    assert!(p.run_id.is_none());
    assert!(p.turns.is_empty());
    assert!(p.tags.is_none());
    assert!(p.extra.is_none());
}

// ==========================================
// 2. 零信任脱敏安全边界测试
// ==========================================

#[test]
fn test_zero_trust_redaction_for_trace_and_secrets() {
    // Arrange
    let home_path_leak = "Loaded config from /home/developer/.config/pony/token";
    let raw_leak = "API Key sk-test-mock-abcdef1234567890abcdef1234567890 and Bearer mock-test-secret-token-xyz789";

    // Act
    let sanitized_path = sanitize(home_path_leak);
    let sanitized_leak = sanitize(raw_leak);

    // Assert
    assert!(
        !sanitized_path.contains("/home/developer"),
        "Unix user path must be redacted (received: {sanitized_path})"
    );
    assert!(
        sanitized_path.contains("[USER_HOME]"),
        "Redacted path must contain [USER_HOME] (received: {sanitized_path})"
    );
    assert!(
        !sanitized_leak.contains("mock-test-secret-token-xyz789"),
        "Bearer secret must not leak in raw string (received: {sanitized_leak})"
    );

    // Act: JSON sanitization
    let json_with_secret = json!({
        "sk_token": "sk-mock-dummy-key-sample-1234567890",
        "authorization": "Bearer mock-test-super-secret-key-12345",
        "normal_field": "safe content",
        "nested": {"api_key": "mock-draft-hidden_key_123"}
    });
    let sanitized_val = sanitize_json(&json_with_secret);

    // Assert
    assert_eq!(
        sanitized_val["authorization"],
        "[REDACTED_SECRET]",
        "authorization key must be fully redacted"
    );
    assert_eq!(
        sanitized_val["nested"]["api_key"],
        "[REDACTED_SECRET]",
        "nested api_key must be fully redacted"
    );
    assert_eq!(sanitized_val["normal_field"], "safe content");
}

// ==========================================
// 3. 客户端异步上报与对抗破坏测试 (Adversarial)
// ==========================================

#[test]
fn test_client_capture_agent_trace_api_red_phase() {
    // Arrange
    let config = PonySentryConfig {
        enabled: true,
        // Port 9 (discard) is a blackhole: connect succeeds superficially, no ingest server.
        endpoint: "http://127.0.0.1:9".to_string(),
        client_token: Some("test-token".to_string()),
        environment: "test".to_string(),
        release: "0.1.0".to_string(),
    };
    let client = PonySentryClient::new(config.clone());

    let trace = AgentTracePayload {
        session_id: "sess-red-001".to_string(),
        run_id: None,
        turn_id: None,
        environment: "test".to_string(),
        release: "0.1.0".to_string(),
        eval_status: EvalStatus::Unreviewed,
        turns: vec![],
        tags: None,
        extra: None,
        total_input_tokens: None,
        total_output_tokens: None,
        total_duration_ms: None,
        reported_at_ms: 1700000000000,
    };

    // Act & Assert:
    // 1. Client instance method: capture_agent_trace
    client.capture_agent_trace(trace.clone());

    // 2. Global module function: capture_agent_trace
    capture_agent_trace(trace);
}

#[test]
fn test_queue_overflow_and_avalanche_defense() {
    // Arrange: create client with unreachable endpoint
    let config = PonySentryConfig {
        enabled: true,
        endpoint: "http://127.0.0.1:9".to_string(),
        client_token: Some("test-token".to_string()),
        environment: "test".to_string(),
        release: "0.1.0".to_string(),
    };
    let client = PonySentryClient::new(config);

    let start = Instant::now();

    // Act: Burst 2000 events (> 1024 bounded capacity) into queue
    for i in 0..2000 {
        let trace = AgentTracePayload {
            session_id: format!("sess-burst-{i}"),
            run_id: None,
            turn_id: None,
            environment: "test".to_string(),
            release: "0.1.0".to_string(),
            eval_status: EvalStatus::Unreviewed,
            turns: vec![],
            tags: None,
            extra: None,
            total_input_tokens: None,
            total_output_tokens: None,
            total_duration_ms: None,
            reported_at_ms: 1700000000000,
        };
        client.capture_agent_trace(trace);
    }

    let elapsed = start.elapsed();

    // Assert: Non-blocking enqueue must take < 500ms even when queue overflows and drops
    assert!(
        elapsed.as_millis() < 500,
        "Burst enqueue took {}ms; must be strictly non-blocking under overflow",
        elapsed.as_millis()
    );
}

#[test]
fn test_large_payload_limit_handling() {
    // Arrange: Create large trace with 100 turns and 1000 tool calls (100 x 10)
    let mut turns = Vec::with_capacity(100);
    for t in 0..100 {
        let mut tool_calls = Vec::with_capacity(10);
        for c in 0..10 {
            tool_calls.push(ToolCallTraceItem {
                call_id: Some(format!("call-{t}-{c}")),
                tool_name: "edit".to_string(),
                arguments_summary: Some("Large file replacement payload argument".to_string()),
                status: "success".to_string(),
                duration_ms: Some(15),
                error: None,
            });
        }
        turns.push(TurnTraceItem {
            turn_id: format!("turn-{t}"),
            sequence: Some(t as u64),
            phase: Some("act".to_string()),
            provider: Some("anthropic".to_string()),
            model: Some("claude-3-7-sonnet".to_string()),
            input_tokens: Some(5000),
            output_tokens: Some(800),
            cache_hit_tokens: Some(2000),
            duration_ms: Some(300),
            error: None,
            tool_calls,
            started_at_ms: Some(1700000000000 + (t as u64 * 1000)),
            completed_at_ms: Some(1700000000300 + (t as u64 * 1000)),
        });
    }

    let large_trace = AgentTracePayload {
        session_id: "sess-large-stress".to_string(),
        run_id: Some("run-large".to_string()),
        turn_id: None,
        environment: "production".to_string(),
        release: "0.1.109".to_string(),
        eval_status: EvalStatus::EvalDataset,
        turns,
        tags: None,
        extra: Some(json!({"stress": true})),
        total_input_tokens: Some(500000),
        total_output_tokens: Some(80000),
        total_duration_ms: Some(30000),
        reported_at_ms: 1700000030000,
    };

    // Act
    let serialized = serde_json::to_string(&large_trace).expect("Large payload must serialize");
    let deserialized: AgentTracePayload =
        serde_json::from_str(&serialized).expect("Large payload must deserialize");

    // Assert
    assert_eq!(deserialized.turns.len(), 100);
    assert_eq!(deserialized.turns[0].tool_calls.len(), 10);
    assert_eq!(deserialized.eval_status, EvalStatus::EvalDataset);
}