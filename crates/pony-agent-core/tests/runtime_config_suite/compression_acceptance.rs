//! Stage 1: 上下文压缩水位计算与切分策略验收测试
//!
//! 契约要求（2026-11-10 修订：保留窗口由「轮数」改为**token 预算**）：
//! 1. 默认及自定义 `CompressionConfig` 水位阈值计算
//!    - threshold_tokens = min(context_window × trigger_threshold,
//!      context_window − reserved_completion_tokens − headroom_tokens)
//! 2. 保留比例与尾部 turn 边界切分（`retain_ratio` 派生 token 预算；切点必为 user 消息）
//! 3. 验证 Tool Call 与 Tool Result 不会被跨边界拆分截断（tool pairing 完整性）

use pony_agent_core::agent::compression::{
    compact_native_transcript, split_history_by_tokens, split_history_with_transcript,
    trim_native_transcript_to_token_budget, CompressionConfig, ContextPressure,
};
use pony_agent_core::agent::session::TurnHistoryMessage;
use serde_json::json;

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
    // threshold_tokens = min(cw × ratio, cw − reserved − headroom)
    let config = CompressionConfig {
        trigger_threshold: 0.8,
        headroom_tokens: 65536,
        reserved_completion_tokens: 4096,
        retain_ratio: 0.16,
        summary_role: "user".to_string(),
    };

    // Case A: context_window = 200,000
    // cw × 0.8 = 160,000
    // pressure_budget = 200,000 - 4096 - 65536 = 130,368
    // min(160000, 130368) = 130368
    assert_eq!(
        config.calculate_threshold_tokens(200_000),
        130_368,
        "Threshold must take the minimum between ratio and headroom/reserved budget"
    );

    // Case B: context_window = 1,000,000
    // cw × 0.8 = 800,000
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
    // 契约验证 2：保留窗口按 **token 预算**（保留比例派生），切点必须落在 user 消息。
    let config = CompressionConfig {
        trigger_threshold: 0.8,
        headroom_tokens: 1000,
        reserved_completion_tokens: 500,
        retain_ratio: 0.2,
        summary_role: "user".to_string(),
    };

    let mut history = Vec::new();
    for i in 0..10 {
        history.push(make_msg("user", &format!("user prompt {}", i)));
        history.push(make_msg("assistant", &format!("agent reply {}", i)));
    }

    // 每条消息 = 4 底盘 + ceil(chars/4)；"user prompt 0" 与 "agent reply 0" 均 13 字符
    // ⇒ 每条 8 token，一轮 16 token。预算 48 = 3 轮。
    let retain_tokens = config.retain_tokens(740, 500); // (740 - 500) × 0.2 = 48
    assert_eq!(retain_tokens, 48);

    let (to_compress, to_keep) = split_history_by_tokens(&history, retain_tokens);
    assert_eq!(to_keep.len(), 6, "Must keep 3 recent turns (6 messages)");
    assert_eq!(to_keep[0].content, "user prompt 7");
    assert_eq!(
        to_compress.len(),
        14,
        "Must compress older 7 turns (14 messages)"
    );
}

#[test]
fn test_ac03_tool_pairing_integrity_across_compression_boundary() {
    // 契约验证 3：
    // Tool Call 与 Tool Result 绝不能被跨边界拆分截断（tool pairing 完整性）。
    // 切点固定在 **user 消息**上 ⇒ 一轮内的 tool call / result 必然同侧。
    let mut history = Vec::new();
    // Turn 0
    history.push(make_msg("user", "turn 0"));
    history.push(make_msg("assistant", "reply 0"));
    // Turn 1（含 tool 往返）
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

    // 预算只够最后一轮（"turn 3" / "reply 3" 各 7 字符 ⇒ 6 token，一轮 12）。
    let (to_compress, to_keep) = split_history_by_tokens(&history, 12);
    assert_eq!(to_keep[0].role, "user", "切点必须是 user 消息");

    // 检查 to_compress 与 to_keep 的边界，验证没有任何一对 tool call / result 被拆分
    let compress_has_call_1 = to_compress.iter().any(|m| m.content.contains("call_1"));
    let keep_has_call_1 = to_keep.iter().any(|m| m.content.contains("call_1"));

    assert!(
        !(compress_has_call_1 && keep_has_call_1),
        "Tool call and Tool result for call_1 must NOT be split across the compression boundary!"
    );
}

