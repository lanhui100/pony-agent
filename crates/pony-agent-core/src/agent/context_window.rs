//! ADR 0123：跨轮次上下文窗口装配。
//!
//! 这里收纳「把会话历史裁进模型窗口」的纯函数：轮次边界对齐、token 预算截断、
//! 原生工具转录的确定性修剪、修复与尾部锚定对齐。`context.rs` 只保留分层装配与提示词渲染，
//! 不再持有窗口裁剪细节。

use crate::agent::compression::estimate_native_message_tokens;
use crate::agent::provider::{
    sanitize_dsml_protocol_text, ProviderManager, ProviderMessage, ProviderRole,
};
use crate::agent::session::{AttachmentAsset, TurnHistoryMessage};
use serde_json::{json, Value};

fn has_native_tool_interactions(messages: &[Value]) -> bool {
    messages.iter().any(|msg| {
        msg.get("role").and_then(Value::as_str) == Some("tool")
            || msg
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty())
    })
}

pub(super) fn trim_turn_history_to_turn_boundary(
    mut history: Vec<TurnHistoryMessage>,
) -> Vec<TurnHistoryMessage> {
    match history.iter().position(|message| message.role == "user") {
        Some(0) => history,
        Some(index) => {
            history.drain(0..index);
            history
        }
        None => Vec::new(),
    }
}

pub(super) fn recent_attachment_assets(assets: &[AttachmentAsset], limit: usize) -> Vec<AttachmentAsset> {
    let mut recent = assets.to_vec();
    recent.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
    recent.truncate(limit);
    recent
}

fn recent_transcript_slice(transcript: &[Value], limit: usize) -> Vec<Value> {
    let start = transcript.len().saturating_sub(limit);
    trim_native_to_turn_boundary(transcript[start..].to_vec())
}


pub(super) fn truncate_native_messages(
    transcript: Vec<Value>,
    reserved_messages: &[Value],
    input_budget_tokens: Option<usize>,
) -> (Vec<Value>, usize) {
    let Some(input_budget_tokens) = input_budget_tokens else {
        return (transcript, 0);
    };
    let reserved_tokens = reserved_messages
        .iter()
        .map(estimate_native_message_tokens)
        .sum::<usize>();

    if reserved_tokens >= input_budget_tokens {
        return (Vec::new(), transcript.len());
    }

    let transcript = trim_native_to_turn_boundary(transcript);
    let original_len = transcript.len();
    let mut remaining_budget = input_budget_tokens - reserved_tokens;
    let mut kept_turns = Vec::new();

    for turn in split_native_turns(transcript).into_iter().rev() {
        let turn_tokens = estimate_native_turn_tokens(&turn);
        if turn_tokens > remaining_budget {
            // 防御预算击穿（P2-5）：若最新一轮自身即超出剩余预算，尝试保留其基础问答骨架，
            // 避免 kept_turns 为空导致整个历史全部归零蒸发。
            if kept_turns.is_empty() {
                let skeleton: Vec<Value> = turn
                    .into_iter()
                    .filter(|msg| {
                        let role = msg.get("role").and_then(Value::as_str);
                        role == Some("user") || role == Some("assistant")
                    })
                    .collect();
                if !skeleton.is_empty()
                    && estimate_native_turn_tokens(&skeleton) <= remaining_budget
                {
                    kept_turns.push(skeleton);
                }
            }
            break;
        }

        remaining_budget -= turn_tokens;
        kept_turns.push(turn);
    }

    kept_turns.reverse();
    let kept = kept_turns.into_iter().flatten().collect::<Vec<_>>();
    let truncated_count = original_len.saturating_sub(kept.len());

    (kept, truncated_count)
}

pub(super) fn trim_history_to_turn_boundary(mut history: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    match history
        .iter()
        .position(|message| matches!(message.role, ProviderRole::User))
    {
        Some(0) => history,
        Some(index) => {
            history.drain(0..index);
            history
        }
        None => Vec::new(),
    }
}

fn trim_native_to_turn_boundary(mut transcript: Vec<Value>) -> Vec<Value> {
    match transcript.iter().position(is_native_user_message) {
        Some(0) => transcript,
        Some(index) => {
            transcript.drain(0..index);
            transcript
        }
        None => Vec::new(),
    }
}

