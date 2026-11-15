//! 标题字段（`ai_title`）衍生补齐相关用例（B186 file-size 收口：从 `tests.rs` **纯搬迁**）。
//!
//! 断言与夹具逐字未改；仅位置变化，避免 `tests.rs` 越过规模基线。

use super::*;

#[test]
fn session_state_for_backend_keeps_legacy_blob_intact_under_write_separate() {
    // PA-088：WriteSeparate + LegacyBlob 存量会话不晋升、不剥离（保持可读写）。
    let session = SessionState {
        conversation_id: "s1".to_string(),
        title: "t".to_string(),
        ai_title: None,
        summary: "s".to_string(),
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: vec![TurnTraceRecord {
            turn_id: "turn-1".to_string(),
            updated_at: 100,
            ..TurnTraceRecord::default()
        }],
        trace_migration_state: TraceMigrationState::LegacyBlob,
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 100,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: HistoryCursor::default(),
        workspace_id: None,
        title_override: None,
        archived: false,
        event_watermark: 0,
        last_commit_watermark: 0,
    };

    let prepared = session_state_for_backend(&session, SeparateTraceTableMode::WriteSeparate);

    assert_eq!(
        prepared.trace_migration_state,
        TraceMigrationState::LegacyBlob,
        "legacy 会话不自动晋升"
    );
    assert_eq!(prepared.turn_trace_history.len(), 1, "blob trace 保留");
    assert!(prepared.turn_trace_refs.is_none());
}

#[test]
fn collect_trace_union_merges_top_level_and_node_traces() {
    // PA-088 P0-1 回归：写表用全量 Union（顶层 ∪ 节点 trace），
    // 保证 >24 轮会话的旧节点 refs 在重启 materialize 时可解析。
    let session = SessionState {
        conversation_id: "s1".to_string(),
        title: "t".to_string(),
        ai_title: None,
        summary: "s".to_string(),
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: vec![TurnTraceRecord {
            turn_id: "turn-1".to_string(),
            updated_at: 100,
            ..TurnTraceRecord::default()
        }],
        trace_migration_state: TraceMigrationState::TraceTableAuthoritative,
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 3,
        last_referenced_file: None,
        updated_at_ms: 300,
        history_nodes: vec![
            HistoryNode {
                node_id: "node-1".to_string(),
                session_id: "s1".to_string(),
                parent_node_id: None,
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                forked_from_node_id: None,
                kind: HistoryNodeKind::TurnCommitted,
                run_id: None,
                workspace_ref: WorkspaceRef::default(),
                summary: "s".to_string(),
                title: "t".to_string(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-1".to_string(),
                    updated_at: 100,
                    ..TurnTraceRecord::default()
                }],
                turn_id: Some("turn-1".to_string()),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
                created_at_ms: 100,
                event_seq_range: None,
            },
            HistoryNode {
                node_id: "node-2".to_string(),
                session_id: "s1".to_string(),
                parent_node_id: Some("node-1".to_string()),
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                forked_from_node_id: None,
                kind: HistoryNodeKind::TurnCommitted,
                run_id: None,
                workspace_ref: WorkspaceRef::default(),
                summary: "s".to_string(),
                title: "t".to_string(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                // 节点 2 引用了顶层已淘汰的 turn-2（>24 轮场景）
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-2".to_string(),
                    updated_at: 200,
                    ..TurnTraceRecord::default()
                }],
                turn_id: Some("turn-2".to_string()),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                turn_count: 2,
                last_referenced_file: None,
                created_at_ms: 200,
                event_seq_range: None,
            },
        ],
        history_branches: Vec::new(),
        history_cursor: HistoryCursor::default(),
        workspace_id: None,
        title_override: None,
        archived: false,
        event_watermark: 0,
        last_commit_watermark: 0,
    };

    let union = collect_trace_union(&session);
    let mut turn_ids: Vec<String> = union.iter().map(|trace| trace.turn_id.clone()).collect();
    turn_ids.sort();
    assert_eq!(turn_ids, vec!["turn-1".to_string(), "turn-2".to_string()]);
}
// ── PA-093 checkpoint 引用化（阶段 3）测试 ──

