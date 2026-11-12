//! PA-174 P1：从 durable `turn_events` 重建会话历史的纯函数。
//!
//! 背景（真实事故会话 `pa_01a1ba70`）：停写 blob 后索引里只剩 `__index_only` 轻量行，
//! `normalized_messages` 零行 ⇒ 投影侧历史为空，装载后 `history: []`（重启后历史为零、
//! 请求里零 carry，UI 看起来像换了新会话）。`turn_events` 是 append-only 事实层，仍完整
//! 保留全部 turn 的消息事件，因此「投影侧全空」时必须由事件日志重建历史，而不是把空壳
//! 当作真相返回。
//!
//! 折叠口径**只有一份**：本模块不实现自己的消息映射，直接复用
//! [`HistoryProjectionState`]（读路径与写路径、重启与进程内共用同一套折叠）。此前这里
//! 另有一份 `message_from_turn_event`，只认 `user/message` 与 `assistant/message`，于是
//! 与正式投影产生两处分叉：
//! - `history/squash` 被忽略 ⇒ 压缩过的会话一旦走到「投影空 → 事件重建」，被压缩的旧
//!   历史整段复活（压缩白做）；
//! - 失败轮只有 `assistant/chunk` 没有 `assistant/message` ⇒ 折叠出「悬空的 user」，
//!   重启后模型看不到该轮做过什么（`pa_01a1c4d0` 的事故形态换了一种形状复现）。
//!
//! PA-174 B1（审查整改）：重建**必须按活动血缘过滤**。`turn_events` 同时容纳所有分支的
//! 事件（main / fork-*），只按 seq 全量取会把被放弃 fork 的轮次复活；且装载侧会把重建
//! 结果经 `refresh_session_metadata` 写回索引（错误会被持久化）。分支可见性判定**复用**
//! `projection::fold_all_with_branches`（既有唯一判据：血缘链 + `checkpoint/checkout` 重放），
//! 本模块只提供「事件流 → 消息历史」这一层的入口，不复制第二套血缘逻辑。

use std::collections::HashMap;

use crate::agent::projection::{fold_all_with_branches, HistoryProjectionState, Projection};
use crate::agent::session::{HistoryBranch, TurnHistoryMessage};
use crate::agent::turn_event::TurnEvent;