fn split_native_turns(transcript: Vec<Value>) -> Vec<Vec<Value>> {
    let mut turns = Vec::new();
    let mut current_turn = Vec::new();

    for message in transcript {
        if is_native_user_message(&message) {
            if !current_turn.is_empty() {
                turns.push(current_turn);
            }
            current_turn = vec![message];
            continue;
        }

        if !current_turn.is_empty() {
            current_turn.push(message);
        }
    }

    if !current_turn.is_empty() {
        turns.push(current_turn);
    }

    turns
}

fn estimate_native_turn_tokens(turn: &[Value]) -> usize {
    turn.iter().map(estimate_native_message_tokens).sum()
}

fn is_native_user_message(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
}


/// 将 TurnHistoryMessage 列表转为 provider-native 格式的消息列表。
/// 用于在 provider_native_transcript 为空时，从 history 构建 fallback 上下文，
/// 确保切换到 native-tool-flow 模型时不会丢失此前标准模型轮次的历史对话。
pub(super) fn history_to_native_messages(history: &[TurnHistoryMessage]) -> Vec<Value> {
    history
        .iter()
        .map(|msg| {
            // wave-3 L3-B：native transcript 同样不得携带已落盘的 DSML 协议原文。
            let content = sanitize_dsml_protocol_text(msg.content.trim());
            json!({
                "role": msg.role,
                "content": content,
            })
        })
        .collect()
}

/// 取 native 中最后一条 user 的可锚定文本；多模态 content 数组不参与锚定。
fn native_user_anchor_text(native_messages: &[Value]) -> Option<String> {
    native_messages
        .iter()
        .rev()
        .find(|msg| msg.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|msg| msg.get("content").and_then(Value::as_str))
        .map(|content| sanitize_dsml_protocol_text(content.trim()))
}

/// native 中最后一条非检查点 assistant 文本，用于重复 user 文案的二次消歧。
fn native_last_assistant_text(native_messages: &[Value]) -> Option<String> {
    native_messages
        .iter()
        .rev()
        .filter(|msg| msg.get("role").and_then(Value::as_str) == Some("assistant"))
        .find_map(|msg| {
            let content = msg.get("content").and_then(Value::as_str)?;
            if content.contains("[中断轮次检查点") {
                return None;
            }
            let normalized = sanitize_dsml_protocol_text(content.trim());
            (!normalized.is_empty()).then_some(normalized)
        })
}

/// 对齐 native transcript 与 history，防止历史混合轮次中纯文本轮次丢失（ADR 0123 / P1-2）。
/// 核心契约：
/// 1. 基于 native_messages 中最后一条 user 消息的内容在 history 中倒序定位锚点（Tail Identity Anchoring）；
/// 2. 重复文案必须由 native 末条 assistant 内容二次校验消歧，无法消歧时保守不动；
/// 3. 仅当定位成功且锚点之后存在未落入 native 的尾部新轮次时，才截取并追加；
/// 4. 天然免疫 400 条滑动窗口截断（不会因绝对轮次计数偏小而误重复注入）；
/// 5. 多模态/结构异常无法文本锚定时，已有工具交互必须原样保留，绝不整体替换。
fn align_native_transcript_with_history(
    native_messages: Vec<Value>,
    history: &[TurnHistoryMessage],
) -> Vec<Value> {
    if history.is_empty() {
        return native_messages;
    }
    if native_messages.is_empty() {
        return history_to_native_messages(history);
    }

    let Some(last_native_user_content) = native_user_anchor_text(&native_messages) else {
        // 多模态/结构异常无法文本锚定：已有工具交互必须原样保留，绝不整体替换丢轮次。
        return if has_native_tool_interactions(&native_messages) {
            native_messages
        } else {
            history_to_native_messages(history)
        };
    };

    let candidates: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, msg)| {
            msg.role == "user"
                && sanitize_dsml_protocol_text(msg.content.trim()) == last_native_user_content
        })
        .map(|(idx, _)| idx)
        .collect();

    if candidates.is_empty() {
        // 异构会话或深度重写：保守保持 native，绝不追加未验证轮次。
        return native_messages;
    }

    let anchor_idx = if candidates.len() == 1 {
        candidates[0]
    } else {
        // 重复文案（如「继续」）必须由 native 末条 assistant 内容二次校验消歧；
        // 无法消歧时保守不动——宁可少追加一轮，也不丢中间轮或重复注入。
        let Some(expected_assistant) = native_last_assistant_text(&native_messages) else {
            return native_messages;
        };
        let matched = candidates.iter().rev().copied().find(|&idx| {
            history[idx + 1..]
                .iter()
                .take_while(|msg| msg.role != "user")
                .any(|msg| {
                    msg.role == "assistant"
                        && sanitize_dsml_protocol_text(msg.content.trim()) == expected_assistant
                })
        });
        let Some(idx) = matched else {
            return native_messages;
        };
        idx
    };

    // 检查在 anchor_idx 之后是否还有更晚的 user 轮次（未落入 native 的纯文本尾轮）
    let next_user_offset = history[anchor_idx + 1..]
        .iter()
        .position(|msg| msg.role == "user");

    if let Some(offset) = next_user_offset {
        let append_start_idx = anchor_idx + 1 + offset;
        let tail_native = history_to_native_messages(&history[append_start_idx..]);
        let mut merged = native_messages;
        merged.extend(tail_native);
        return merged;
    }

    // 锚点之后无新 user 消息，说明 native 已包含最新交互，无需追加
    native_messages
}

