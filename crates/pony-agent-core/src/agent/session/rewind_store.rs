//! ADR 0113「撤回」：会话侧的轮次定位与「空会话 checkpoint 根」补齐。
//!
//! 为什么独立成文件：`session/store.rs` 已计入 `scripts/file-size-baseline.json` 的文件行数
//! 棘轮（净值只许减不许增），本模块新增的撤回编排不复用其预算；只为跨模块调用把
//! `store.rs` 里 `new_history_node_id` 的可见性放开到 `pub(super)`（行数中性）。
//!
//! 契约：
//! - 活动链 = 从 `branch_head_node_id`（缺失才回退 `visible_node_id`）沿 `parent_node_id`
//!   回溯后**反转**（旧→新）。只有这条链上的节点才是「用户可撤回的轮次」；
//! - 撤回到某轮 = checkout 到该轮的**父节点**；父节点为空 ⇒ 需要一个「空会话 checkpoint
//!   根」承接（[`SessionStore::ensure_rewind_root_checkpoint`]，幂等）；
//! - 本模块只做**定位/补根**，不改历史内容；真正的截断仍由既有
//!   `checkout_history_node` 真值路径执行（事件水位 + 分支头前移）。

use super::store::{new_history_node_id, now_timestamp_ms, SessionStore};
use super::types::{
    HistoryCursorMode, HistoryNode, HistoryNodeKind, SessionState, WorkspaceRef,
    DEFAULT_HISTORY_BRANCH_ID, DEFAULT_SESSION_ID, DEFAULT_SESSION_SUMMARY, DEFAULT_SESSION_TITLE,
};

/// 一次撤回的定位结果（只读事实，供控制面编排文件恢复 + checkout）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindPlan {
    /// 被撤掉的那一轮的历史节点 id。
    pub target_node_id: String,
    /// 被撤掉那一轮的 turn id（文件快照键之一；缺失 = 旧数据无 turn 归属）。
    pub target_turn_id: Option<String>,
    /// 被撤掉那一轮的节点标题（派生自该轮首条用户消息）。
    pub target_title: String,
    /// 该轮在活动链上的序号（1 起，旧→新）。
    pub turn_index: usize,
    /// 该轮的用户消息正文（撤回后回填输入框的真值；`None` = 历史缺正文）。
    pub user_message: Option<String>,
    /// 撤回归宿：目标节点的父节点；`None` = 撤回到空会话。
    pub keep_node_id: Option<String>,
    /// 会被撤销的轮次 id（旧→新）——文件快照恢复按此升序取「最早 pre-image」。
    pub cancelled_turn_ids: Vec<String>,
}

