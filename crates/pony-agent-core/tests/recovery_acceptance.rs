//! L2-T & L2-AT 双模态验收测试：统一错误与恢复管线 (Recovery Acceptance)

use pony_agent_core::agent::recovery::{
    ExecutionRecoveryPolicy, RecoveryAction, RecoveryContext, RecoveryLayer, RecoveryPipelineConfig,
};
use pony_agent_core::agent::retry::{RetryBudget, StreamState};

#[test]
fn test_l1_context_overflow_triggers_token_adjustment() {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig {
        floor_output_tokens: 2048,
        ..Default::default()
    });
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L1TransportUpstream,
        raw_error: "prompt tokens and max_tokens exceed context limit: 120000 > 100000".to_string(),
        error_code: None,
        stream_state: Some(StreamState::NoDelta),
        current_attempt: 0,
        consecutive_failures: 1,
        primary_model: Some("claude-3-5-sonnet".to_string()),
        fallback_model: None,
        tool_name: None,
        tool_is_idempotent: true,
    };

    let action = policy.decide(&ctx, &budget);
    match action {
        RecoveryAction::AdjustTokensAndRetry { suggested_max_tokens, .. } => {
            assert_eq!(suggested_max_tokens, 2048);
        }
        other => panic!("Expected AdjustTokensAndRetry, got {:?}", other),
    }
}

#[test]
fn test_l1_repeated_529_triggers_model_fallback() {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig::default());
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L1TransportUpstream,
        raw_error: "status: 529, type: overloaded_error".to_string(),
        error_code: None,
        stream_state: Some(StreamState::NoDelta),
        current_attempt: 2,
        consecutive_failures: 2,
        primary_model: Some("claude-3-opus".to_string()),
        fallback_model: Some("claude-3-5-sonnet".to_string()),
        tool_name: None,
        tool_is_idempotent: true,
    };

    let action = policy.decide(&ctx, &budget);
    match action {
        RecoveryAction::SwitchModelFallback { fallback_model, .. } => {
            assert_eq!(fallback_model, "claude-3-5-sonnet");
        }
        other => panic!("Expected SwitchModelFallback, got {:?}", other),
    }
}

#[test]
fn test_l1_stream_break_falls_back_to_sync() {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig::default());
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L1TransportUpstream,
        raw_error: "stream disconnected unexpectedly in reasoning phase".to_string(),
        error_code: None,
        stream_state: Some(StreamState::ReasoningOnly),
        current_attempt: 1,
        consecutive_failures: 1,
        primary_model: Some("deepseek-r1".to_string()),
        fallback_model: None,
        tool_name: None,
        tool_is_idempotent: true,
    };

    let action = policy.decide(&ctx, &budget);
    assert!(matches!(action, RecoveryAction::FallbackToSyncRequest { .. }));
}

#[test]
fn test_l2_non_idempotent_tool_timeout_prevents_retry() {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig::default());
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L2ToolExecution,
        raw_error: "execution timeout after 30000ms".to_string(),
        error_code: None,
        stream_state: None,
        current_attempt: 0,
        consecutive_failures: 1,
        primary_model: None,
        fallback_model: None,
        tool_name: Some("bash".to_string()),
        tool_is_idempotent: false,
    };

    let action = policy.decide(&ctx, &budget);
    assert!(matches!(action, RecoveryAction::Abort { is_exhausted: false, .. }));
}

#[test]
fn test_l2_idempotent_tool_error_feeds_back_to_model() {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig::default());
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L2ToolExecution,
        raw_error: "File not found: src/main.rs".to_string(),
        error_code: None,
        stream_state: None,
        current_attempt: 0,
        consecutive_failures: 1,
        primary_model: None,
        fallback_model: None,
        tool_name: Some("read_file".to_string()),
        tool_is_idempotent: true,
    };

    let action = policy.decide(&ctx, &budget);
    match action {
        RecoveryAction::FeedbackToModelAsToolResult { tool_call_id, error_message } => {
            assert_eq!(tool_call_id, "read_file");
            assert!(error_message.contains("File not found"));
        }
        other => panic!("Expected FeedbackToModelAsToolResult, got {:?}", other),
    }
}

#[test]
fn test_l3_consecutive_tool_failures_circuit_breaks_turn() {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig {
        max_consecutive_tool_failures: 3,
        ..Default::default()
    });
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L3TurnProgression,
        raw_error: "Compilation error repeatedly".to_string(),
        error_code: None,
        stream_state: None,
        current_attempt: 0,
        consecutive_failures: 3,
        primary_model: None,
        fallback_model: None,
        tool_name: Some("bash".to_string()),
        tool_is_idempotent: false,
    };

    let action = policy.decide(&ctx, &budget);
    assert!(matches!(action, RecoveryAction::CircuitBreakTurn { .. }));
}