/// 读侧唯一入口（ADR 0123）：滑动切窗 → 就地修剪 → 脏数据自愈 → 与纯文本历史尾部对齐。
pub(super) fn load_native_transcript_messages(
    transcript: &[Value],
    limit: usize,
    recent_history: &[TurnHistoryMessage],
) -> Vec<Value> {
    let mut messages = recent_transcript_slice(transcript, limit);
    crate::agent::tool_pruner::prune_native_messages_in_place(&mut messages);
    let recovered = if crate::agent::tool_pruner::needs_tool_roundtrip_repair(&messages) {
        crate::agent::tool_pruner::recover_incomplete_tool_roundtrips(&messages)
    } else {
        messages
    };
    align_native_transcript_with_history(recovered, recent_history)
}

/// 请求装配协议守卫（ADR 0123）：原生工具流回放，或 OpenAI 工具模型已有原生工具交互。
pub(super) fn should_use_native_tool_flow(
    provider: &ProviderManager,
    retrieved: &crate::agent::context::RetrievedContextState,
) -> bool {
    provider.requires_provider_native_tool_flow()
        || (provider.supports_native_tool_transcript_replay()
            && has_native_tool_interactions(&retrieved.transcript.provider_native_messages))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_native_transcript_appends_tail_plain_turns_from_history() {
        let native_messages = vec![
            json!({ "role": "user", "content": "第一轮提问" }),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "ls", "arguments": "{}" } }]
            }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "file1" }),
            json!({ "role": "assistant", "content": "第一轮回答" }),
        ];

        let history = vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "第一轮提问".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "第一轮回答".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "第二轮纯文本提问".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "第二轮纯文本回答".to_string(),
                ..Default::default()
            },
        ];

        let aligned = align_native_transcript_with_history(native_messages, &history);
        // 原有 4 条 + 补齐的 2 条 = 6 条
        assert_eq!(aligned.len(), 6);
        assert_eq!(aligned[4].get("role").and_then(Value::as_str), Some("user"));
        assert_eq!(
            aligned[4].get("content").and_then(Value::as_str),
            Some("第二轮纯文本提问")
        );
        assert_eq!(
            aligned[5].get("role").and_then(Value::as_str),
            Some("assistant")
        );
        assert_eq!(
            aligned[5].get("content").and_then(Value::as_str),
            Some("第二轮纯文本回答")
        );
    }

    #[test]
    fn test_align_native_transcript_no_duplicate_on_sliding_window_slice() {
        // 模拟 100 轮历史
        let mut history = Vec::new();
        for i in 0..100 {
            history.push(TurnHistoryMessage {
                role: "user".to_string(),
                content: format!("问题_{i}"),
                ..Default::default()
            });
            history.push(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: format!("回答_{i}"),
                ..Default::default()
            });
        }

        // 模拟 native transcript 被滑动窗口截断，仅保留最后 20 轮（80~99）
        let mut native_messages = Vec::new();
        for i in 80..100 {
            native_messages.push(json!({ "role": "user", "content": format!("问题_{i}") }));
            native_messages.push(json!({ "role": "assistant", "content": format!("回答_{i}") }));
        }

        let original_len = native_messages.len(); // 40
        let aligned = align_native_transcript_with_history(native_messages, &history);

        // 核心断言：绝不能重复注入！长度必须保持 40，最后一条问题依然是 问题_99，且 问题_99 仅出现 1 次
        assert_eq!(
            aligned.len(),
            original_len,
            "滑动窗口截断下绝不能重复注入已有的历史！"
        );
        let q99_count = aligned
            .iter()
            .filter(|m| m.get("content").and_then(Value::as_str) == Some("问题_99"))
            .count();
        assert_eq!(q99_count, 1, "问题_99 只能出现 1 次");
    }

    #[test]
    fn test_align_native_transcript_disambiguates_repeated_user_text_by_assistant() {
        let native_messages = vec![
            json!({ "role": "user", "content": "继续" }),
            json!({ "role": "assistant", "content": "第一轮回答" }),
        ];

        let history = vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "继续".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "第一轮回答".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "继续".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "第二轮回答".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "第三轮提问".to_string(),
                ..Default::default()
            },
        ];

        let aligned = align_native_transcript_with_history(native_messages, &history);
        // 锚点必须落在第一处「继续」（assistant 内容二次校验），否则会丢掉第二轮回答。
        assert_eq!(aligned.len(), 5);
        let contents: Vec<&str> = aligned
            .iter()
            .filter_map(|msg| msg.get("content").and_then(Value::as_str))
            .collect();
        assert_eq!(
            contents,
            vec!["继续", "第一轮回答", "继续", "第二轮回答", "第三轮提问"]
        );
    }

    #[test]
    fn test_align_native_transcript_conservative_when_repeated_text_without_assistant_anchor() {
        let native_messages = vec![
            json!({ "role": "user", "content": "继续" }),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "id": "call_x", "type": "function", "function": { "name": "f", "arguments": "{}" } }]
            }),
            json!({ "role": "tool", "tool_call_id": "call_x", "content": "ok" }),
        ];

        let history = vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "继续".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "第一轮回答".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "继续".to_string(),
                ..Default::default()
            },
        ];

        // 无法用 assistant 文本消歧时必须保守不动，绝不猜测性追加导致中间轮丢失。
        let aligned = align_native_transcript_with_history(native_messages, &history);
        assert_eq!(aligned.len(), 3);
        assert_eq!(aligned[2].get("role").and_then(Value::as_str), Some("tool"));
    }

    #[test]
    fn test_align_native_transcript_keeps_native_when_multimodal_anchor_unavailable() {
        let native_messages = vec![
            json!({
                "role": "user",
                "content": [
                    { "type": "text", "text": "看图" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,abc" } }
                ]
            }),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "id": "call_img", "type": "function", "function": { "name": "f", "arguments": "{}" } }]
            }),
            json!({ "role": "tool", "tool_call_id": "call_img", "content": "ok" }),
            json!({ "role": "assistant", "content": "已处理图片" }),
        ];

        let history = vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "看图".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "已处理图片".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "尾部新提问".to_string(),
                ..Default::default()
            },
        ];

        let aligned = align_native_transcript_with_history(native_messages, &history);
        // 已有工具交互必须原样保留，绝不能被 history 纯文本整体替换（会丢工具轮）。
        assert_eq!(aligned.len(), 4);
        assert!(aligned
            .iter()
            .any(|msg| msg.get("role").and_then(Value::as_str) == Some("tool")));
        assert!(!aligned
            .iter()
            .any(|msg| msg.get("content").and_then(Value::as_str) == Some("尾部新提问")));
    }
}