#[test]
fn test_ac04_native_token_estimate_counts_reasoning_and_tool_calls() {
    let base_msg = json!({
        "role": "assistant",
        "content": "ok"
    });
    let heavy_msg = json!({
        "role": "assistant",
        "content": "ok",
        "reasoning_content": "r".repeat(4_000),
        "tool_calls": [{
            "id": "call_1",
            "type": "function",
            "function": { "name": "Edit", "arguments": "a".repeat(4_000) }
        }]
    });
    let base_tokens = ContextPressure::from_layers(&[], &[base_msg]).transcript_tokens;
    let heavy_tokens = ContextPressure::from_layers(&[], &[heavy_msg]).transcript_tokens;
    assert!(
        heavy_tokens >= base_tokens + 2_000,
        "native token estimate must include reasoning_content and tool_calls: base={base_tokens}, heavy={heavy_tokens}"
    );
}

#[test]
fn test_ac05_split_with_transcript_compresses_heavy_tool_session() {
    // 复现真实缺陷现场：文本 history 仅 ~200 tokens，但原生转录带有 ~16k tokens 的工具结果与思考链；
    // 当 retain_tokens=4000 时，旧的 split_history_by_tokens 只看 history 会错误地返回空压缩段。
    let mut history = Vec::new();
    let mut transcript = Vec::new();
    for i in 0..4 {
        let user_prompt = format!("user turn {i}");
        history.push(make_msg("user", &user_prompt));
        history.push(make_msg("assistant", &format!("done {i}")));

        transcript.push(json!({ "role": "user", "content": user_prompt }));
        transcript.push(json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "t".repeat(4_000),
            "tool_calls": [{
                "id": format!("call_{i}"),
                "type": "function",
                "function": { "name": "Run", "arguments": "{}" }
            }]
        }));
        transcript.push(json!({
            "role": "tool",
            "tool_call_id": format!("call_{i}"),
            "content": "o".repeat(12_000)
        }));
        transcript.push(json!({ "role": "assistant", "content": format!("done {i}") }));
    }

    let (old_compress, _) = split_history_by_tokens(&history, 4_000);
    assert!(
        old_compress.is_empty(),
        "precondition: pure history is smaller than retain_tokens"
    );

    let (to_compress, to_keep) = split_history_with_transcript(&history, &transcript, 4_000);
    assert!(
        !to_compress.is_empty(),
        "split_history_with_transcript must compress earlier turns when native transcript exceeds retain budget"
    );
    assert_eq!(to_keep.len(), 2, "must keep only the latest 1 user turn");
    let kept_turns = to_keep.iter().filter(|m| m.role == "user").count();
    let trimmed_transcript = compact_native_transcript(&transcript, kept_turns);
    assert_eq!(trimmed_transcript.len(), 4);
}

#[test]
fn test_ac06_trim_native_transcript_keeps_closed_roundtrips() {
    let mut transcript = vec![json!({ "role": "user", "content": "single giant turn" })];
    for i in 0..10 {
        transcript.push(json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "x".repeat(800),
            "tool_calls": [{
                "id": format!("call_{i}"),
                "type": "function",
                "function": { "name": "Run", "arguments": "{}" }
            }]
        }));
        transcript.push(json!({
            "role": "tool",
            "tool_call_id": format!("call_{i}"),
            "content": "y".repeat(2_000)
        }));
    }

    let before = ContextPressure::from_layers(&[], &transcript).transcript_tokens;
    let trimmed = trim_native_transcript_to_token_budget(&transcript, 2_500);
    let after = ContextPressure::from_layers(&[], &trimmed).transcript_tokens;

    assert!(after <= 2_500, "trimmed tokens={after} must respect budget");
    assert!(after < before, "trimmed tokens must shrink");
    assert_eq!(
        trimmed.first().and_then(|v| v.get("role")).and_then(|v| v.as_str()),
        Some("user"),
        "user anchor must remain first"
    );
    // 验证最后保留的工具配对闭合（无孤儿 tool_call / tool）
    let mut pending_call_ids: Vec<String> = Vec::new();
    for msg in &trimmed {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                        pending_call_ids.push(id.to_string());
                    }
                }
            }
        } else if role == "tool" {
            let call_id = msg.get("tool_call_id").and_then(|v| v.as_str()).unwrap();
            let pos = pending_call_ids
                .iter()
                .position(|id| id == call_id)
                .expect("every tool message must match a kept assistant tool_call");
            pending_call_ids.remove(pos);
        }
    }
    assert!(pending_call_ids.is_empty(), "all kept tool_calls must be closed: {pending_call_ids:?}");
}
