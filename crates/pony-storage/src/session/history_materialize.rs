//! `ensure_history_graph` 的历史物化构造（B186 file-size 收口：**纯搬迁**，行为逐字等价）。
//!
//! 原实现把整段 `SessionState` 字面量内联在 `store.rs::ensure_history_graph` 里，令该函数与
//! `store.rs` 双双越过规模基线；此处只把「构造 + 元数据刷新」搬出，输入/输出与原先一致。

use super::store::refresh_session_metadata;
use super::types::{
    HistoryCursor, LongTermMemoryRecord, SessionState, TurnHistoryMessage,
    DEFAULT_SESSION_SUMMARY, DEFAULT_SESSION_TITLE,
};

/// 构造某一轮（`turn_index`）的物化历史快照并刷新派生元数据。
///
/// 逐字段等价于搬迁前 `store.rs` 内联字面量：标题/摘要用默认值、`ai_title: None`、冻结的
/// trace 迁移状态、空图结构、零水位；`refresh_session_metadata(.., false)` 后返回。
pub(super) fn materialized_history_session(
    session: &SessionState,
    history: Vec<TurnHistoryMessage>,
    long_term_memory_entries: Vec<LongTermMemoryRecord>,
    turn_index: usize,
    user_index_total: usize,
) -> SessionState {
    let mut materialized = SessionState {
        conversation_id: session.conversation_id.clone(),
        title: DEFAULT_SESSION_TITLE.to_string(),
        ai_title: None,
        summary: DEFAULT_SESSION_SUMMARY.to_string(),
        history,
        provider_native_transcript: if turn_index + 1 == user_index_total {
            session.provider_native_transcript.clone()
        } else {
            Vec::new()
        },
        turn_trace_history: session
            .turn_trace_history
            .iter()
            .take((turn_index + 1).min(session.turn_trace_history.len()))
            .cloned()
            .collect(),
        trace_migration_state: session.trace_migration_state,
        turn_trace_refs: None,
        long_term_memory_entries,
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 0,
        last_referenced_file: None,
        updated_at_ms: session.updated_at_ms,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: HistoryCursor::default(),
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: session.workspace_id.clone(),
        title_override: None,
        archived: false,
    };
    refresh_session_metadata(&mut materialized, false);
    materialized
}