/// 按事件顺序（装载侧已 `ORDER BY seq`）把事件流还原成会话历史。
///
/// 入参必须携带 **seq**：`history/squash` 的边界判据是「丢弃 seq <= base_seq 的消息」，
/// 丢掉 seq 就等于丢掉压缩边界（这也是本函数签名从 `&[TurnEvent]` 改为
/// `&[(u64, TurnEvent)]` 的唯一原因）。
///
/// 口径（与 [`HistoryProjectionState`] 完全一致）：
/// - `user/message`、`assistant/message`、失败轮的过程累积（chunk + 工具名）、
///   `history/squash` 参与折叠，其余事件忽略；
/// - 空文本消息忽略——空历史项会污染下一轮 carry，且下游按空文本判无效；
/// - 顺序 = 传入顺序，不重排；
/// - 空事件流 ⇒ 空历史（不造默认消息、不 panic）。
///
/// 注意：本函数**不做分支过滤**（入参只有事件、没有血缘），装载侧只用它处理「事件流只有
/// 单一分支」的旧形态；有分支元数据时走 [`history_from_branch_events`]。
pub fn history_from_turn_events(events: &[(u64, TurnEvent)]) -> Vec<TurnHistoryMessage> {
    let mut state = HistoryProjectionState::init();
    for (seq, event) in events {
        HistoryProjectionState::apply(&mut state, *seq, event);
    }
    state.messages()
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
    fold_all_with_branches::<HistoryProjectionState, HistoryProjectionState>(
        events,
        initial_branch,
        node_branch,
        branches,
    )
    .messages()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::turn_event::TurnEndReason;

    fn seq_events(events: Vec<TurnEvent>) -> Vec<(u64, TurnEvent)> {
        events
            .into_iter()
            .enumerate()
            .map(|(seq, event)| (seq as u64, event))
            .collect()
    }

    #[test]
    fn history_from_turn_events_keeps_only_message_events_in_order() {
        let events = seq_events(vec![
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
            TurnEvent::TurnEnd {
                turn_id: "turn-1".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
                error: None,
            },
        ]);

        let history = history_from_turn_events(&events);

        let pairs: Vec<(&str, &str)> = history
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str()))
            .collect();
        assert_eq!(pairs, vec![("user", "问"), ("assistant", "答")]);
        assert!(history_from_turn_events(&[]).is_empty());
    }

    /// ①压缩边界必须被重建路径尊重：`history/squash` 之后，被压缩的消息不得复活。
    #[test]
    fn history_from_turn_events_honours_history_squash() {
        let events = seq_events(vec![
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "旧问题".to_string(),
                attachments: Vec::new(),
            },
            TurnEvent::AssistantMessage {
                turn_id: "t1".to_string(),
                step: 0,
                text: "旧回答".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
            TurnEvent::UserMessage {
                turn_id: "t2".to_string(),
                text: "保留问题".to_string(),
                attachments: Vec::new(),
            },
            TurnEvent::HistorySquash {
                // 压缩点落在 t2 之前 ⇒ 只有 t1 被压缩。
                base_seq: 1,
                summary_message: "[会话摘要] 旧问题已讨论".to_string(),
            },
        ]);

        let history = history_from_turn_events(&events);
        let contents: Vec<&str> = history
            .iter()
            .map(|message| message.content.as_str())
            .collect();

        assert_eq!(
            contents,
            vec!["[会话摘要] 旧问题已讨论", "保留问题"],
            "squash 之后被压缩轮次不得复活，摘要必须注入且为 user 角色"
        );
        assert_eq!(history[0].role, "user");
    }

    /// ②失败轮不得折成悬空 user：没有 assistant/message 时用过程累积合成中断检查点。
    #[test]
    fn history_from_turn_events_synthesises_checkpoint_for_failed_turn() {
        let events = seq_events(vec![
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "搜索并总结".to_string(),
                attachments: Vec::new(),
            },
            TurnEvent::AssistantChunk {
                turn_id: "t1".to_string(),
                step: 0,
                text: "先看看".to_string(),
            },
            TurnEvent::ToolCall {
                turn_id: "t1".to_string(),
                step: 0,
                call_id: "c1".to_string(),
                name: "search".to_string(),
                arguments: "{}".to_string(),
                started_at_ms: None,
                description: None,
            },
            TurnEvent::TurnEnd {
                turn_id: "t1".to_string(),
                reason: TurnEndReason::Error,
                turn_duration_ms: None,
                error: None,
            },
        ]);

        let history = history_from_turn_events(&events);

        assert_eq!(
            history.len(),
            2,
            "失败轮必须补出 assistant 消息：{history:?}"
        );
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(
            history[1].status,
            Some(crate::agent::session::MessageStatus::Error)
        );
        assert!(
            history[1].content.contains("[中断轮次检查点]"),
            "检查点格式必须与运行时同构：{}",
            history[1].content
        );
        assert!(history[1].content.contains("先看看"));
        assert!(history[1].content.contains("search"));
    }

    /// 正常结束的轮次不得凭空多出一条合成消息。
    #[test]
    fn history_from_turn_events_keeps_completed_turn_without_checkpoint() {
        let events = seq_events(vec![
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "问".to_string(),
                attachments: Vec::new(),
            },
            TurnEvent::AssistantChunk {
                turn_id: "t1".to_string(),
                step: 0,
                text: "过程".to_string(),
            },
            TurnEvent::AssistantMessage {
                turn_id: "t1".to_string(),
                step: 0,
                text: "答".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
            TurnEvent::TurnEnd {
                turn_id: "t1".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
                error: None,
            },
        ]);

        let history = history_from_turn_events(&events);
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].content, "答");
        assert_eq!(
            history[1].status,
            Some(crate::agent::session::MessageStatus::Done)
        );
    }
}
