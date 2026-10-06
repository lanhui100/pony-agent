//! Stage 1: 上下文压缩水位计算与配置策略隔离验收测试
//!
//! 契约要求：
//! 1. 默认及自定义 CompressionConfig 水位阈值计算
//!    - threshold_tokens = min(context_window * threshold_ratio, context_window - reserved_completion_tokens - headroom_tokens)
//! 2. 保留比例与尾部 turn 边界切分（retain_ratio / keep_recent_turns）
//! 3. 验证 Tool Call 与 Tool Result 不会被跨边界拆分截断（tool pairing 完整性）

use pony_agent_core::agent::compression::{
    split_history, CompressionConfig,
};
use pony_agent_core::agent::session::TurnHistoryMessage;

fn make_msg(role: &str, content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

fn make_tool_call_msg(id: &str, name: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "assistant".to_string(),
        content: format!(r#"{{"tool_calls": [{{"id": "{}", "name": "{}"}}]}}"#, id, name),
        ..Default::default()
    }
}

fn make_tool_result_msg(id: &str, output: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "tool".to_string(),
        content: format!(r#"{{"tool_call_id": "{}", "output": "{}"}}"#, id, output),
        ..Default::default()
    }
}

#[test]
fn test_ac01_threshold_tokens_calculation_with_headroom_and_reserved() {
    // 契约验证 1：
    // threshold_tokens = min(context_window * threshold_ratio, context_window - reserved_completion_tokens - headroom_tokens)
    let config = CompressionConfig {
        trigger_threshold: 0.8,
        headroom_tokens: 65536,
        reserved_completion_tokens: 4096,
        retain_ratio: 0.16,
        keep_recent_turns: 10,
        summary_role: "user".to_string(),
    };

    // Case A: context_window = 200,000
    // context_window * 0.8 = 160,000
    // pressure_budget = 200,000 - 4096 - 65536 = 130,368
    // min(160000, 130368) = 130368
    assert_eq!(
        config.calculate_threshold_tokens(200_000),
        130_368,
        "Threshold must take the minimum between ratio and headroom/reserved budget"
    );

    // Case B: context_window = 1,000,000
    // context_window * 0.8 = 800,000
    // pressure_budget = 1,000,000 - 4096 - 65536 = 930,368
    // min(800000, 930368) = 800000
    assert_eq!(
        config.calculate_threshold_tokens(1_000_000),
        800_000,
        "When context window is large, threshold_ratio (80%) should cap the threshold"
    );
}

#[test]
fn test_ac02_retain_ratio_and_turn_boundary_split() {
    // 契约验证 2：
    // 根据 retain_ratio 或 keep_recent_turns 保证尾部完整性
    let config = CompressionConfig {
        trigger_threshold: 0.8,
        headroom_tokens: 1000,
        reserved_completion_tokens: 500,
        retain_ratio: 0.2, // 期望保留约 20%
        keep_recent_turns: 3,
        summary_role: "user".to_string(),
    };

    let mut history = Vec::new();
    for i in 0..10 {
        history.push(make_msg("user", &format!("user prompt {}", i)));
        history.push(make_msg("assistant", &format!("agent reply {}", i)));
    }

    let (to_compress, to_keep) = split_history(&history, &config);
    // 验证保留尾部 3 轮
    assert_eq!(to_keep.len(), 6, "Must keep 3 recent turns (6 messages)");
    assert_eq!(to_keep[0].content, "user prompt 7");
    assert_eq!(to_compress.len(), 14, "Must compress older 7 turns (14 messages)");
}

#[test]
fn test_ac03_tool_pairing_integrity_across_compression_boundary() {
    // 契约验证 3：
    // Tool Call 与 Tool Result 绝不能被跨边界拆分截断（tool pairing 完整性）
    // 场景：在切分点附近，assistant 发起了 tool_calls，随后跟着 tool 结果。
    // 如果切分点恰好落在 tool_call 与 tool_result 之间，必须将切分点前移或后移，
    // 确保 pair 处于同一侧，严禁出现孤儿 tool_call 或孤儿 tool_result。
    let config = CompressionConfig {
        trigger_threshold: 0.8,
        headroom_tokens: 1000,
        reserved_completion_tokens: 500,
        retain_ratio: 0.2,
        keep_recent_turns: 2,
        summary_role: "user".to_string(),
    };

    let mut history = Vec::new();
    // Turn 0
    history.push(make_msg("user", "turn 0"));
    history.push(make_msg("assistant", "reply 0"));
    // Turn 1
    history.push(make_msg("user", "turn 1"));
    history.push(make_tool_call_msg("call_1", "read_file"));
    history.push(make_tool_result_msg("call_1", "file contents"));
    history.push(make_msg("assistant", "reply 1 after tool"));
    // Turn 2
    history.push(make_msg("user", "turn 2"));
    history.push(make_msg("assistant", "reply 2"));
    // Turn 3
    history.push(make_msg("user", "turn 3"));
    history.push(make_msg("assistant", "reply 3"));

    let (to_compress, to_keep) = split_history(&history, &config);

    // 检查 to_compress 与 to_keep 的边界，验证没有任何一对 tool call / result 被拆分
    let compress_has_call_1 = to_compress.iter().any(|m| m.content.contains("call_1"));
    let keep_has_call_1 = to_keep.iter().any(|m| m.content.contains("call_1"));

    assert!(
        !(compress_has_call_1 && keep_has_call_1),
        "Tool call and Tool result for call_1 must NOT be split across the compression boundary!"
    );
}
