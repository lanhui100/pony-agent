//! provider native transcript 的装载期清洗（自 `session/store.rs` 外移，纯搬迁、语义不变）。
//!
//! 职责：工具往返自愈 + reasoning 缺口判据与「截断到首个缺口前」的降级动作。

use serde_json::Value;

use super::store::is_interrupt_placeholder_text;
use super::types::SessionState;

pub(crate) fn sanitize_provider_native_transcript(session: &mut SessionState) -> bool {
    let mut mutated = false;

    // 1. 若存在未闭合的工具调用或孤儿 tool 结果，在持久化层执行就地自愈与过滤（单一真源状态机），
    // 补齐 synthetic 取消占位并闭合工具往返，丢弃无主孤儿，杜绝 HTTP 400（ADR 0123）。
    let (healed, roundtrip_mutated) = crate::agent::tool_pruner::repair_and_filter_tool_roundtrips(
        &session.provider_native_transcript,
    );
    if roundtrip_mutated {
        session.provider_native_transcript = healed;
        mutated = true;
    }

    // 2. 检查 reasoning 模型的思维链缺口。缺口消息本身不可用（缺 reasoning 的 assistant 会让
    //    上游 400），缺口之后的一切都建立在它之上、无法安全续接，故一并丢弃；缺口之前是完整
    //    可复用的前缀，必须保留（B114：旧逻辑 clear() 整段，续跑时上下文全丢）。
    if let Some(gap_index) = first_legacy_reasoning_gap(&session.provider_native_transcript) {
        session.provider_native_transcript.truncate(gap_index);
        // 截断可能把末尾留成孤儿 tool 结果（缺口紧跟在 tool 之后时），再走一次状态机自愈
        // 保持尾部合法，杜绝 HTTP 400。
        let (healed, _) = crate::agent::tool_pruner::repair_and_filter_tool_roundtrips(
            &session.provider_native_transcript,
        );
        session.provider_native_transcript = healed;
        return true;
    }

    mutated
}

/// 首个 reasoning 缺口消息的下标（`None` = 无缺口），本判定的单一真源。
///
/// 消费方：`sanitize_provider_native_transcript` 据此做「截断到缺口前」的细粒度降级（B114）；
/// 中断/失败轮闭合片段的「末条 assistant 必须带 reasoning」约束也以本判据为准。
pub(crate) fn first_legacy_reasoning_gap(transcript: &[Value]) -> Option<usize> {
    // 只有当 transcript 中至少有一条【非中断注记】的消息带有非空的 reasoning_content 时，
    // 该 transcript 才属于真正的 reasoning 模型上下文；纯工具模型直接豁免！
    // D1：判据统一走 `is_interrupt_placeholder_text`（检查点 / 上游中断 / 用户终止三类注记）。
    let is_reasoning_transcript = transcript.iter().any(|msg| {
        let content_str = msg.get("content").and_then(Value::as_str).unwrap_or("");
        if is_interrupt_placeholder_text(content_str) {
            return false;
        }
        msg.get("reasoning_content")
            .map(|val| !reasoning_content_missing(val))
            .unwrap_or(false)
    });
    if !is_reasoning_transcript {
        return None;
    }

    let mut awaiting_tool_turn_reasoning = false;

    for (index, message) in transcript.iter().enumerate() {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                awaiting_tool_turn_reasoning = false;
            }
            Some("tool") => {
                awaiting_tool_turn_reasoning = true;
            }
            Some("assistant") => {
                let content_str = message.get("content").and_then(Value::as_str).unwrap_or("");
                if is_interrupt_placeholder_text(content_str) {
                    awaiting_tool_turn_reasoning = false;
                    continue;
                }
                let has_tool_calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| !calls.is_empty())
                    .unwrap_or(false);
                let missing_reasoning = message
                    .get("reasoning_content")
                    .map(reasoning_content_missing)
                    .unwrap_or(true);

                if (has_tool_calls || awaiting_tool_turn_reasoning) && missing_reasoning {
                    return Some(index);
                }

                if has_tool_calls {
                    awaiting_tool_turn_reasoning = true;
                }
            }
            _ => {}
        }
    }

    None
}

fn reasoning_content_missing(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}
