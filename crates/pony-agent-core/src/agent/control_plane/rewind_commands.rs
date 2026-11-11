//! ADR 0113「撤回」（rewind）控制面命令。
//!
//! 契约：
//! - 撤销点是**用户轮次节点**：撤回到该轮之前 = checkout 到它的父节点（父为空则先补
//!   「空会话 checkpoint 根」，见 `SessionStore::ensure_rewind_root_checkpoint`）；
//! - 会话截断只走既有 `checkout_history_node` 真值路径（事件水位 + 分支头前移），本文件
//!   不复制任何截断逻辑；
//! - 文件还原由 [`RewindFileStore`] 负责（写工具落盘前的 pre-image），失败/跳过一律显式
//!   上报（`file_restore_degraded`），绝不假装成功；
//! - 先还原文件再截断会话：checkout 失败时用户至少拿回代码，且会话未被半截断。

use super::*;
use crate::agent::session::{RewindFileStore, RewindRestoreOutcome};

/// 撤回点点位查询（当前会话；`None` = 默认会话）。
#[derive(Clone, Default)]
pub struct RewindPointsQuery {
    pub session_id: Option<String>,
}

/// 一个可撤回的用户轮次（TUI 菜单行）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindPointView {
    pub node_id: String,
    pub turn_id: Option<String>,
    /// 轮次序号（1 起，旧→新，只数已提交用户轮次）。
    pub turn_index: usize,
    /// 该轮节点标题（派生自首条用户消息）。
    pub title: String,
    /// 该轮用户消息正文（撤回后回填输入框的真值）。
    pub user_message: String,
    /// 该轮写工具抓到 pre-image 的文件数（0 = 没有可回滚的代码改动）。
    pub files_tracked: usize,
    pub created_at_ms: Option<u64>,
}

/// 撤回范围：只回滚会话，或会话 + 已落盘代码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewindMode {
    ConversationOnly,
    ConversationAndFiles,
}

#[derive(Clone)]
pub struct RewindSessionCommand {
    pub session_id: Option<String>,
    /// 要撤掉的那一轮的节点 id（不是它的父节点）。
    pub node_id: String,
    pub mode: RewindMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindFileFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindSessionResponse {
    pub session_id: String,
    pub target_node_id: String,
    pub target_turn_id: Option<String>,
    pub turn_index: usize,
    /// 被撤掉那一轮的用户消息（宿主回填输入框；`None` = 历史缺正文）。
    pub user_message: Option<String>,
    /// 被撤销的轮次 id（旧→新，文件快照键）。
    pub cancelled_turn_ids: Vec<String>,
    pub files_restored: Vec<String>,
    pub files_deleted: Vec<String>,
    pub files_skipped: Vec<String>,
    pub file_failures: Vec<RewindFileFailure>,
    /// `true` ⇒ 文件还原**不完整**（有失败或超限跳过），宿主必须显式告知用户。
    pub file_restore_degraded: bool,
    /// 会话截断回执（复用既有 checkout 响应形状，便于宿主对拍）。
    pub checkout: HistoryCheckoutResponse,
}

impl HostControlPlane {
    /// 列出当前会话可撤回的用户轮次（**新→旧**，菜单自上而下）。
    pub fn list_rewind_points(&self, query: RewindPointsQuery) -> Vec<RewindPointView> {
        let session_id = self.normalize_history_session_id(query.session_id);
        let turns = {
            let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            sessions.rewind_turns(Some(session_id.as_str()))
        };
        let store = RewindFileStore::shared();
        let mut points: Vec<RewindPointView> = turns
            .into_iter()
            .enumerate()
            .map(|(index, node)| {
                let user_message = {
                    let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
                        tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                        e.into_inner()
                    });
                    sessions
                        .rewind_user_message_for_node(&session_id, &node.node_id)
                        .unwrap_or_default()
                };
                let files_tracked = node
                    .turn_id
                    .as_deref()
                    .map(|turn_id| store.count_pre_images(&session_id, turn_id))
                    .unwrap_or(0);
                RewindPointView {
                    node_id: node.node_id.clone(),
                    turn_id: node.turn_id.clone(),
                    turn_index: index + 1,
                    title: node.title.clone(),
                    user_message,
                    files_tracked,
                    created_at_ms: Some(node.created_at_ms),
                }
            })
            .collect();
        points.reverse();
        points
    }

    /// 撤回到指定用户轮次**之前**：会话截断（既有 checkout 真值路径）+ 可选文件还原。
    pub fn rewind_session(
        &self,
        command: RewindSessionCommand,
    ) -> Result<RewindSessionResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let plan = {
            let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            sessions.rewind_plan(Some(session_id.as_str()), &command.node_id)?
        };
        let keep_node_id = match plan.keep_node_id.clone() {
            Some(node_id) => node_id,
            None => {
                let mut sessions = self.sessions_rwlock.write().unwrap_or_else(|e| {
                    tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                    e.into_inner()
                });
                sessions.ensure_rewind_root_checkpoint(Some(session_id.as_str()))?
            }
        };

        // 文件还原在会话锁之外进行（纯 fs IO），且先于截断——见模块文档最后一条契约。
        let outcome = if command.mode == RewindMode::ConversationAndFiles {
            RewindFileStore::shared().restore_turns(&session_id, &plan.cancelled_turn_ids)
        } else {
            RewindRestoreOutcome::default()
        };

        let checkout = self.checkout_history_node(CheckoutHistoryNodeCommand {
            session_id: Some(session_id.clone()),
            node_id: keep_node_id,
            mode: HistoryCheckoutMode::TranscriptOnly,
            expected_cursor_version: None,
        })?;

        Ok(RewindSessionResponse {
            session_id,
            target_node_id: plan.target_node_id,
            target_turn_id: plan.target_turn_id,
            turn_index: plan.turn_index,
            user_message: plan.user_message,
            cancelled_turn_ids: plan.cancelled_turn_ids,
            files_restored: outcome.restored.clone(),
            files_deleted: outcome.deleted.clone(),
            files_skipped: outcome.skipped.clone(),
            file_failures: outcome
                .failures
                .iter()
                .map(|(path, reason)| RewindFileFailure {
                    path: path.clone(),
                    reason: reason.clone(),
                })
                .collect(),
            file_restore_degraded: outcome.degraded(),
            checkout,
        })
    }
}