impl SessionStore {
    /// 当前会话活动分支链（旧→新）。
    pub fn rewind_activity_chain(&mut self, session_id: Option<&str>) -> Vec<HistoryNode> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let (nodes, _branches, cursor) = self.load_history_graph(Some(session_key.as_str()));
        let head = cursor
            .branch_head_node_id
            .clone()
            .or_else(|| cursor.visible_node_id.clone());
        let Some(head) = head else {
            return Vec::new();
        };
        let index: std::collections::HashMap<&str, &HistoryNode> = nodes
            .iter()
            .map(|node| (node.node_id.as_str(), node))
            .collect();
        let mut chain: Vec<HistoryNode> = Vec::new();
        let mut cursor = Some(head);
        // 环保护：节点表长度 + 1 足够覆盖任何合法链。
        while let Some(node_id) = cursor {
            let Some(node) = index.get(node_id.as_str()) else {
                break;
            };
            chain.push((*node).clone());
            if chain.len() > nodes.len() {
                break;
            }
            cursor = node.parent_node_id.clone();
        }
        chain.reverse();
        chain
    }

    /// 活动链上的**已提交用户轮次**（旧→新，撤回菜单的候选集）。
    pub fn rewind_turns(&mut self, session_id: Option<&str>) -> Vec<HistoryNode> {
        self.rewind_activity_chain(session_id)
            .into_iter()
            .filter(|node| matches!(node.kind, HistoryNodeKind::TurnCommitted))
            .collect()
    }

    /// 解析「撤回到 `node_id` 那一轮之前」的计划（纯读）。
    pub fn rewind_plan(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
    ) -> Result<RewindPlan, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let chain = self.rewind_activity_chain(Some(session_key.as_str()));
        if chain.is_empty() {
            return Err("当前会话还没有可撤回的轮次".to_string());
        }
        let position = chain
            .iter()
            .position(|node| node.node_id == node_id)
            .ok_or_else(|| format!("历史节点不在当前活动分支：{node_id}"))?;
        let target = chain[position].clone();
        if !matches!(target.kind, HistoryNodeKind::TurnCommitted) {
            return Err("只能撤回到已提交的用户轮次".to_string());
        }
        let cancelled_turn_ids: Vec<String> = chain[position..]
            .iter()
            .filter_map(|node| node.turn_id.clone())
            .collect();
        let user_message = self.rewind_user_message_for_node(&session_key, &target.node_id);
        // 轮次序号按**已提交用户轮次**计（活动链上还有 checkpoint 根等非轮次节点）。
        let turn_index = chain[..=position]
            .iter()
            .filter(|node| matches!(node.kind, HistoryNodeKind::TurnCommitted))
            .count();
        Ok(RewindPlan {
            target_node_id: target.node_id.clone(),
            target_turn_id: target.turn_id.clone(),
            target_title: target.title.clone(),
            turn_index,
            user_message,
            keep_node_id: target.parent_node_id.clone(),
            cancelled_turn_ids,
        })
    }

    /// 某节点视角下的用户消息正文（最近一条非空 user 消息，事件折叠/内嵌快照同口径）。
    pub fn rewind_user_message_for_node(
        &mut self,
        session_id: &str,
        node_id: &str,
    ) -> Option<String> {
        if !self.sessions.contains_key(session_id) {
            return None;
        }
        let snapshot = self.snapshot_for_session_at(session_id, Some(node_id));
        snapshot
            .history
            .iter()
            .rev()
            .find(|message| message.role == "user" && !message.content.trim().is_empty())
            .map(|message| message.content.clone())
    }

    /// 补齐「空会话 checkpoint 根」并返回其 id（幂等）。
    ///
    /// 撤回到**第一轮之前**必须有一个可 checkout 的节点；本方法在缺根时插入一个
    /// 空历史 `Checkpoint` 节点，并把现有无父节点挂到它下面，同时把分支 base 指向它。
    pub fn ensure_rewind_root_checkpoint(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<String, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        // `load_history_graph` 会顺带 `ensure_history_graph`，保证 legacy 会话已建好节点表。
        let _ = self.load_history_graph(Some(session_key.as_str()));
        if !self.sessions.contains_key(&session_key) {
            return Err(format!("会话不存在：{session_key}"));
        }

        let existing_root = {
            let session = self
                .sessions
                .get(&session_key)
                .expect("session existence checked above");
            session
                .history_nodes
                .iter()
                .find(|node| {
                    node.parent_node_id.is_none()
                        && node.turn_id.is_none()
                        && node.history.is_empty()
                        && node.event_seq_range.is_none()
                })
                .map(|node| node.node_id.clone())
        };
        if let Some(node_id) = existing_root {
            return Ok(node_id);
        }

        let node_id = {
            let session: &mut SessionState = self
                .sessions
                .get_mut(&session_key)
                .expect("session existence checked above");
            let created_at_ms = session
                .history_nodes
                .iter()
                .map(|node| node.created_at_ms)
                .min()
                .unwrap_or_else(now_timestamp_ms)
                .saturating_sub(1);
            let branch_id = session
                .history_nodes
                .first()
                .map(|node| node.branch_id.clone())
                .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
            let node_id = new_history_node_id(session, 0, created_at_ms);
            let root = HistoryNode {
                node_id: node_id.clone(),
                session_id: session.conversation_id.clone(),
                parent_node_id: None,
                branch_id: branch_id.clone(),
                forked_from_node_id: None,
                kind: HistoryNodeKind::Checkpoint,
                run_id: None,
                workspace_ref: WorkspaceRef::default(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                turn_trace_history: Vec::new(),
                turn_id: None,
                turn_trace_refs: Some(Vec::new()),
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                turn_count: 0,
                last_referenced_file: None,
                event_seq_range: None,
                created_at_ms,
            };
            for node in session.history_nodes.iter_mut() {
                if node.parent_node_id.is_none() {
                    node.parent_node_id = Some(node_id.clone());
                }
            }
            if let Some(branch) = session
                .history_branches
                .iter_mut()
                .find(|branch| branch.branch_id == branch_id)
            {
                if branch.base_node_id.is_none() {
                    branch.base_node_id = Some(node_id.clone());
                }
            }
            // 光标保持指向当前可见节点（不把视图切到空根）。
            if session.history_cursor.mode == HistoryCursorMode::Live
                && session.history_cursor.visible_node_id.is_none()
            {
                session.history_cursor.visible_node_id = Some(node_id.clone());
            }
            session.history_nodes.insert(0, root);
            node_id
        };
        self.save_to_backend();
        Ok(node_id)
    }
}
