//! 会话 → 附件 asset id 索引：**整表重建**与**按会话增量重算**共用同一份口径。
//!
//! 为什么独立成文件：`session/store.rs` 已计入 `scripts/file-size-baseline.json` 的行数棘轮
//! （净值只许减）。这里承载的是纯函数（无状态、无 IO），`store.rs` 只留模块声明与调用点。
//!
//! 为什么要两套入口：稀疏（懒装载）实例的 `sessions` 只含**已装载**会话，
//! 整表重建会把未装载会话的索引条目抹掉，而附件清理以索引判孤儿 ⇒ 会真删别人的附件。
//! 稀疏模式因此只重算已装载会话的条目（见 `store_sparse::refresh_loaded_attachment_entries`）。

use super::attachment_asset_id;
use super::{SessionAttachmentIndex, SessionMap, SessionState};

/// 整表重建：`sessions` 决定索引内容（全量模式专用）。
pub(in crate::agent::session) fn rebuild_session_attachment_index(
    sessions: &SessionMap,
) -> SessionAttachmentIndex {
    let mut index = SessionAttachmentIndex::new();
    for session in sessions.values() {
        let entry = session_attachment_entry(session);
        if !entry.is_empty() {
            index.insert(session.conversation_id.clone(), entry);
        }
    }
    index
}

/// 单会话的附件 asset id 清单（去重保序）。
///
/// 两套入口共用：避免「整表重建」与「增量重算」各写一份逻辑而漂移。
pub(in crate::agent::session) fn session_attachment_entry(session: &SessionState) -> Vec<String> {
    let mut entry = Vec::new();
    for attachment in session
        .history
        .iter()
        .flat_map(|message| message.attachments.iter())
    {
        let asset_id = if attachment.asset_id.trim().is_empty() {
            attachment_asset_id(&attachment.relative_path)
        } else {
            attachment.asset_id.clone()
        };
        if !entry.iter().any(|existing| existing == &asset_id) {
            entry.push(asset_id);
        }
    }
    entry
}
