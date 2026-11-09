//! PA-174 P1：从 durable `turn_events` 重建会话历史的纯函数。
//!
//! 背景（真实事故会话 `pa_01a1ba70`）：停写 blob 后索引里只剩 `__index_only` 轻量行，
//! `normalized_messages` 零行 ⇒ 投影侧历史为空，装载后 `history: []`（重启后历史为零、
//! 请求里零 carry，UI 看起来像换了新会话）。`turn_events` 是 append-only 事实层，仍完整
//! 保留全部 turn 的消息事件，因此「投影侧全空」时必须由事件日志重建历史，而不是把空壳
//! 当作真相返回。
//!
//! 判据口径：只锁「消息事件 → 历史项」这一层可观察语义，不做分支/压缩推导
//! （`history/squash`、turn 状态标记等由 `projection::HistoryProjectionState` 负责，
//! 本函数刻意保持无窗口、无 squash 语义：重建目标是「不返回空壳」）。

use crate::agent::session::TurnHistoryMessage;
use crate::agent::turn_event::TurnEvent;

/// 按事件顺序（装载侧已 `ORDER BY seq`）把消息事件还原成会话历史。
///
/// 口径（PA-174 契约冻结）：
/// - 只取 `user/message` 与 `assistant/message`；`assistant/chunk`、`tool/*`、`turn/*`、
///   trace 等一律忽略（chunk 是过程增量，重复折叠会产生半截重复消息）；
/// - 空文本（含纯空白）忽略——空历史项会污染下一轮 carry，且下游按空文本判无效；
/// - 顺序 = 传入顺序，不重排；
/// - 空事件流 ⇒ 空历史（不造默认消息、不 panic）。
pub fn history_from_turn_events(events: &[TurnEvent]) -> Vec<TurnHistoryMessage> {
    events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::UserMessage {
                turn_id,
                text,
                attachments,
            } if !text.trim().is_empty() => Some(TurnHistoryMessage {
                role: "user".to_string(),
                content: text.clone(),
                attachments: attachments.clone(),
                turn_id: Some(turn_id.clone()),
                status: None,
                model_name: None,
                token_count: None,
                reasoning_content: None,
            }),
            TurnEvent::AssistantMessage {
                turn_id,
                text,
                reasoning_content,
                usage,
                ..
            } if !text.trim().is_empty() => Some(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: text.clone(),
                attachments: Vec::new(),
                turn_id: Some(turn_id.clone()),
                status: None,
                model_name: None,
                // usage 仅消息元数据展示（不参与 MetricsProjection totals，防双计数）。
                token_count: usage.as_ref().and_then(|usage| usage.total_tokens),
                reasoning_content: reasoning_content.clone(),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_from_turn_events_keeps_only_message_events_in_order() {
        let events = vec![
            TurnEvent::UserMessage {
                turn_id: "turn-1".to_string(),
                text: "问".to_string(),
                attachments: Vec::new(),
            },
            TurnEvent::AssistantChunk {
                turn_id: "turn-1".to_string(),
                step: 1,
                text: "半截".to_string(),
            },
            TurnEvent::AssistantMessage {
                turn_id: "turn-1".to_string(),
                step: 1,
                text: "答".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ];

        let history = history_from_turn_events(&events);

        let triples: Vec<(&str, &str)> = history
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str()))
            .collect();
        assert_eq!(triples, vec![("user", "问"), ("assistant", "答")]);
        assert!(history_from_turn_events(&[]).is_empty());
    }
}
