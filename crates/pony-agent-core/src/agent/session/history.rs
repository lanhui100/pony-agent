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
//!
//! PA-174 B1（审查整改）：重建**必须按活动血缘过滤**。`turn_events` 同时容纳所有分支的
//! 事件（main / fork-*），只按 seq 全量取会把被放弃 fork 的轮次复活；且装载侧会把重建
//! 结果经 `refresh_session_metadata` 写回索引（错误会被持久化）。分支可见性判定**复用**
//! `projection::fold_all_with_branches`（既有唯一判据：血缘链 + `checkpoint/checkout` 重放），
//! 本模块只提供「消息事件 → 历史项」这一层投影，不复制第二套血缘逻辑。

use std::collections::HashMap;

use crate::agent::projection::{fold_all_with_branches, Projection};
use crate::agent::session::{HistoryBranch, TurnHistoryMessage};
use crate::agent::turn_event::TurnEvent;

fn message_from_turn_event(event: &TurnEvent) -> Option<TurnHistoryMessage> {
    match event {
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
    }
}

/// 按事件顺序（装载侧已 `ORDER BY seq`）把消息事件还原成会话历史。
///
/// 口径（PA-174 契约冻结）：
/// - 只取 `user/message` 与 `assistant/message`；`assistant/chunk`、`tool/*`、`turn/*`、
///   trace 等一律忽略（chunk 是过程增量，重复折叠会产生半截重复消息）；
/// - 空文本（含纯空白）忽略——空历史项会污染下一轮 carry，且下游按空文本判无效；
/// - 顺序 = 传入顺序，不重排；
/// - 空事件流 ⇒ 空历史（不造默认消息、不 panic）。
///
/// 注意：本函数**不做分支过滤**（入参只有事件、没有血缘），装载侧只用它处理「事件流只有
/// 单一分支」的旧形态；有分支元数据时走 [`history_from_branch_events`]。
pub fn history_from_turn_events(events: &[TurnEvent]) -> Vec<TurnHistoryMessage> {
    events.iter().filter_map(message_from_turn_event).collect()
}

/// 消息事件投影：分支可见性由 `fold_all_with_branches` 判定，本类型只负责把可见的
/// 消息事件折叠成历史项（与 [`history_from_turn_events`] 同一口径）。
struct MessageHistoryProjection;

impl Projection<Vec<TurnHistoryMessage>> for MessageHistoryProjection {
    fn init() -> Vec<TurnHistoryMessage> {
        Vec::new()
    }

    fn apply(state: &mut Vec<TurnHistoryMessage>, _seq: u64, event: &TurnEvent) {
        if let Some(message) = message_from_turn_event(event) {
            state.push(message);
        }
    }
}

/// PA-174 B1：按**活动血缘**重建历史（复用 `fold_all_with_branches` 的可见性判据）。
///
/// - `initial_branch` = 会话 `historyCursor.activeBranchId`（缺省 `main`）；
/// - `node_branch` = `historyNodes` 的 `node_id -> branch_id`（`checkpoint/checkout` 重放用）；
/// - `branches` = `historyBranches`（血缘链 `forked_from_branch_id` 的来源）。
///
/// 非活动血缘的事件一律跳过 ⇒ 被放弃的 fork 轮次不会复活。
pub(crate) fn history_from_branch_events(
    events: &[(u64, String, TurnEvent)],
    initial_branch: &str,
    node_branch: &HashMap<&str, &str>,
    branches: &[HistoryBranch],
) -> Vec<TurnHistoryMessage> {
    fold_all_with_branches::<Vec<TurnHistoryMessage>, MessageHistoryProjection>(
        events,
        initial_branch,
        node_branch,
        branches,
    )
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
