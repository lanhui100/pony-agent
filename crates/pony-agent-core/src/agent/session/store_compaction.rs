// store_compaction: 上下文压缩的**唯一写入口**（`SessionStore::commit_compaction`）。
//
// 单独成文件的两个理由：
// 1. `store.rs` 有 file-size 基线（净值不得增长），压缩提交是一段成规模的新逻辑；
// 2. 语义内聚：一次压缩 = 一条 append-only 日志命令（`CompactHistory`）+ 一次 blob 落盘，
//    与「每轮追加」的 `append_turn` 是两条**不同写语义**，混在一个巨文件里更容易走偏。

use super::super::backend::{PersistCommand, PersistCommandOutcome};
use super::super::types::{
    HistoryNodeKind, SessionState, TurnHistoryMessage, DEFAULT_HISTORY_BRANCH_ID,
    DEFAULT_SESSION_ID,
};
use super::super::SessionStore;
use super::{commit_history_node_from_live_state, refresh_session_metadata};
use crate::agent::compression::derive_squash_base_seq;
use crate::agent::projection::{fold_all_with_branches, HistoryProjectionState};
use crate::agent::turn_event::TurnEvent;
use serde_json::Value;
use std::collections::HashMap;

impl SessionStore {
    /// 上下文压缩提交（**唯一写入口**：自动压缩与 `/compact` 都走这里）。
    ///
    /// 一次压缩 = 一条 append-only 日志命令（`CompactHistory`）+ 一次 blob 落盘：
    /// - 命令携带**压缩后的可见节点快照**（会话历史的可见视图由它派生）与**整份替换的
    ///   原生 transcript**；装载以日志为真源，不落这条命令 ⇒ 重启后压缩结果被日志里的
    ///   旧历史覆盖（「压缩了，但重启就回到未压缩」）；
    /// - 内存与 blob 同步改写，保证进程内视图与重启后视图一致。
    ///
    /// 落盘失败只降级告警（与其它热路径写盘一致）：压缩不是回合正确性的前置条件。
    pub fn commit_compaction(
        &mut self,
        session_id: Option<&str>,
        compacted_history: Vec<TurnHistoryMessage>,
        compacted_transcript: Vec<Value>,
        summary_message: String,
    ) {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let mut staged = self.ensure_session(&session_key).clone();
        staged.history = compacted_history;
        staged.provider_native_transcript = compacted_transcript.clone();
        refresh_session_metadata(&mut staged, true);
        commit_history_node_from_live_state(&mut staged, HistoryNodeKind::TurnCommitted, None);
        let summary_for_event = summary_message.clone();
        if self.backend.supports_turn_events() {
            if let Some(node) = staged.history_nodes.last().cloned() {
                let commands = vec![
                    PersistCommand::CompactHistory {
                        epoch: 1,
                        session_id: session_key.clone(),
                        node,
                        transcript: compacted_transcript,
                        summary_message,
                    },
                    PersistCommand::UpdateCursor {
                        epoch: 1,
                        session_id: session_key.clone(),
                        cursor: staged.history_cursor.clone(),
                    },
                ];
                let outcome = self.backend.persist_commands_batch(commands);
                if !matches!(
                    outcome,
                    PersistCommandOutcome::Succeeded | PersistCommandOutcome::LogPersisted
                ) {
                    eprintln!(
                        "[pony-agent][session] compaction log persist failed: {outcome:?}; \
                         falling back to blob (索引滞后将由重建补齐)"
                    );
                }
            }
            // ①压缩必须**可重放**：边界与摘要同时进事件日志（见方法文档）。
            self.append_compaction_squash_event(&session_key, &staged, &summary_for_event);
        }
        self.sessions.insert(session_key.clone(), staged);
        let persisted = self.sessions.get(&session_key).cloned();
        if let Some(persisted) = persisted {
            self.save_session_state_to_backend(&session_key, &persisted);
        }
    }

    /// ①压缩可重放：把「压缩边界 + 摘要」落成一条 append-only 的 `history/squash` 事件。
    ///
    /// 只写 blob/索引时，「压到哪」只有派生视图知道，事件日志不知道。一旦索引投影丢失
    /// （自愈路径「投影空 → 按事件重建历史」），折叠就会把被压缩的旧轮次整段复活——压缩
    /// 等于白做，且上下文会被顶回预算上限。事件化后重放与进程内视图同口径
    /// （对齐 dsh 的 `compaction/summary` 落日志 + `surfaceOp:replace`）。
    fn append_compaction_squash_event(
        &self,
        session_key: &str,
        state: &SessionState,
        summary_message: &str,
    ) {
        if summary_message.trim().is_empty() {
            // 旧入口 `replace_session_history` 不带摘要：没有摘要就没有「压缩语义」，
            // 不写空摘要事件（空摘要注入会污染历史）。
            return;
        }
        // 入参契约：压缩后历史的第一条就是摘要本身，其余为保留窗口。
        let retained_start = usize::from(
            state
                .history
                .first()
                .is_some_and(|message| message.content == summary_message),
        );
        let retained = state.history.get(retained_start..).unwrap_or(&[]);
        let folded = self.folded_history_records(session_key, state);
        let base_seq = derive_squash_base_seq(&folded, retained);
        let branch_id = state
            .history_cursor
            .active_branch_id
            .clone()
            .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
        let persisted = self.persist_events(
            session_key,
            "compaction",
            &branch_id,
            vec![TurnEvent::HistorySquash {
                base_seq,
                summary_message: summary_message.to_string(),
            }],
        );
        if !persisted {
            eprintln!(
                "[pony-agent][session] compaction squash event persist failed: \
                 session={session_key} base_seq={base_seq} —— 重启后按事件重建可能复活被压缩历史"
            );
        }
    }

    /// 折叠当前事件流并保留 seq（压缩边界推导的唯一数据源）。
    ///
    /// 复用 `fold_all_with_branches` 的血缘判据：压缩对象是**活动分支**的历史，
    /// 拿全分支事件推导边界会把被放弃 fork 的轮次算进来（边界落错位置）。
    fn folded_history_records(
        &self,
        session_key: &str,
        state: &SessionState,
    ) -> Vec<(u64, TurnHistoryMessage)> {
        let events = self.load_turn_events(session_key, None);
        if events.is_empty() {
            return Vec::new();
        }
        let initial_branch = state
            .history_cursor
            .active_branch_id
            .clone()
            .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
        let node_branch: HashMap<&str, &str> = state
            .history_nodes
            .iter()
            .map(|node| (node.node_id.as_str(), node.branch_id.as_str()))
            .collect();
        let projected = fold_all_with_branches::<HistoryProjectionState, HistoryProjectionState>(
            &events,
            &initial_branch,
            &node_branch,
            &state.history_branches,
        );
        projected.messages_with_seq().to_vec()
    }

    /// 仅替换文本历史（**历史遗留入口**；生产压缩一律走 [`Self::commit_compaction`]）。
    ///
    /// 保留它是为了让「只改历史、不动 transcript」的集成测试仍能表达原语义；实现委托到
    /// 压缩写入口，因此两者共享同一条写路径（不会出现「一个落日志、一个只写 blob」）。
    pub fn replace_session_history(
        &mut self,
        session_id: Option<&str>,
        new_history: Vec<TurnHistoryMessage>,
    ) {
        let transcript = self
            .snapshot_at_readonly(session_id, None, &[])
            .provider_native_transcript;
        self.commit_compaction(session_id, new_history, transcript, String::new());
    }
}
