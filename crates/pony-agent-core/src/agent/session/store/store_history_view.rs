//! 会话快照与 trace→历史消息的映射辅助。
//!
//! 自 `session/store.rs` 拆分（file-size 收口，行为逐字等价）：
//! 函数体、分支顺序与语义一字未改；`enrich_history_from_traces` 的 `pub(super)`
//! 改写为等价的 `pub(in crate::agent::session)`，保持 `session/mod.rs` 的 re-export 契约。

use super::*;

/// 从 trace_timeline 提取最终 assistant 消息的 reasoning_content。
/// 取最后一条 `call_model` 类型 timeline entry 的 reasoning_content，
/// 与前端 traceReasoningContent 逻辑一致。
fn extract_reasoning_content(trace: &TurnTraceRecord) -> Option<String> {
    trace
        .trace_timeline
        .iter()
        .rev()
        .find(|e| e.kind == "call_model")
        .and_then(|e| e.reasoning_content.clone())
}

/// TurnTraceRecord.phase → MessageStatus 映射
fn derive_status_from_trace(trace: &TurnTraceRecord) -> MessageStatus {
    match trace.phase.as_str() {
        "completed" => MessageStatus::Done,
        "failed" | "cancelled" => MessageStatus::Error,
        _ => MessageStatus::Error,
    }
}

/// 用 turn_trace_history 中的元数据补全 TurnHistoryMessage 列表。
/// 使用末端对齐策略处理 history 与 turn_trace_history 截断步长不一致的问题。
/// 幂等：已携带元数据的条目不覆写。
pub(in crate::agent::session) fn enrich_history_from_traces(
    history: &mut [TurnHistoryMessage],
    turn_trace_history: &[TurnTraceRecord],
) {
    let msg_count = history.len();
    let trace_count = turn_trace_history.len();
    if msg_count == 0 || trace_count == 0 {
        return;
    }
    let offset = trace_count.saturating_sub(msg_count / 2);

    for (i, msg) in history.iter_mut().enumerate() {
        let turn_idx = i / 2;
        let trace_idx = turn_idx + offset;

        if msg.turn_id.is_some() {
            continue; // 幂等：跳过已携带元数据的条目
        }

        if let Some(trace) = turn_trace_history.get(trace_idx) {
            if msg.role == "assistant" {
                msg.turn_id = Some(trace.turn_id.clone());
                msg.model_name = trace.provider_model.clone();
                msg.token_count = trace.output_tokens;
                msg.reasoning_content = extract_reasoning_content(trace);
                msg.status = Some(derive_status_from_trace(trace));
            } else if msg.role == "user" {
                msg.turn_id = Some(trace.turn_id.clone());
            }
        }
    }
}

/// 会话尚不存在时的只读默认快照（与 ensure_session 默认结构一致，但不创建、不落盘）。
pub(super) fn default_snapshot_for_session(
    session_key: &str,
    node_id: Option<&str>,
    workspace_cwd: Option<String>,
) -> SessionSnapshot {
    let session = SessionState {
        conversation_id: session_key.to_string(),
        title: default_session_title(),
        summary: DEFAULT_SESSION_SUMMARY.to_string(),
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: TraceMigrationState::LegacyBlob,
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 0,
        last_referenced_file: None,
        updated_at_ms: 0,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: HistoryCursor {
            session_id: session_key.to_string(),
            ..HistoryCursor::default()
        },
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: None,
        archived: false,
    };
    snapshot_from_state(&session, Vec::new(), node_id, workspace_cwd)
}

/// env note 的 env_info（R1）：`workspace_cwd` 可用 → 基于该工作区 root 收集
/// （cwd=会话实际工作目录，git 判定也基于 root）；None（解析全失败且进程 cwd 不可得）
/// → 原 `collect_env_info()` 兜底（进程 cwd，不 panic）。
pub(super) fn snapshot_env_info(workspace_cwd: Option<&str>) -> EnvironmentInfo {
    match workspace_cwd {
        Some(cwd) => collect_env_info_for_workspace(Path::new(cwd)),
        None => collect_env_info(),
    }
}
