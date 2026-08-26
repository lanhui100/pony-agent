use super::*;
use crate::agent::turn_event::{TurnEndReason, TurnEvent};
use std::time::{SystemTime, UNIX_EPOCH};

struct StaticMemoryWriteHookExecutor {
    results: Vec<crate::agent::hooks::HookExecutionResult>,
}

impl MemoryWriteHookExecutor for StaticMemoryWriteHookExecutor {
    fn execute(
        &self,
        _envelope: &MemoryWriteHookEnvelope,
    ) -> Result<Vec<crate::agent::hooks::HookExecutionResult>, String> {
        Ok(self.results.clone())
    }
}

struct StaticHistoryStateHookExecutor {
    start_results: Vec<crate::agent::hooks::HookExecutionResult>,
    resolved_results: Vec<crate::agent::hooks::HookExecutionResult>,
}

impl HistoryStateHookExecutor for StaticHistoryStateHookExecutor {
    fn execute(
        &self,
        envelope: &HistoryStateHookEnvelope,
    ) -> Result<Vec<crate::agent::hooks::HookExecutionResult>, String> {
        Ok(match envelope.hook_point {
            HistoryStateHookPoint::HistoryCheckoutStart
            | HistoryStateHookPoint::BranchRestoreStart
            | HistoryStateHookPoint::BranchForkStart
            | HistoryStateHookPoint::BranchSwitchStart => self.start_results.clone(),
            HistoryStateHookPoint::HistoryCheckoutResolved
            | HistoryStateHookPoint::BranchRestoreResolved
            | HistoryStateHookPoint::BranchForkResolved
            | HistoryStateHookPoint::BranchSwitchResolved => self.resolved_results.clone(),
        })
    }
}

#[test]
fn memory_backend_keeps_turns_in_process() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("test"),
        "查看 tauri.conf.json",
        "已读取",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("test"), &[]);
    assert_eq!(snapshot.title, "查看 tauri.conf.json");
    assert_eq!(snapshot.turn_count, 1);
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(
        snapshot.last_referenced_file.as_deref(),
        Some("tauri.conf.json")
    );
}

#[test]
fn serde_roundtrip_enriched_metadata() {
    let msg = TurnHistoryMessage {
        role: "assistant".to_string(),
        content: "test".to_string(),
        attachments: Vec::new(),
        turn_id: Some("turn-1".to_string()),
        status: Some(MessageStatus::Done),
        model_name: Some("gpt-5".to_string()),
        token_count: Some(42),
        reasoning_content: Some("thinking...".to_string()),
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let deserialized: TurnHistoryMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deserialized.turn_id, Some("turn-1".to_string()));
    assert_eq!(deserialized.status, Some(MessageStatus::Done));
    assert_eq!(deserialized.model_name, Some("gpt-5".to_string()));
    assert_eq!(deserialized.token_count, Some(42));
    assert_eq!(
        deserialized.reasoning_content,
        Some("thinking...".to_string())
    );
    assert_eq!(deserialized.stable_id(), "turn-1-assistant");
}

#[test]
fn serde_roundtrip_old_blob_compatible() {
    let old_json = r#"{"role":"user","content":"hello","attachments":[]}"#;
    let msg: TurnHistoryMessage = serde_json::from_str(old_json).expect("old blob deserialize");
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content, "hello");
    assert!(msg.turn_id.is_none());
    assert!(msg.status.is_none());
    assert!(msg.model_name.is_none());
    assert!(msg.token_count.is_none());
    assert!(msg.reasoning_content.is_none());
    assert_eq!(msg.stable_id(), "unknown-user");

    let re_json = serde_json::to_string(&msg).expect("serialize");
    assert!(!re_json.contains("turnId"));
    assert!(!re_json.contains("status"));
    assert!(!re_json.contains("modelName"));
    assert!(!re_json.contains("tokenCount"));
    assert!(!re_json.contains("reasoningContent"));
}

#[test]
fn snapshot_enriched_history_metadata() {
    let mut store = SessionStore::memory_only();
    let session_id = "enriched-test";
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "你好".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "收到。".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                phase: "completed".to_string(),
                title: "test turn".to_string(),
                provider_model: Some("gpt-5".to_string()),
                output_tokens: Some(42),
                ..Default::default()
            }],
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );
    let snapshot = store.snapshot(Some(session_id), &[]);
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.history[0].role, "user");
    assert_eq!(snapshot.history[0].turn_id.as_deref(), Some("turn-1"));
    assert_eq!(snapshot.history[1].role, "assistant");
    assert_eq!(snapshot.history[1].turn_id.as_deref(), Some("turn-1"));
    assert_eq!(snapshot.history[1].model_name.as_deref(), Some("gpt-5"));
    assert_eq!(snapshot.history[1].token_count, Some(42));
    assert_eq!(snapshot.history[1].status, Some(MessageStatus::Done));
}

#[test]
fn snapshot_mixed_metadata_preserves_existing() {
    // 空 trace：幂等性——已有元数据不被清除
    let msg = TurnHistoryMessage {
        role: "assistant".to_string(),
        content: "已有元数据".to_string(),
        attachments: Vec::new(),
        turn_id: Some("existing-turn".to_string()),
        status: Some(MessageStatus::Done),
        model_name: Some("existing-model".to_string()),
        token_count: Some(99),
        reasoning_content: Some("existing".to_string()),
    };
    let empty_trace: Vec<TurnTraceRecord> = Vec::new();
    let mut history = vec![msg];
    enrich_history_from_traces(&mut history, &empty_trace);
    assert_eq!(history[0].turn_id, Some("existing-turn".to_string()));
    assert_eq!(history[0].model_name, Some("existing-model".to_string()));
    assert_eq!(history[0].token_count, Some(99));

    // 有 trace：已有元数据不被覆写
    let msg2 = TurnHistoryMessage {
        role: "assistant".to_string(),
        content: "不变".to_string(),
        attachments: Vec::new(),
        turn_id: Some("preserved-turn".to_string()),
        ..Default::default()
    };
    let traces = vec![TurnTraceRecord {
        turn_id: "trace-turn".to_string(),
        phase: "completed".to_string(),
        title: "trace".to_string(),
        provider_model: Some("trace-model".to_string()),
        output_tokens: Some(1),
        ..Default::default()
    }];
    let mut history2 = vec![
        TurnHistoryMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            attachments: Vec::new(),
            ..Default::default()
        },
        msg2,
    ];
    enrich_history_from_traces(&mut history2, &traces);
    assert_eq!(history2[0].turn_id, Some("trace-turn".to_string()));
    assert_eq!(history2[1].turn_id, Some("preserved-turn".to_string()));
    assert!(history2[1].model_name.is_none());
}

/// PA-095 #6：最小单节点会话种子（冲突检测/水位镜像测试共用）。
fn pa095_seed_single_node_session(store: &mut SessionStore, session_id: &str) {
    let node_id = "node-1".to_string();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: vec![HistoryNode {
                node_id: node_id.clone(),
                session_id: session_id.to_string(),
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                ..Default::default()
            }],
            history_branches: vec![HistoryBranch {
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                head_node_id: Some(node_id.clone()),
                ..Default::default()
            }],
            history_cursor: HistoryCursor {
                session_id: session_id.to_string(),
                visible_node_id: Some(node_id.clone()),
                active_branch_id: Some(DEFAULT_HISTORY_BRANCH_ID.to_string()),
                branch_head_node_id: Some(node_id.clone()),
                workspace_node_id: Some(node_id.clone()),
                mode: HistoryCursorMode::Live,
                ..Default::default()
            },
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );
}

/// PA-095 #6：四类 history-control command 冲突检测改水位比较——表驱动。
/// 精确当前水位放行、stale 水位拒绝（错误信息含 watermark 语义）、None 放行。
#[test]
fn pa095_history_command_conflict_detection_is_watermark_based() {
    type HistoryCommand =
        fn(&mut SessionStore, &str, Option<u64>) -> Result<SessionSnapshot, String>;
    let commands: Vec<(&str, HistoryCommand)> = vec![
        ("checkout", |store, sid, expected| {
            store.checkout_history_node(
                Some(sid),
                "node-1",
                HistoryCheckoutMode::TranscriptOnly,
                expected,
            )
        }),
        ("restore", |store, sid, expected| {
            store.restore_branch_head(Some(sid), None, expected)
        }),
        ("fork", |store, sid, expected| {
            store.fork_from_history_node(Some(sid), "node-1", expected)
        }),
        ("switch", |store, sid, expected| {
            store.switch_history_branch(Some(sid), DEFAULT_HISTORY_BRANCH_ID, expected)
        }),
    ];
    for (name, command) in commands {
        let mut store = SessionStore::memory_only();
        pa095_seed_single_node_session(&mut store, "conflict-s");
        // None = 不做乐观并发检测，永远放行。
        assert!(
            command(&mut store, "conflict-s", None).is_ok(),
            "{name}: None passes"
        );
        // 精确当前水位（0）→ 放行。
        assert!(
            command(&mut store, "conflict-s", Some(0)).is_ok(),
            "{name}: exact watermark passes"
        );
        // stale 水位（单调域外）→ 冲突拒绝，语义指向 watermark。
        let error = command(&mut store, "conflict-s", Some(7))
            .expect_err(&format!("{name}: stale watermark must conflict"));
        assert!(
            error.contains("watermark"),
            "{name}: conflict message must reference watermark: {error}"
        );
        // 命令成功后 cursor_version 镜像当前水位。
        let snapshot = command(&mut store, "conflict-s", Some(0)).expect("{name}: pass again");
        assert_eq!(
            snapshot.history_cursor.cursor_version, snapshot.history_cursor.event_watermark,
            "{name}: cursor_version mirrors watermark"
        );
    }
}

/// PA-095 #6：checkout 后版本严格递增（无 ABA）——水位随事件提交只增不减，
/// cursor_version 恒等镜像；旧期望值在新水位下被拒绝。
#[test]
fn pa095_checkout_version_strictly_increases_with_watermark_no_aba() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let mut store = SessionStore::with_backend(Box::new(
        crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
            std::env::temp_dir().join(format!("pony-pa095-watermark-{stamp}.db")),
            SeparateTraceTableMode::Off,
        ),
    ));
    pa095_seed_single_node_session(&mut store, "aba-s");
    // 隔离并行噪声：本会话事件通道绑定为无操作（不落外部存储）。
    let _binding = crate::agent::turn_flow::bind_event_persist_session(
        "aba-s",
        std::sync::Arc::new(|_, _, _, _| {}),
    );

    // 第一批事件提交 → 水位推进。
    store.persist_events(
        "aba-s",
        "t1",
        DEFAULT_HISTORY_BRANCH_ID,
        vec![
            crate::agent::turn_event::TurnEvent::UserMessage {
                turn_id: "t1".into(),
                text: "第一轮".into(),
                attachments: Vec::new(),
            },
            crate::agent::turn_event::TurnEvent::AssistantMessage {
                turn_id: "t1".into(),
                step: 0,
                text: "答复一".into(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ],
    );
    store.finalize_event_watermark("aba-s", "t1");
    let watermark_1 = store.sessions.get("aba-s").unwrap().event_watermark;
    assert!(watermark_1 >= 2, "first batch advances watermark");

    // checkout（携带当前水位）→ 成功，版本镜像水位。
    let snapshot = store
        .checkout_history_node(
            Some("aba-s"),
            "node-1",
            HistoryCheckoutMode::TranscriptOnly,
            Some(watermark_1),
        )
        .expect("checkout with current watermark");
    assert_eq!(snapshot.history_cursor.event_watermark, watermark_1);
    assert_eq!(snapshot.history_cursor.cursor_version, watermark_1);

    // 第二批事件提交 → 水位严格递增。
    store.persist_events(
        "aba-s",
        "t2",
        DEFAULT_HISTORY_BRANCH_ID,
        vec![crate::agent::turn_event::TurnEvent::UserMessage {
            turn_id: "t2".into(),
            text: "第二轮".into(),
            attachments: Vec::new(),
        }],
    );
    store.finalize_event_watermark("aba-s", "t2");
    let watermark_2 = store.sessions.get("aba-s").unwrap().event_watermark;
    assert!(
        watermark_2 > watermark_1,
        "watermark strictly increases: {watermark_2} > {watermark_1}"
    );

    // 无 ABA：旧水位期望被新状态拒绝，当前水位放行。
    let stale = store
        .checkout_history_node(
            Some("aba-s"),
            "node-1",
            HistoryCheckoutMode::TranscriptOnly,
            Some(watermark_1),
        )
        .expect_err("stale watermark must be rejected (no ABA)");
    assert!(stale.contains("watermark"), "{stale}");
    store
        .checkout_history_node(
            Some("aba-s"),
            "node-1",
            HistoryCheckoutMode::TranscriptOnly,
            Some(watermark_2),
        )
        .expect("current watermark passes");
    let _ =
        std::fs::remove_file(std::env::temp_dir().join(format!("pony-pa095-watermark-{stamp}.db")));
}

#[test]
fn checkout_history_node_metadata() {
    let mut store = SessionStore::memory_only();
    let session_id = "checkout-meta";
    let node_id = "node-checkout-1".to_string();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "第一轮".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "收到。".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                phase: "completed".to_string(),
                title: "first turn".to_string(),
                provider_model: Some("gpt-5".to_string()),
                output_tokens: Some(42),
                ..Default::default()
            }],
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: vec![HistoryNode {
                node_id: node_id.clone(),
                session_id: session_id.to_string(),
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![
                    TurnHistoryMessage {
                        role: "user".to_string(),
                        content: "第一轮".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        content: "收到。".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                ],
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-1".to_string(),
                    phase: "completed".to_string(),
                    title: "first turn".to_string(),
                    provider_model: Some("gpt-5".to_string()),
                    output_tokens: Some(42),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            history_branches: vec![HistoryBranch {
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                head_node_id: Some(node_id.clone()),
                ..Default::default()
            }],
            history_cursor: HistoryCursor {
                session_id: session_id.to_string(),
                visible_node_id: Some(node_id.clone()),
                active_branch_id: Some(DEFAULT_HISTORY_BRANCH_ID.to_string()),
                branch_head_node_id: Some(node_id.clone()),
                workspace_node_id: Some(node_id.clone()),
                mode: HistoryCursorMode::Live,
                ..Default::default()
            },
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );
    let snapshot = store
        .checkout_history_node(
            Some(session_id),
            &node_id,
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("checkout should succeed");

    assert!(snapshot.history.len() >= 2);
    assert_eq!(
        snapshot.history[0].turn_id.as_deref(),
        Some("turn-1"),
        "user message should have turn_id after enrichment"
    );
    assert_eq!(
        snapshot.history[1].turn_id.as_deref(),
        Some("turn-1"),
        "assistant message should have turn_id after enrichment"
    );
    assert_eq!(snapshot.history[1].model_name.as_deref(), Some("gpt-5"));
    assert_eq!(snapshot.history[1].token_count, Some(42));
    assert_eq!(snapshot.history[1].status, Some(MessageStatus::Done));
}

#[test]
fn ensure_history_graph_inserts_initial_root_node_for_legacy_sessions() {
    let mut store = SessionStore::memory_only();
    let session_id = "legacy-root-session";
    let node_id = "node-checkout-1".to_string();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "第一轮".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "收到。".to_string(),
                    attachments: Vec::new(),
                    ..Default::default()
                },
            ],
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                phase: "completed".to_string(),
                title: "first turn".to_string(),
                ..Default::default()
            }],
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: vec![HistoryNode {
                node_id: node_id.clone(),
                session_id: session_id.to_string(),
                parent_node_id: None,
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![
                    TurnHistoryMessage {
                        role: "user".to_string(),
                        content: "第一轮".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        content: "收到。".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                ],
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-1".to_string(),
                    phase: "completed".to_string(),
                    title: "first turn".to_string(),
                    ..Default::default()
                }],
                turn_count: 1,
                ..Default::default()
            }],
            history_branches: vec![HistoryBranch {
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                head_node_id: Some(node_id.clone()),
                base_node_id: Some(node_id.clone()),
                ..Default::default()
            }],
            history_cursor: HistoryCursor {
                session_id: session_id.to_string(),
                visible_node_id: Some(node_id.clone()),
                active_branch_id: Some(DEFAULT_HISTORY_BRANCH_ID.to_string()),
                branch_head_node_id: Some(node_id.clone()),
                workspace_node_id: Some(node_id.clone()),
                mode: HistoryCursorMode::Live,
                ..Default::default()
            },
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let (history_nodes, history_branches, _) = store.load_history_graph(Some(session_id));
    let root_node_id = format!("{}-legacy-root", session_id);

    assert_eq!(history_nodes.len(), 2);
    assert_eq!(history_nodes[0].node_id, root_node_id);
    assert_eq!(history_nodes[0].turn_count, 0);
    assert_eq!(
        history_nodes[1].parent_node_id.as_deref(),
        Some(history_nodes[0].node_id.as_str())
    );
    assert_eq!(
        history_branches[0].base_node_id.as_deref(),
        Some(history_nodes[0].node_id.as_str())
    );
}

#[test]
fn file_backend_roundtrip_restores_sessions() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.append_turn(
        Some("persisted"),
        "打开 Cargo.toml",
        "已读取",
        None,
        Vec::new(),
    );

    let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let mut reloaded = reloaded;
    let snapshot = reloaded.snapshot(Some("persisted"), &[]);
    assert_eq!(snapshot.title, "打开 Cargo.toml");

    assert_eq!(snapshot.turn_count, 1);
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.last_referenced_file.as_deref(), Some("Cargo.toml"));

    let _ = fs::remove_file(path);
}

#[test]
fn long_term_memory_entries_roundtrip_through_store() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    let snapshot = store.replace_long_term_memory(
        Some("memory-session"),
        vec![LongTermMemoryRecord {
            kind: "user_preference".to_string(),
            content: "Reply in Chinese and keep answers concise.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms: 42,
        }],
    );

    assert_eq!(snapshot.long_term_memory_entries.len(), 1);
    assert_eq!(snapshot.long_term_memory_entries[0].kind, "user_preference");

    let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let mut reloaded = reloaded;
    let snapshot = reloaded.snapshot(Some("memory-session"), &[]);
    assert_eq!(snapshot.long_term_memory_entries.len(), 1);
    assert_eq!(
        snapshot.long_term_memory_entries[0].content,
        "Reply in Chinese and keep answers concise."
    );

    let _ = fs::remove_file(path);
}

/// PA-095 #2：append_turn characterization 快照——改造（事件源物化）前录制
/// 现有可观测行为作回归防护：history 追加形态（user 带 attachments、assistant
/// 文本）、turn_count、transcript 扩展、SQLite blob 往返（重载一致）。
/// 改造后本测试必须保持全绿（豁免差异走对拍豁免清单，不在此处）。
#[test]
fn append_turn_characterization_snapshot() {
    let dir = std::env::temp_dir().join(format!(
        "pony-append-char-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("mkdir");
    let db_path = dir.join("sessions.db");

    let attachment = crate::agent::session::AttachmentReference {
        id: "att-1".to_string(),
        asset_id: String::new(),
        name: Some("note.txt".to_string()),
        mime_type: "text/plain".to_string(),
        relative_path: "imports/note.txt".to_string(),
        size_bytes: 12,
        created_at_ms: 1_000,
    };
    let mut store = SessionStore::with_backend(Box::new(
        crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ),
    ));
    let snapshot = store.append_turn(
        Some("char-snap"),
        "问题Q",
        "回答A",
        Some(vec![serde_json::json!({"hop": 1})]),
        vec![attachment.clone()],
    );

    // history 追加形态：user（带 attachments）+ assistant。
    assert_eq!(snapshot.history.len(), 2, "one turn = two entries");
    assert_eq!(snapshot.history[0].role, "user");
    assert_eq!(snapshot.history[0].content, "问题Q");
    assert_eq!(snapshot.history[0].attachments.len(), 1);
    assert_eq!(snapshot.history[0].attachments[0].id, "att-1");
    assert_eq!(snapshot.history[1].role, "assistant");
    assert_eq!(snapshot.history[1].content, "回答A");
    assert_eq!(snapshot.turn_count, 1);
    assert!(snapshot
        .provider_native_transcript
        .iter()
        .any(|value| value == &serde_json::json!({"hop": 1})));

    // 第二轮：history 累积、turn_count 递增。
    let snapshot = store.append_turn(Some("char-snap"), "问题Q2", "回答A2", None, Vec::new());
    assert_eq!(snapshot.history.len(), 4);
    assert_eq!(snapshot.history[2].content, "问题Q2");
    assert_eq!(snapshot.history[3].content, "回答A2");
    assert_eq!(snapshot.turn_count, 2);

    // SQLite blob 往返：重载后 history 一致（改造不得破坏持久化语义）。
    drop(store);
    let mut reloaded = SessionStore::with_backend(Box::new(
        crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
            db_path,
            SeparateTraceTableMode::WriteSeparate,
        ),
    ));
    let reloaded_snapshot = reloaded.snapshot(Some("char-snap"), &[]);
    let replayed: Vec<(String, String)> = reloaded_snapshot
        .history
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect();
    assert_eq!(
        replayed,
        vec![
            ("user".to_string(), "问题Q".to_string()),
            ("assistant".to_string(), "回答A".to_string()),
            ("user".to_string(), "问题Q2".to_string()),
            ("assistant".to_string(), "回答A2".to_string()),
        ],
        "blob roundtrip preserves history"
    );
    assert_eq!(reloaded_snapshot.history[0].attachments.len(), 1);

    let _ = fs::remove_dir_all(&dir);
}

/// PA-095 #2：物化选取逻辑——最近 turn 的 user/assistant 提取；
/// failed turn（仅 user/message）assistant 缺席由调用方补占位。
#[test]
fn materialize_last_turn_messages_picks_latest_turn() {
    use crate::agent::turn_event::{TurnEndReason, TurnEvent};
    let events = |events: Vec<TurnEvent>| {
        events
            .into_iter()
            .enumerate()
            .map(|(seq, event)| (seq as u64, "main".to_string(), event))
            .collect::<Vec<_>>()
    };
    // 两个 turn：物化必须取最近一个。
    let two_turns = events(vec![
        TurnEvent::TurnStart {
            turn_id: "t1".into(),
        },
        TurnEvent::UserMessage {
            turn_id: "t1".into(),
            text: "第一问".into(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantMessage {
            turn_id: "t1".into(),
            step: 0,
            text: "第一答".into(),
            reasoning_content: None,
            usage: None,
            chunk_missing: None,
        },
        TurnEvent::TurnEnd {
            turn_id: "t1".into(),
            reason: TurnEndReason::Completed,
            turn_duration_ms: Some(10),
        },
        TurnEvent::TurnStart {
            turn_id: "t2".into(),
        },
        TurnEvent::UserMessage {
            turn_id: "t2".into(),
            text: "第二问".into(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantMessage {
            turn_id: "t2".into(),
            step: 0,
            text: "第二答".into(),
            reasoning_content: Some("推理R".into()),
            usage: None,
            chunk_missing: None,
        },
    ]);
    let (user, assistant) = materialize_last_turn_messages(&two_turns).expect("materialized");
    assert_eq!(user.as_deref(), Some("第二问"));
    assert_eq!(
        assistant
            .as_ref()
            .map(|(text, reasoning)| (text.as_str(), reasoning.as_deref())),
        Some(("第二答", Some("推理R")))
    );

    // failed turn：仅 user/message——assistant 为 None（调用方补占位）。
    let failed_turn = events(vec![
        TurnEvent::TurnStart {
            turn_id: "t3".into(),
        },
        TurnEvent::UserMessage {
            turn_id: "t3".into(),
            text: "失败问".into(),
            attachments: Vec::new(),
        },
        TurnEvent::TurnEnd {
            turn_id: "t3".into(),
            reason: TurnEndReason::Error,
            turn_duration_ms: None,
        },
    ]);
    let (user, assistant) = materialize_last_turn_messages(&failed_turn).expect("materialized");
    assert_eq!(user.as_deref(), Some("失败问"));
    assert!(assistant.is_none(), "failed turn has no assistant message");

    // 无 turn 归属事件 → None（整体回退）。
    assert!(materialize_last_turn_messages(&[]).is_none());
}

#[test]
fn append_turn_persists_memory_write_evidence_for_explicit_note() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-evidence"),
        "请记住这个项目当前优先推进 PA-018。",
        "我会记住这条信息。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-evidence"), &[]);
    assert!(!snapshot.memory_write_evidence.is_empty());
    let latest_node_id = snapshot
        .history_cursor
        .visible_node_id
        .clone()
        .expect("latest history node id");
    assert!(snapshot.memory_write_evidence.iter().any(|evidence| {
        evidence.effect_kind == "memory_write.long_term_memory"
            && evidence.boundary == "session.update_long_term_memory_from_user_message"
            && evidence.replay_required_if_missing
            && evidence
                .persistence_ref
                .starts_with("long_term_memory_entries/")
            && evidence.source_history_node_id.as_deref() == Some(latest_node_id.as_str())
    }));
}

#[test]
fn memory_write_evidence_roundtrip_through_store() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.append_turn(
        Some("memory-evidence-roundtrip"),
        "请记住这个项目当前优先推进 PA-018。",
        "我会记住这条信息。",
        None,
        Vec::new(),
    );

    let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let mut reloaded = reloaded;
    let snapshot = reloaded.snapshot(Some("memory-evidence-roundtrip"), &[]);
    assert!(!snapshot.memory_write_evidence.is_empty());
    assert!(snapshot.memory_write_evidence.iter().all(|evidence| {
        evidence.effect_kind == "memory_write.long_term_memory"
            && evidence.replay_required_if_missing
            && evidence.source_history_node_id.is_some()
    }));

    let _ = fs::remove_file(path);
}

#[test]
fn memory_write_guard_deny_blocks_persistence_and_memory_mutation() {
    let mut store = SessionStore::memory_only();
    store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
        results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "memory.guard".to_string(),
            hook_class: crate::agent::hooks::HookClass::Guard,
            hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildStart,
            hook_order: 1,
            result_kind: crate::agent::hooks::HookResultKind::Deny,
            structured_result: HookStructuredResult::Deny(crate::agent::hooks::HookDenyDecision {
                reason_code: "memory_write_blocked".to_string(),
                message: "memory write denied by guard".to_string(),
            }),
            blocked: true,
            elapsed_ms: 1,
            input_summary: Some("deny memory write".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "memory write denied".to_string(),
        }],
    }));

    store.append_turn(
        Some("memory-guard-deny"),
        "请记住这个项目当前优先推进 PA-039。",
        "收到。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-guard-deny"), &[]);
    assert!(snapshot.long_term_memory_entries.is_empty());
    assert!(snapshot.memory_write_evidence.is_empty());
    assert_eq!(snapshot.memory_write_hook_trace_records.len(), 1);
    assert_eq!(
        snapshot.memory_write_hook_trace_records[0].hook_name,
        "memory.guard"
    );
    assert!(snapshot.memory_write_hook_trace_records[0].blocked);
    assert_eq!(
        snapshot.memory_write_hook_trace_records[0].summary,
        "memory write denied"
    );
}

#[test]
fn memory_write_transform_patch_can_rewrite_persisted_memory_intent() {
    let mut store = SessionStore::memory_only();
    store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
        results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "memory.transform".to_string(),
            hook_class: crate::agent::hooks::HookClass::Transform,
            hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
            hook_order: 1,
            result_kind: crate::agent::hooks::HookResultKind::Patch,
            structured_result: HookStructuredResult::Patch {
                operations: vec![crate::agent::hooks::HookPatchOperation {
                    target: HookPatchTarget::MemoryWriteIntent,
                    path: "writes[1].content".to_string(),
                    operation: HookPatchOperationKind::Set,
                    value_summary: Some("Current active task is PA-040.".to_string()),
                    value_text: Some("Current active task is PA-040.".to_string()),
                }],
            },
            blocked: false,
            elapsed_ms: 1,
            input_summary: Some("rewrite memory content".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "memory write transformed".to_string(),
        }],
    }));

    store.append_turn(
        Some("memory-transform"),
        "请记住这个项目当前优先推进 PA-039。",
        "收到。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-transform"), &[]);
    let active_task = snapshot
        .long_term_memory_entries
        .iter()
        .find(|entry| entry.kind == "project_focus.active_task")
        .expect("transformed active task entry");
    assert_eq!(active_task.content, "Current active task is PA-040.");
    assert!(snapshot.memory_write_evidence.iter().any(|evidence| {
        evidence.target_summary.contains("PA-040")
            && evidence.persistence_ref == "long_term_memory_entries/project_focus.active_task"
    }));
    assert_eq!(snapshot.memory_write_hook_trace_records.len(), 1);
    assert_eq!(
        snapshot.memory_write_hook_trace_records[0].hook_name,
        "memory.transform"
    );
    assert!(!snapshot.memory_write_hook_trace_records[0].blocked);
}

#[test]
fn memory_write_hook_trace_records_roundtrip_through_store() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
        results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "memory.transform".to_string(),
            hook_class: crate::agent::hooks::HookClass::Transform,
            hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
            hook_order: 1,
            result_kind: crate::agent::hooks::HookResultKind::Patch,
            structured_result: HookStructuredResult::Patch {
                operations: vec![crate::agent::hooks::HookPatchOperation {
                    target: HookPatchTarget::MemoryWriteIntent,
                    path: "writes[1].content".to_string(),
                    operation: HookPatchOperationKind::Set,
                    value_summary: Some("Current active task is PA-040.".to_string()),
                    value_text: Some("Current active task is PA-040.".to_string()),
                }],
            },
            blocked: false,
            elapsed_ms: 1,
            input_summary: Some("rewrite memory content".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "memory write transformed".to_string(),
        }],
    }));
    store.append_turn(
        Some("memory-hook-trace-roundtrip"),
        "请记住这个项目当前优先推进 PA-039。",
        "收到。",
        None,
        Vec::new(),
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("memory-hook-trace-roundtrip"), &[]);
    assert_eq!(snapshot.memory_write_hook_trace_records.len(), 1);
    assert_eq!(
        snapshot.memory_write_hook_trace_records[0].hook_name,
        "memory.transform"
    );
    assert_eq!(
        snapshot.memory_write_hook_trace_records[0].summary,
        "memory write transformed"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn history_checkout_restores_memory_write_hook_trace_records_from_selected_node() {
    let mut store = SessionStore::memory_only();
    store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
        results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "memory.transform".to_string(),
            hook_class: crate::agent::hooks::HookClass::Transform,
            hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
            hook_order: 1,
            result_kind: crate::agent::hooks::HookResultKind::Patch,
            structured_result: HookStructuredResult::Patch {
                operations: vec![crate::agent::hooks::HookPatchOperation {
                    target: HookPatchTarget::MemoryWriteIntent,
                    path: "writes[1].content".to_string(),
                    operation: HookPatchOperationKind::Set,
                    value_summary: Some("Current active task is PA-040.".to_string()),
                    value_text: Some("Current active task is PA-040.".to_string()),
                }],
            },
            blocked: false,
            elapsed_ms: 1,
            input_summary: Some("rewrite memory content".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "memory write transformed".to_string(),
        }],
    }));
    store.append_turn(
        Some("memory-hook-history"),
        "请记住这个项目当前优先推进 PA-039。",
        "收到。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-hook-history"),
        "请记住这个项目当前风险是 trace reload 不稳定。",
        "收到。",
        None,
        Vec::new(),
    );

    let (nodes, _, _) = store.load_history_graph(Some("memory-hook-history"));
    assert!(nodes.len() >= 3);
    assert_eq!(
        nodes[nodes.len() - 2].memory_write_hook_trace_records.len(),
        1
    );
    assert_eq!(
        nodes[nodes.len() - 1].memory_write_hook_trace_records.len(),
        2
    );

    let historical = store
        .checkout_history_node(
            Some("memory-hook-history"),
            nodes[1].node_id.as_str(),
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("checkout should succeed");
    assert_eq!(historical.memory_write_hook_trace_records.len(), 1);
    assert_eq!(
        historical.memory_write_hook_trace_records[0].hook_name,
        "memory.transform"
    );

    let live = store.snapshot(Some("memory-hook-history"), &[]);
    assert_eq!(live.memory_write_hook_trace_records.len(), 1);
}

#[test]
fn checkout_history_node_persists_history_state_hook_evidence() {
    let mut store = SessionStore::memory_only();
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.guard.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout start observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 2,
            input_summary: Some("checkout start".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout start observed".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.resolved.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout resolved".to_string(),
            },
            blocked: false,
            elapsed_ms: 3,
            input_summary: Some("checkout resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout resolved".to_string(),
        }],
    }));
    store.append_turn(
        Some("history-hook-session"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-hook-session"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );

    let (nodes, _, _) = store.load_history_graph(Some("history-hook-session"));
    let snapshot = store
        .checkout_history_node(
            Some("history-hook-session"),
            nodes[0].node_id.as_str(),
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("checkout should succeed");

    assert_eq!(snapshot.history_state_evidence.len(), 2);
    assert_eq!(
        snapshot.history_state_evidence[0].boundary,
        "history.checkout.start"
    );
    assert_eq!(
        snapshot.history_state_evidence[1].boundary,
        "history.checkout.resolved"
    );
    assert_eq!(
        snapshot.history_state_evidence[1]
            .resolved_node_id
            .as_deref(),
        Some(nodes[0].node_id.as_str())
    );
    assert!(!snapshot.history_state_evidence[1].degraded);
}

#[test]
fn checkout_history_node_blocked_by_hook_persists_only_start_evidence() {
    let mut store = SessionStore::memory_only();
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.guard.deny".to_string(),
            hook_class: crate::agent::hooks::HookClass::Guard,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Deny,
            structured_result: HookStructuredResult::Deny(crate::agent::hooks::HookDenyDecision {
                reason_code: "history_checkout_blocked".to_string(),
                message: "history checkout denied by guard".to_string(),
            }),
            blocked: true,
            elapsed_ms: 1,
            input_summary: Some("checkout denied".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout denied".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.resolved.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "should not execute".to_string(),
            },
            blocked: false,
            elapsed_ms: 1,
            input_summary: Some("unexpected resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "unexpected resolved".to_string(),
        }],
    }));
    store.append_turn(
        Some("history-hook-blocked"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-hook-blocked"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );

    let live_before = store.snapshot(Some("history-hook-blocked"), &[]);
    let latest_before = live_before
        .history_cursor
        .visible_node_id
        .clone()
        .expect("latest visible node before blocked checkout");
    let (nodes, _, _) = store.load_history_graph(Some("history-hook-blocked"));
    let error = store
        .checkout_history_node(
            Some("history-hook-blocked"),
            nodes[0].node_id.as_str(),
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect_err("checkout should be blocked by hook");
    assert!(error.contains("history checkout blocked by hook"));

    let live_after = store.snapshot(Some("history-hook-blocked"), &[]);
    assert_eq!(live_after.history_state_evidence.len(), 1);
    assert_eq!(
        live_after.history_state_evidence[0].boundary,
        "history.checkout.start"
    );
    assert_eq!(live_after.history_state_evidence[0].resolved_node_id, None);
    assert_eq!(
        live_after.history_cursor.visible_node_id.as_deref(),
        Some(latest_before.as_str())
    );
    assert_eq!(live_after.history_cursor.mode, HistoryCursorMode::Live);
}

#[test]
fn history_state_hook_evidence_roundtrip_through_file_backend() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.guard.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout start observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 2,
            input_summary: Some("checkout start".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout start observed".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.resolved.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout resolved".to_string(),
            },
            blocked: false,
            elapsed_ms: 3,
            input_summary: Some("checkout resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout resolved".to_string(),
        }],
    }));
    store.append_turn(
        Some("history-hook-roundtrip"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-hook-roundtrip"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );

    let (nodes, _, _) = store.load_history_graph(Some("history-hook-roundtrip"));
    store
        .checkout_history_node(
            Some("history-hook-roundtrip"),
            nodes[0].node_id.as_str(),
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("checkout should succeed");

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("history-hook-roundtrip"), &[]);
    assert_eq!(snapshot.history_state_evidence.len(), 2);
    assert_eq!(
        snapshot.history_state_evidence[0].boundary,
        "history.checkout.start"
    );
    assert_eq!(
        snapshot.history_state_evidence[1].boundary,
        "history.checkout.resolved"
    );
    assert_eq!(
        snapshot.history_state_evidence[1]
            .resolved_node_id
            .as_deref(),
        Some(nodes[0].node_id.as_str())
    );

    let _ = fs::remove_file(path);
}

#[test]
fn restore_branch_head_persists_history_state_hook_evidence() {
    let mut store = SessionStore::memory_only();
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.restore.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history restore start observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 2,
            input_summary: Some("restore start".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history restore start observed".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.restore.resolved".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history restore resolved".to_string(),
            },
            blocked: false,
            elapsed_ms: 3,
            input_summary: Some("restore resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history restore resolved".to_string(),
        }],
    }));
    store.append_turn(
        Some("history-restore-session"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-restore-session"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );

    let snapshot = store
        .restore_branch_head(Some("history-restore-session"), Some("branch-main"), None)
        .expect("restore should succeed");

    assert_eq!(snapshot.history_state_evidence.len(), 2);
    assert_eq!(
        snapshot.history_state_evidence[0].boundary,
        "history.branch_restore.start"
    );
    assert_eq!(
        snapshot.history_state_evidence[1].boundary,
        "history.branch_restore.resolved"
    );
    assert_eq!(
        snapshot.history_state_evidence[1]
            .resolved_branch_id
            .as_deref(),
        Some("branch-main")
    );
    assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
}

#[test]
fn fork_from_history_node_blocked_by_hook_persists_only_start_evidence() {
    let mut store = SessionStore::memory_only();
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.fork.deny".to_string(),
            hook_class: crate::agent::hooks::HookClass::Guard,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Deny,
            structured_result: HookStructuredResult::Deny(crate::agent::hooks::HookDenyDecision {
                reason_code: "history_fork_blocked".to_string(),
                message: "history fork denied by guard".to_string(),
            }),
            blocked: true,
            elapsed_ms: 1,
            input_summary: Some("fork denied".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history fork denied".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.fork.resolved".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "should not execute".to_string(),
            },
            blocked: false,
            elapsed_ms: 1,
            input_summary: Some("unexpected fork resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "unexpected fork resolved".to_string(),
        }],
    }));
    store.append_turn(
        Some("history-fork-blocked"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-fork-blocked"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );

    let live_before = store.snapshot(Some("history-fork-blocked"), &[]);
    let latest_before = live_before
        .history_cursor
        .visible_node_id
        .clone()
        .expect("latest visible node before blocked fork");
    let (nodes, branches, _) = store.load_history_graph(Some("history-fork-blocked"));
    let error = store
        .fork_from_history_node(
            Some("history-fork-blocked"),
            nodes[0].node_id.as_str(),
            None,
        )
        .expect_err("fork should be blocked by hook");
    assert!(error.contains("history branch fork blocked by hook"));

    let live_after = store.snapshot(Some("history-fork-blocked"), &[]);
    assert_eq!(live_after.history_state_evidence.len(), 1);
    assert_eq!(
        live_after.history_state_evidence[0].boundary,
        "history.branch_fork.start"
    );
    assert_eq!(
        live_after.history_cursor.visible_node_id.as_deref(),
        Some(latest_before.as_str())
    );
    let (branches_after, _, _) = {
        let (nodes_after, branches_after, cursor_after) =
            store.load_history_graph(Some("history-fork-blocked"));
        (branches_after, nodes_after, cursor_after)
    };
    assert_eq!(branches_after.len(), branches.len());
}

#[test]
fn switch_history_branch_persists_history_state_hook_evidence() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("history-switch-session"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-switch-session"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );
    let (nodes, _, _) = store.load_history_graph(Some("history-switch-session"));
    store
        .fork_from_history_node(
            Some("history-switch-session"),
            nodes[0].node_id.as_str(),
            None,
        )
        .expect("fork should succeed before switch test");
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.switch.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history switch start observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 2,
            input_summary: Some("switch start".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history switch start observed".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.switch.resolved".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history switch resolved".to_string(),
            },
            blocked: false,
            elapsed_ms: 3,
            input_summary: Some("switch resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history switch resolved".to_string(),
        }],
    }));

    let snapshot = store
        .switch_history_branch(Some("history-switch-session"), "branch-main", None)
        .expect("switch should succeed");

    assert_eq!(snapshot.history_state_evidence.len(), 2);
    assert_eq!(
        snapshot.history_state_evidence[0].boundary,
        "history.branch_switch.start"
    );
    assert_eq!(
        snapshot.history_state_evidence[1].boundary,
        "history.branch_switch.resolved"
    );
    assert_eq!(
        snapshot.history_state_evidence[1]
            .resolved_branch_id
            .as_deref(),
        Some("branch-main")
    );
    assert_eq!(
        snapshot.history_cursor.active_branch_id.as_deref(),
        Some("branch-main")
    );
}

#[test]
fn checkout_history_node_preserves_degraded_truth_source_with_hooks() {
    let mut store = SessionStore::memory_only();
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.checkout.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout start observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 2,
            input_summary: Some("checkout start".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout start observed".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.checkout.resolved".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout resolved".to_string(),
            },
            blocked: false,
            elapsed_ms: 3,
            input_summary: Some("checkout resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout resolved".to_string(),
        }],
    }));
    store.append_turn(
        Some("history-degrade-truth"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-degrade-truth"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );

    let (nodes, _, _) = store.load_history_graph(Some("history-degrade-truth"));
    let snapshot = store
        .checkout_history_node(
            Some("history-degrade-truth"),
            nodes[0].node_id.as_str(),
            HistoryCheckoutMode::TranscriptAndWorkspace,
            None,
        )
        .expect("degraded checkout should succeed");

    assert_eq!(
        snapshot.history_cursor.checkout_mode,
        HistoryCheckoutMode::TranscriptAndWorkspace
    );
    assert_eq!(
        snapshot.history_cursor.checkout_status,
        HistoryCheckoutStatus::DegradedToTranscriptOnly
    );
    // After truncation the target node IS the branch head, so mode is Live.
    assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
    assert_eq!(snapshot.history_state_evidence.len(), 2);
    assert_eq!(
        snapshot.history_state_evidence[1].boundary,
        "history.checkout.resolved"
    );
    assert!(snapshot.history_state_evidence[1].degraded);
    assert_eq!(
        snapshot.history_state_audit_summary.action.status,
        "available"
    );
    assert_eq!(
        snapshot
            .history_state_audit_summary
            .action
            .boundary
            .as_deref(),
        Some("history.checkout.resolved")
    );
    assert!(snapshot.history_state_audit_summary.action.degraded);
    assert_eq!(
        snapshot.history_state_audit_summary.current_context.mode,
        "live"
    );
}

#[test]
fn missing_history_state_evidence_does_not_reconstruct_restore_conclusion_after_reload() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
        start_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.checkout.observe".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout start observed".to_string(),
            },
            blocked: false,
            elapsed_ms: 2,
            input_summary: Some("checkout start".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout start observed".to_string(),
        }],
        resolved_results: vec![crate::agent::hooks::HookExecutionResult {
            hook_name: "history.checkout.resolved".to_string(),
            hook_class: crate::agent::hooks::HookClass::Observe,
            hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
            hook_order: 1,
            result_kind: HookResultKind::Observe,
            structured_result: HookStructuredResult::Observe {
                summary: "history checkout resolved".to_string(),
            },
            blocked: false,
            elapsed_ms: 3,
            input_summary: Some("checkout resolved".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "history checkout resolved".to_string(),
        }],
    }));
    store.append_turn(
        Some("history-missing-evidence"),
        "第一轮对话",
        "收到第一轮。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-missing-evidence"),
        "第二轮对话",
        "收到第二轮。",
        None,
        Vec::new(),
    );

    let (nodes, _, _) = store.load_history_graph(Some("history-missing-evidence"));
    let initial = store
        .checkout_history_node(
            Some("history-missing-evidence"),
            nodes[0].node_id.as_str(),
            HistoryCheckoutMode::TranscriptAndWorkspace,
            None,
        )
        .expect("degraded checkout should succeed");
    // TranscriptAndWorkspace checkout on a non-rollback-capable node
    // degrades to transcript-only, so the cursor status reflects the
    // degrade conclusion rather than NotRequested (d6e1fbf semantics).
    assert_eq!(
        initial.history_cursor.checkout_status,
        HistoryCheckoutStatus::DegradedToTranscriptOnly
    );
    assert_eq!(initial.history_state_evidence.len(), 2);
    let resolved_node_id = initial
        .resolved_node_id
        .clone()
        .expect("resolved node id after checkout");

    {
        let session = store
            .sessions
            .get_mut("history-missing-evidence")
            .expect("persisted session should exist");
        session.history_state_evidence.clear();
    }
    store.save_to_backend();

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("history-missing-evidence"), &[]);
    assert!(snapshot.history_state_evidence.is_empty());
    assert_eq!(
        snapshot.history_state_audit_summary.action.status,
        "missing"
    );
    assert_eq!(
        snapshot.history_cursor.checkout_status,
        HistoryCheckoutStatus::DegradedToTranscriptOnly
    );
    assert_eq!(
        snapshot.history_cursor.checkout_mode,
        HistoryCheckoutMode::TranscriptAndWorkspace
    );
    // After destructive truncation the checked-out node IS the branch
    // head, so the persisted mode is Live rather than Historical.
    assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
    assert_eq!(
        snapshot.resolved_node_id.as_deref(),
        Some(resolved_node_id.as_str())
    );

    let _ = fs::remove_file(path);
}

#[test]
fn append_turn_extracts_explicit_user_preferences_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-write"),
        "请用中文回复，并尽量简洁。",
        "好的，我会用中文并尽量简洁。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-write"),
        "请用中文回复，并尽量简洁。",
        "收到。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-write"), &[]);
    assert_eq!(snapshot.long_term_memory_entries.len(), 2);
    assert_eq!(
        snapshot.long_term_memory_entries[0].kind,
        "user_preference.response_language"
    );
    assert_eq!(
        snapshot.long_term_memory_entries[0].source,
        "explicit_user_message"
    );
    assert_eq!(
        snapshot.long_term_memory_entries[1].kind,
        "user_preference.response_style"
    );
}

#[test]
fn append_turn_extracts_explicit_memory_note_without_overwriting_preferences() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-note"),
        "请用中文回复，并尽量简洁。",
        "好的。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-note"),
        "请记住这个项目当前优先推进 PA-018。",
        "我会记住这条信息。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-note"), &[]);
    assert_eq!(snapshot.long_term_memory_entries.len(), 4);
    assert!(snapshot
        .long_term_memory_entries
        .iter()
        .any(|entry| entry.kind == "user_preference.response_language"));
    assert!(snapshot
        .long_term_memory_entries
        .iter()
        .any(|entry| entry.kind == "user_preference.response_style"));
    assert!(snapshot.long_term_memory_entries.iter().any(|entry| {
        entry.kind == "user_memory.explicit_note"
            && entry.content == "这个项目当前优先推进 PA-018。"
            && entry.source == "explicit_user_message"
    }));
    assert!(snapshot.long_term_memory_entries.iter().any(|entry| {
        entry.kind == "project_focus.active_task"
            && entry.content == "Current active task is PA-018."
            && entry.source == "explicit_user_message"
    }));
}

#[test]
fn append_turn_extracts_explicit_file_reference_preference_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-path-style"),
        "引用文件时请使用绝对路径，不要使用相对路径。",
        "好的，后续引用文件时我会使用绝对路径。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-path-style"),
        "引用文件时请使用绝对路径，不要使用相对路径。",
        "收到。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-path-style"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "user_preference.file_reference_style")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Use absolute paths when referencing workspace files."
    );
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_extracts_task_system_sync_preference_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-task-sync"),
        "推进 PA-018 的同时记得更新任务文档，并同步任务系统。",
        "好的，我会同步更新任务文档和任务系统。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-task-sync"),
        "推进 PA-018 的同时记得更新任务文档，并同步任务系统。",
        "收到。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-task-sync"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "user_preference.task_system_sync")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Keep task-system documents updated while progressing work."
    );
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_extracts_change_scope_preference_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-change-scope"),
        "不要修改无关文件，也不要回滚无关改动。",
        "好的，我会避免修改无关文件和无关改动。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-change-scope"),
        "不要修改无关文件，也不要回滚无关改动。",
        "收到。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-change-scope"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "user_preference.change_scope")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Avoid modifying unrelated existing changes."
    );
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_extracts_acceptance_gate_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-acceptance-gate"),
        "现在开始 PA-018 任务，建立验收标准，确保执行成功完成交付，并更新任务文档。",
        "收到，我会先建立验收标准并持续回写任务文档。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-acceptance-gate"),
        "现在开始 PA-018 任务，建立验收标准，确保执行成功完成交付，并更新任务文档。",
        "继续推进。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-acceptance-gate"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "project_workflow.acceptance_gate")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Establish acceptance criteria and run a closeout audit before claiming delivery."
    );
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_extracts_project_dependency_prerequisite_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-prerequisite"),
        "先完成 PA-017，再做 PA-018。",
        "收到，我会先确认前置任务。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-prerequisite"),
        "先完成 PA-017，再做 PA-018。",
        "继续推进。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-prerequisite"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "project_dependency.prerequisite")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(matching_entries[0].content, "PA-018 depends on PA-017.");
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_extracts_closeout_requirement_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-closeout"),
        "完成后说明改了哪些文件、做了什么验证、还有什么未解决风险。",
        "收到，我会按这个收口口径汇报。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-closeout"),
        "完成后说明改了哪些文件、做了什么验证、还有什么未解决风险。",
        "继续。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-closeout"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "project_workflow.closeout_requirement")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Summarize changed files, verification performed, and unresolved risks at closeout."
    );
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_extracts_task_boundary_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-task-boundary"),
        "目标是 PA-018，不能越界到 PA-024、PA-025。",
        "收到，我会控制范围。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-task-boundary"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "project_scope.task_boundary")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Do not expand scope into PA-024, PA-025."
    );
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_extracts_explicit_active_task_focus_into_long_term_memory() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-active-task"),
        "现在开始 PA-018 任务，当前优先推进这个任务。",
        "好的，我会优先推进 PA-018。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-active-task"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "project_focus.active_task")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Current active task is PA-018."
    );
    assert_eq!(matching_entries[0].source, "explicit_user_message");
}

#[test]
fn append_turn_updates_active_task_focus_instead_of_accumulating_duplicates() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-active-task-update"),
        "现在开始 PA-018 任务，当前优先推进这个任务。",
        "好的，我会优先推进 PA-018。",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("memory-active-task-update"),
        "现在开始 PA-020 任务，后续优先推进这个任务。",
        "好的，我会切换到 PA-020。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-active-task-update"), &[]);
    let matching_entries = snapshot
        .long_term_memory_entries
        .iter()
        .filter(|entry| entry.kind == "project_focus.active_task")
        .collect::<Vec<_>>();
    assert_eq!(matching_entries.len(), 1);
    assert_eq!(
        matching_entries[0].content,
        "Current active task is PA-020."
    );
}

#[test]
fn append_turn_does_not_extract_active_task_focus_from_incidental_task_mentions() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-active-task-incidental"),
        "PA-018 看起来像是目前其他任务的前置任务，对吗？",
        "是的，它像一个前置任务。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-active-task-incidental"), &[]);
    assert!(!snapshot
        .long_term_memory_entries
        .iter()
        .any(|entry| entry.kind == "project_focus.active_task"));
}

#[test]
fn append_turn_does_not_extract_acceptance_gate_from_incidental_acceptance_mentions() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("memory-incidental-acceptance"),
        "帮我看看这个目录里有没有验收标准文档模板。",
        "我先去定位相关文档。",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("memory-incidental-acceptance"), &[]);
    assert!(!snapshot
        .long_term_memory_entries
        .iter()
        .any(|entry| entry.kind == "project_workflow.acceptance_gate"));
}

#[test]
fn attachment_payloads_can_be_restored_from_recent_history() {
    let path = temp_sessions_path();
    let attachment_root = path
        .parent()
        .map(|parent| parent.join("attachments"))
        .expect("attachment root");
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    let images = vec![TurnInputImage {
        data_url: "data:image/png;base64,AAAA".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("diagram.png".to_string()),
    }];

    let attachments = store
        .save_input_attachments("with-attachments", &images)
        .expect("save attachments");
    store.append_turn(
        Some("with-attachments"),
        "[已附图片 1 张：diagram.png]",
        "我看到了这张图。",
        None,
        attachments,
    );

    let restored = store.load_recent_images(Some("with-attachments"), 1);
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].data_url, images[0].data_url);
    assert_eq!(restored[0].mime_type, "image/png");
    let snapshot = store.snapshot(Some("with-attachments"), &[]);
    assert_eq!(snapshot.attachment_assets.len(), 1);
    assert_eq!(snapshot.attachment_assets[0].mime_type, "image/png");
    assert_eq!(
        store.list_attachment_assets(Some("with-attachments")).len(),
        1
    );

    store.remove_session("with-attachments");
    assert!(!attachment_root.join("with-attachments").exists());

    let _ = fs::remove_file(path);
}

#[test]
fn recent_images_only_recall_from_latest_user_turn() {
    let mut store = SessionStore::memory_only();
    let images = vec![TurnInputImage {
        data_url: "data:image/png;base64,AAAA".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("diagram.png".to_string()),
    }];

    let attachments = store
        .save_input_attachments("latest-only", &images)
        .expect("save attachments");
    store.append_turn(
        Some("latest-only"),
        "[已附图片 1 张：diagram.png]",
        "我看到了这张图。",
        None,
        attachments,
    );
    store.append_turn(
        Some("latest-only"),
        "继续查看 runtime.rs。",
        "好的，我继续查看代码。",
        None,
        Vec::new(),
    );

    let restored = store.load_recent_images(Some("latest-only"), 1);
    assert!(restored.is_empty());
}

#[test]
fn attachment_assets_are_indexed_across_sessions_without_scanning_history() {
    let mut store = SessionStore::memory_only();
    let alpha_images = vec![TurnInputImage {
        data_url: "data:image/png;base64,AAAA".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("alpha.png".to_string()),
    }];
    let beta_images = vec![TurnInputImage {
        data_url: "data:image/jpeg;base64,BBBB".to_string(),
        mime_type: "image/jpeg".to_string(),
        name: Some("beta.jpg".to_string()),
    }];

    let alpha_attachments = store
        .save_input_attachments("alpha", &alpha_images)
        .expect("save alpha attachments");
    store.append_turn(
        Some("alpha"),
        "[已附图片 1 张：alpha.png]",
        "我看到了 alpha 图片。",
        None,
        alpha_attachments,
    );

    let beta_attachments = store
        .save_input_attachments("beta", &beta_images)
        .expect("save beta attachments");
    store.append_turn(
        Some("beta"),
        "[已附图片 1 张：beta.jpg]",
        "我看到了 beta 图片。",
        None,
        beta_attachments,
    );

    let all_assets = store.list_attachment_assets(None);
    assert_eq!(all_assets.len(), 2);
    assert_eq!(store.list_attachment_assets(Some("alpha")).len(), 1);
    assert_eq!(store.list_attachment_assets(Some("beta")).len(), 1);
}

#[test]
fn doc_text_reference_attachments_do_not_materialize_phantom_assets() {
    // PA-078 AC#4：doc/text 以引用附着（.tmp/imports/...），不应进入 AttachmentAsset 目录
    // （relative_path 相对 attachment_root 不存在 → 否则物化为 MissingPayload 幽灵资产）。
    let mut store = SessionStore::memory_only();
    let session_id = format!("docref-{}", now_timestamp_ms());
    let doc_attachment = AttachmentReference {
        id: "ref-doc-1".to_string(),
        asset_id: String::new(),
        name: Some("foo.md".to_string()),
        mime_type: "text/markdown".to_string(),
        relative_path: ".tmp/imports/foo.md".to_string(),
        size_bytes: 12,
        created_at_ms: now_timestamp_ms(),
    };
    store.append_turn(
        Some(&session_id),
        "[附件: foo.md]",
        "已收到。",
        None,
        vec![doc_attachment.clone()],
    );
    store.refresh_attachment_catalog();

    let phantom_id = attachment_asset_id(".tmp/imports/foo.md");
    assert!(
        !store.attachment_assets.contains_key(&phantom_id),
        "doc/text 引用不应物化为 AttachmentAsset（phantom: {phantom_id}）"
    );

    // 对照：图片资产仍正常物化
    let images = vec![TurnInputImage {
        data_url: "data:image/png;base64,AAAA".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("active.png".to_string()),
    }];
    let image_attachments = store
        .save_input_attachments(&session_id, &images)
        .expect("save image attachments");
    store.append_turn(
        Some(&session_id),
        "[image: active.png]",
        "看到了。",
        None,
        image_attachments,
    );
    store.refresh_attachment_catalog();
    assert_eq!(
        store.list_attachment_assets(None).len(),
        1,
        "图片资产应保留"
    );
}

#[test]
fn session_store_workspace_registry_default_and_crud() {
    // PA-079：默认 workspace 始终存在；create/list/resolve 正确。
    let mut store = SessionStore::memory_only();
    let workspaces = store.list_workspaces();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(
        workspaces[0].id,
        crate::agent::workspace::DEFAULT_WORKSPACE_ID
    );

    let root = std::env::temp_dir().join(format!("pa079-store-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let created = store
        .create_workspace("Docs", root.to_str().unwrap())
        .unwrap();
    assert!(created.id.starts_with("ws-docs-"));

    assert_eq!(store.list_workspaces().len(), 2);
    assert_eq!(
        store.resolve_workspace_root(Some(&created.id)).unwrap(),
        crate::agent::workspace::normalize_workspace_root(root.to_str().unwrap()).unwrap()
    );
    // 缺省 → 默认 workspace
    assert!(store.resolve_workspace_root(None).is_ok());

    // 重复 root 拒绝
    assert!(store
        .create_workspace("Docs2", root.to_str().unwrap())
        .is_err());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn session_store_stamps_workspace_id_only_once() {
    // PA-079：TurnInput.workspace_id 首次盖章；后续轮 no-op。
    let root = std::env::temp_dir().join(format!("pa-stamp-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut store = SessionStore::memory_only();
    let registered = store.create_workspace("Proj", &root.display().to_string()).unwrap();
    store.ensure_session("ws-session");
    assert!(store.sessions["ws-session"].workspace_id.is_none());

    store.stamp_workspace_id("ws-session", &registered.id);
    assert_eq!(
        store.sessions["ws-session"].workspace_id.as_deref(),
        Some(registered.id.as_str())
    );

    // 已盖章 → 后续（哪怕不同 id）no-op。
    store.stamp_workspace_id("ws-session", "default");
    assert_eq!(
        store.sessions["ws-session"].workspace_id.as_deref(),
        Some(registered.id.as_str())
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn stamp_normalizes_unregistered_workspace_to_default() {
    // 三级树纵深防御（裁决②）：已注销/未知 id 盖章一律归一 default——
    // 陈旧提交不得把会话重新盖成死 id（附件硬失败 vs 工具根软回退的分叉）。
    let mut store = SessionStore::memory_only();
    store.ensure_session("stale");
    store.stamp_workspace_id("stale", "ws-deleted-long-ago");
    assert_eq!(
        store.sessions["stale"].workspace_id.as_deref(),
        Some(crate::agent::workspace::DEFAULT_WORKSPACE_ID)
    );
}

#[test]
fn legacy_session_without_workspace_id_projects_to_default() {
    // PA-079：旧会话（无 workspace_id 字段）serde 兼容 + 快照投影 None。
    let mut store = SessionStore::memory_only();
    store.ensure_session("legacy");
    let snapshot = store.snapshot(Some("legacy"), &[]);
    assert_eq!(snapshot.workspace_id, None);

    // P2-7b：模拟旧 schema blob（无 workspace_id 字段）→ 全字段原样往返 + workspace_id=None。
    let mut legacy_session = store.sessions["legacy"].clone();
    legacy_session.title = "旧标题".to_string();
    legacy_session.summary = "旧摘要".to_string();
    legacy_session.turn_count = 3;
    legacy_session.updated_at_ms = 12345;
    legacy_session.history = vec![TurnHistoryMessage {
        role: "user".to_string(),
        content: "旧内容".to_string(),
        attachments: Vec::new(),
        ..Default::default()
    }];
    let serialized = serde_json::to_string(&legacy_session).unwrap();
    // 去掉 workspace_id 键模拟旧版本写入的 blob
    let mut value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    if let serde_json::Value::Object(map) = &mut value {
        map.remove("workspaceId");
    }
    let restored: SessionState = serde_json::from_str(&value.to_string()).unwrap();
    assert_eq!(
        restored.workspace_id, None,
        "旧 blob 无 workspace_id → None"
    );
    assert_eq!(restored.title, "旧标题");
    assert_eq!(restored.summary, "旧摘要");
    assert_eq!(restored.turn_count, 3);
    assert_eq!(restored.updated_at_ms, 12345);
    assert_eq!(restored.history.len(), 1);
    assert_eq!(restored.history[0].content, "旧内容");
}

#[test]
fn corrupt_file_backend_falls_back_to_default_workspace() {
    // PA-079 P1-2：File backend 整文件损坏 → load_store None → 默认 workspace 重建。
    let dir = std::env::temp_dir().join(format!("pa079-file-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let storage_path = dir.join("store.json");
    std::fs::write(&storage_path, "{bad json").unwrap();

    let backend = FileSessionBackend {
        storage_path: storage_path.clone(),
    };
    let store = SessionStore::with_backend(Box::new(backend));
    let workspaces = store.list_workspaces();
    assert_eq!(workspaces.len(), 1, "损坏回退后应只剩默认 workspace");
    assert_eq!(
        workspaces[0].id,
        crate::agent::workspace::DEFAULT_WORKSPACE_ID
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn attachment_assets_expose_lifecycle_statuses_and_queries() {
    let mut store = SessionStore::memory_only();
    let session_id = format!("lifecycle-{}", now_timestamp_ms());
    let active_images = vec![TurnInputImage {
        data_url: "data:image/png;base64,AAAA".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("active.png".to_string()),
    }];
    let missing_images = vec![TurnInputImage {
        data_url: "data:image/jpeg;base64,BBBB".to_string(),
        mime_type: "image/jpeg".to_string(),
        name: Some("missing.jpg".to_string()),
    }];
    let reclaimable_images = vec![TurnInputImage {
        data_url: "data:image/webp;base64,CCCC".to_string(),
        mime_type: "image/webp".to_string(),
        name: Some("draft.webp".to_string()),
    }];
    let expired_images = vec![TurnInputImage {
        data_url: "data:image/gif;base64,DDDD".to_string(),
        mime_type: "image/gif".to_string(),
        name: Some("old.gif".to_string()),
    }];

    let active_attachments = store
        .save_input_attachments(&session_id, &active_images)
        .expect("save active attachments");
    store.append_turn(
        Some(&session_id),
        "[image 1: active.png]",
        "active image is still referenced",
        None,
        active_attachments,
    );

    // 每次保存之间推进时钟，避免同一毫秒内 asset_id 碰撞（asset_id 基于 created_at_ms）
    std::thread::sleep(std::time::Duration::from_millis(2));
    let missing_attachments = store
        .save_input_attachments(&session_id, &missing_images)
        .expect("save missing attachments");
    let missing_relative_path = missing_attachments[0].relative_path.clone();
    store.append_turn(
        Some(&session_id),
        "[image 1: missing.jpg]",
        "missing image is still referenced",
        None,
        missing_attachments.clone(),
    );
    let _ = fs::remove_file(store.attachment_root.join(&missing_relative_path));

    std::thread::sleep(std::time::Duration::from_millis(2));
    let reclaimable_attachments = store
        .save_input_attachments(&session_id, &reclaimable_images)
        .expect("save reclaimable attachments");
    let reclaimable_asset_id = reclaimable_attachments[0].asset_id.clone();

    std::thread::sleep(std::time::Duration::from_millis(2));
    let expired_attachments = store
        .save_input_attachments(&session_id, &expired_images)
        .expect("save expired attachments");
    let expired_asset_id = expired_attachments[0].asset_id.clone();
    let expired_created_at_ms = now_timestamp_ms()
        .saturating_sub(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS)
        .saturating_sub(1_000);
    store
        .attachment_assets
        .get_mut(&expired_asset_id)
        .expect("expired asset")
        .created_at_ms = expired_created_at_ms;

    let all_assets = store.list_attachment_assets(None);
    let active_asset = all_assets
        .iter()
        .find(|asset| asset.name.as_deref() == Some("active.png"))
        .expect("active asset");
    let missing_asset = all_assets
        .iter()
        .find(|asset| asset.name.as_deref() == Some("missing.jpg"))
        .expect("missing asset");
    let reclaimable_asset = all_assets
        .iter()
        .find(|asset| asset.id == reclaimable_asset_id)
        .expect("reclaimable asset");
    let expired_asset = all_assets
        .iter()
        .find(|asset| asset.id == expired_asset_id)
        .expect("expired asset");

    assert_eq!(active_asset.status, AttachmentLifecycleStatus::Active);
    assert_eq!(active_asset.reference_count, 1);
    assert_eq!(
        missing_asset.status,
        AttachmentLifecycleStatus::MissingPayload
    );
    assert_eq!(missing_asset.reference_count, 1);
    assert_eq!(
        reclaimable_asset.status,
        AttachmentLifecycleStatus::Reclaimable
    );
    assert_eq!(reclaimable_asset.reference_count, 0);
    assert_eq!(expired_asset.status, AttachmentLifecycleStatus::Expired);
    assert_eq!(expired_asset.reference_count, 0);
    assert!(expired_asset.expires_at_ms.is_some());

    let filtered = store.query_attachment_assets(&AttachmentAssetQuery {
        session_id: Some(session_id.clone()),
        mime_type: Some("jpeg".to_string()),
        name_contains: Some("missing".to_string()),
        created_after_ms: None,
        created_before_ms: None,
        statuses: vec![AttachmentLifecycleStatus::MissingPayload],
        limit: Some(1),
    });
    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered[0].status,
        AttachmentLifecycleStatus::MissingPayload
    );

    let expired_only = store.query_attachment_assets(&AttachmentAssetQuery {
        created_before_ms: Some(expired_created_at_ms),
        statuses: vec![AttachmentLifecycleStatus::Expired],
        ..AttachmentAssetQuery::default()
    });
    assert_eq!(expired_only.len(), 1);
    assert_eq!(expired_only[0].id, expired_asset_id);
}

#[test]
fn cleanup_attachment_assets_only_reclaims_unreferenced_payloads() {
    let mut store = SessionStore::memory_only();
    let session_id = format!("cleanup-{}", now_timestamp_ms());
    let active_images = vec![TurnInputImage {
        data_url: "data:image/png;base64,AAAA".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("keep.png".to_string()),
    }];
    let reclaimable_images = vec![TurnInputImage {
        data_url: "data:image/png;base64,BBBB".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("trash.png".to_string()),
    }];
    let expired_images = vec![TurnInputImage {
        data_url: "data:image/png;base64,CCCC".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("old.png".to_string()),
    }];
    let missing_images = vec![TurnInputImage {
        data_url: "data:image/png;base64,DDDD".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("missing.png".to_string()),
    }];

    let active_attachments = store
        .save_input_attachments(&session_id, &active_images)
        .expect("save active attachments");
    let active_relative_path = active_attachments[0].relative_path.clone();
    store.append_turn(
        Some(&session_id),
        "[image 1: keep.png]",
        "keep it",
        None,
        active_attachments,
    );

    // 每次保存之间推进时钟，避免同一毫秒内 asset_id 碰撞（asset_id 基于 created_at_ms）
    std::thread::sleep(std::time::Duration::from_millis(2));
    let reclaimable_attachments = store
        .save_input_attachments(&session_id, &reclaimable_images)
        .expect("save reclaimable attachments");
    let reclaimable_asset_id = reclaimable_attachments[0].asset_id.clone();
    let reclaimable_path = store
        .attachment_root
        .join(&reclaimable_attachments[0].relative_path);

    std::thread::sleep(std::time::Duration::from_millis(2));
    let expired_attachments = store
        .save_input_attachments(&session_id, &expired_images)
        .expect("save expired attachments");
    let expired_asset_id = expired_attachments[0].asset_id.clone();
    let expired_path = store
        .attachment_root
        .join(&expired_attachments[0].relative_path);
    store
        .attachment_assets
        .get_mut(&expired_asset_id)
        .expect("expired asset")
        .created_at_ms = now_timestamp_ms()
        .saturating_sub(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS)
        .saturating_sub(1_000);

    std::thread::sleep(std::time::Duration::from_millis(2));
    let missing_attachments = store
        .save_input_attachments(&session_id, &missing_images)
        .expect("save missing attachments");
    let missing_asset_id = missing_attachments[0].asset_id.clone();
    let missing_path = store
        .attachment_root
        .join(&missing_attachments[0].relative_path);
    store.append_turn(
        Some(&session_id),
        "[image 1: missing.png]",
        "still referenced",
        None,
        missing_attachments,
    );
    let _ = fs::remove_file(&missing_path);

    let result = store.cleanup_attachment_assets(&AttachmentCleanupRequest {
        session_id: Some(session_id.clone()),
        expire_before_ms: Some(now_timestamp_ms()),
        include_reclaimable: true,
        include_expired: true,
        limit: None,
    });

    assert_eq!(result.removed_catalog_count, 2);
    assert!(result.removed_asset_ids.contains(&reclaimable_asset_id));
    assert!(result.removed_asset_ids.contains(&expired_asset_id));
    assert!(!result.removed_asset_ids.contains(&missing_asset_id));
    assert!(!reclaimable_path.exists());
    assert!(!expired_path.exists());
    assert!(store.attachment_root.join(active_relative_path).exists());
    assert_eq!(store.load_recent_images(Some(&session_id), 1).len(), 0);

    let remaining_assets = store.list_attachment_assets(Some(&session_id));
    assert!(remaining_assets
        .iter()
        .any(|asset| asset.status == AttachmentLifecycleStatus::Active));
    assert!(remaining_assets
        .iter()
        .any(|asset| asset.status == AttachmentLifecycleStatus::MissingPayload));
}

#[test]
fn session_title_uses_first_user_message_preview() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("preview"),
        "Please inspect runtime.rs session switching and trace consistency after tool execution.",
        "I will check it.",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("preview"),
        "Also verify provider fallback behavior.",
        "Done.",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("preview"), &[]);
    assert_eq!(snapshot.title, "Please inspect runtime.rs se...");
}

#[test]
fn removing_last_session_recreates_default_session() {
    let mut store = SessionStore::memory_only();
    let sessions = store.remove_session(DEFAULT_SESSION_ID);

    assert!(sessions.is_empty());
    let snapshot = store.snapshot(Some(DEFAULT_SESSION_ID), &[]);
    assert_eq!(snapshot.conversation_id, DEFAULT_SESSION_ID);
    assert_eq!(snapshot.title, DEFAULT_SESSION_TITLE);
}

#[test]
fn removing_session_clears_attachment_catalog_without_full_refresh() {
    let mut store = SessionStore::memory_only();
    let images = vec![TurnInputImage {
        data_url: "data:image/png;base64,AAAA".to_string(),
        mime_type: "image/png".to_string(),
        name: Some("delete-me.png".to_string()),
    }];

    let attachments = store
        .save_input_attachments("remove-attachments", &images)
        .expect("save attachments");
    store.append_turn(
        Some("remove-attachments"),
        "[已附图片 1 张：delete-me.png]",
        "我看到了图片。",
        None,
        attachments,
    );

    assert_eq!(
        store
            .list_attachment_assets(Some("remove-attachments"))
            .len(),
        1
    );
    assert_eq!(
        store
            .session_attachment_index
            .get("remove-attachments")
            .map(Vec::len),
        Some(1)
    );

    store.remove_session("remove-attachments");

    assert!(store
        .list_attachment_assets(Some("remove-attachments"))
        .is_empty());
    assert!(!store
        .session_attachment_index
        .contains_key("remove-attachments"));
    assert!(store
        .attachment_assets
        .values()
        .all(|asset| asset.session_id != "remove-attachments"));
}

#[test]
fn snapshot_clears_legacy_native_transcript_without_reasoning_content() {
    let session_id = "legacy-reasoning";
    let mut store = SessionStore::memory_only();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![TurnHistoryMessage {
                role: "user".to_string(),
                attachments: Vec::new(),
                content: "继续".to_string(),
                ..Default::default()
            }],
            provider_native_transcript: vec![
                serde_json::json!({
                    "role": "user",
                    "content": "看看文件"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_legacy",
                            "type": "function",
                            "function": {
                                "name": "workspace_list_files",
                                "arguments": "{}"
                            }
                        }
                    ]
                }),
            ],
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let snapshot = store.snapshot(Some(session_id), &[]);
    assert!(snapshot.provider_native_transcript.is_empty());
}

#[test]
fn snapshot_clears_tool_turn_transcript_when_final_assistant_lacks_reasoning() {
    let session_id = "legacy-tool-final";
    let mut store = SessionStore::memory_only();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![TurnHistoryMessage {
                role: "user".to_string(),
                attachments: Vec::new(),
                content: "继续".to_string(),
                ..Default::default()
            }],
            provider_native_transcript: vec![
                serde_json::json!({
                    "role": "user",
                    "content": "读取 tauri.conf.json"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "reasoning_content": "先读取文件。",
                    "tool_calls": [
                        {
                            "id": "call_tool",
                            "type": "function",
                            "function": {
                                "name": "workspace_read_file",
                                "arguments": "{\"path\":\"src-tauri/tauri.conf.json\"}"
                            }
                        }
                    ]
                }),
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": "call_tool",
                    "content": "{...}"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "这是 Tauri 配置文件。"
                }),
            ],
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: Some("src-tauri/tauri.conf.json".to_string()),
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let snapshot = store.snapshot(Some(session_id), &[]);
    assert!(snapshot.provider_native_transcript.is_empty());
}

#[test]
fn snapshot_clears_incomplete_native_tool_roundtrip() {
    let session_id = "incomplete-tool-roundtrip";
    let mut store = SessionStore::memory_only();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    attachments: Vec::new(),
                    content: "请记住 tauri.conf.json".to_string(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    attachments: Vec::new(),
                    content: "搜索没有直接命中，让我查看一下工作区的文件结构。".to_string(),
                    ..Default::default()
                },
            ],
            provider_native_transcript: vec![
                serde_json::json!({
                    "role": "user",
                    "content": "请记住 tauri.conf.json"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "先搜索文件",
                    "reasoning_content": "先搜索一下。",
                    "tool_calls": [
                        {
                            "id": "call_search",
                            "type": "function",
                            "function": {
                                "name": "workspace_search_text",
                                "arguments": "{\"query\":\"tauri.conf.json\"}"
                            }
                        }
                    ]
                }),
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": "call_search",
                    "content": "{\"matchCount\":0}"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "搜索没有直接命中，让我查看一下工作区的文件结构。",
                    "reasoning_content": "继续列目录。",
                    "tool_calls": [
                        {
                            "id": "call_list",
                            "type": "function",
                            "function": {
                                "name": "workspace_list_files",
                                "arguments": "{\"path\":\".\"}"
                            }
                        }
                    ]
                }),
            ],
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: Some("tauri.conf.json".to_string()),
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let snapshot = store.snapshot(Some(session_id), &[]);
    assert!(snapshot.provider_native_transcript.is_empty());
}

#[test]
fn snapshot_keeps_structured_reasoning_content_in_native_transcript() {
    let session_id = "structured-reasoning";
    let mut store = SessionStore::memory_only();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![TurnHistoryMessage {
                role: "user".to_string(),
                attachments: Vec::new(),
                content: "继续".to_string(),
                ..Default::default()
            }],
            provider_native_transcript: vec![
                serde_json::json!({
                    "role": "user",
                    "content": "读取 tauri.conf.json"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "reasoning_content": [
                        { "type": "reasoning", "text": "先读取文件。" }
                    ],
                    "tool_calls": [
                        {
                            "id": "call_tool",
                            "type": "function",
                            "function": {
                                "name": "workspace_read_file",
                                "arguments": "{\"path\":\"src-tauri/tauri.conf.json\"}"
                            }
                        }
                    ]
                }),
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": "call_tool",
                    "content": "{...}"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "reasoning_content": [
                        { "type": "reasoning", "text": "已读取并总结。" }
                    ],
                    "content": "这是 Tauri 配置文件。"
                }),
            ],
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: Some("src-tauri/tauri.conf.json".to_string()),
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let snapshot = store.snapshot(Some(session_id), &[]);
    assert_eq!(snapshot.provider_native_transcript.len(), 4);
}

fn temp_sessions_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("pony-agent-session-test-{stamp}"))
        .join("sessions.json")
}

#[test]
fn snapshot_does_not_persist_new_empty_session() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    let snapshot = store.snapshot(Some("fresh"), &[]);
    assert_eq!(snapshot.conversation_id, "fresh");
    assert_eq!(snapshot.turn_count, 0);

    let persisted = load_store_from_path(&path);
    assert!(persisted.is_some());

    let _ = fs::remove_file(path);
}

#[test]
fn file_backend_roundtrip_restores_turn_trace_history() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.record_turn_trace(
        Some("trace-persisted"),
        TurnTraceRecord {
            turn_id: "turn-1".to_string(),
            session_id: Some("trace-persisted".to_string()),
            event_id: Some("turn-1:4".to_string()),
            event_type: Some("turn.completed".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(4),
            emitted_at_ms: Some(4242),
            title: "检查流式输出".to_string(),
            phase: "completed".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-return".to_string(),
                label: "Return result".to_string(),
                state: "completed".to_string(),
            }],
            trace_timeline: vec![TraceTimelineEntry {
                id: "return-1".to_string(),
                kind: "return".to_string(),
                label: "RETURN RESULT".to_string(),
                state: "completed".to_string(),
                sequence: 1,
                provider_requested_name: Some("ppx".to_string()),
                provider_name: Some("ppx".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                build_context_observation_ref: None,
                tool_activities: Vec::new(),
                text: Some("ok".to_string()),
                reasoning_content: None,
                fallback_reason: None,
                error: None,
                input_tokens: Some(12),
                cache_hit_input_tokens: Some(5),
                reasoning_tokens: Some(3),
                output_tokens: Some(34),
                total_tokens: Some(46),
                first_token_latency_ms: Some(180),
                turn_duration_ms: Some(920),
            }],
            tool_activities: vec![TurnToolActivity {
                id: "tool-1".to_string(),
                name: "workspace.read_file".to_string(),
                canonical_tool_name: Some("Read".to_string()),
                display_name_zh: Some("读取".to_string()),
                status: "done".to_string(),
                description: "读取文件".to_string(),
                arguments_text: Some("{\"path\":\"src/main.ts\"}".to_string()),
                result_text: Some("ok".to_string()),
                duration_seconds: Some(0.12),
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: Some(crate::agent::telemetry::CapabilityInvocationRecord {
                    tool_name: "workspace.read_file".to_string(),
                    capability_id: Some("mcp:tool:workspace.read_file".to_string()),
                    source_id: Some("mcp-local".to_string()),
                    source_kind: Some("mcp".to_string()),
                    capability_kind: Some("tool".to_string()),
                    invocation_mode: Some("direct_tool_call".to_string()),
                    failure_kind: None,
                    requires_approval: Some(false),
                    host_mediated: Some(true),
                    permission_scope: Some("workspace.read".to_string()),
                    permission_facts: Some(crate::agent::tools::ToolPermissionFacts {
                        requires_approval: Some(false),
                        permission_scope: Some("workspace.read".to_string()),
                        host_mediated: Some(true),
                        permission_profile: Some("capability_registry".to_string()),
                        approval_mode: Some("none".to_string()),
                        decision_source: Some("capability_registry".to_string()),
                    }),
                    skill_id: None,
                    skill_source_id: None,
                    composed_capability_refs: None,
                    composed_capability_kinds: None,
                    failure_layer: None,
                }),
            }],
            provider_call_records: vec![ProviderCallCacheRecord {
                request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                input_tokens: Some(12),
                cache_hit_input_tokens: Some(5),
                cache_hit_source: None,
                cache_miss_input_tokens: Some(7),
                reasoning_tokens: Some(3),
                output_tokens: Some(34),
                total_tokens: Some(46),
                first_token_latency_ms: Some(180),
                turn_duration_ms: Some(920),
                latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                prefix_mutation_reasons: Vec::new(),
            }],
            hook_trace_records: vec![HookTraceRecord {
                hook_name: "audit.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::ModelCallStart,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Observe,
                structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                    summary: "hook observed lifecycle boundary without mutation".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("prompt-prefix".to_string()),
                persistence_evidence_ref: None,
                summary: "observe hook summary".to_string(),
            }],
            provider_requested_name: Some("ppx".to_string()),
            provider_name: Some("ppx".to_string()),
            provider_protocol: Some("openai".to_string()),
            provider_model: Some("gpt-5.4".to_string()),
            provider_source: Some("provider_decision".to_string()),
            provider_mode: Some("live".to_string()),
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("测试 trace 持久化".to_string()),
            fallback_reason: None,
            error: None,
            input_tokens: Some(12),
            cache_hit_input_tokens: Some(5),
            reasoning_tokens: Some(3),
            output_tokens: Some(34),
            total_tokens: Some(46),
            first_token_latency_ms: Some(180),
            turn_duration_ms: Some(920),
            updated_at: 0,
        },
    );

    let expected_snapshot = store.snapshot(Some("trace-persisted"), &[]);
    let expected_trace = serde_json::to_value(&expected_snapshot.turn_trace_history[0])
        .expect("expected trace should serialize");

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("trace-persisted"), &[]);

    assert_eq!(snapshot.turn_trace_history.len(), 1);
    assert_eq!(snapshot.turn_trace_history[0].turn_id, "turn-1");
    assert_eq!(
        snapshot.turn_trace_history[0].session_id.as_deref(),
        Some("trace-persisted")
    );
    assert_eq!(
        snapshot.turn_trace_history[0].event_id.as_deref(),
        Some("turn-1:4")
    );
    assert_eq!(
        snapshot.turn_trace_history[0].event_type.as_deref(),
        Some("turn.completed")
    );
    assert_eq!(
        snapshot.turn_trace_history[0].event_version.as_deref(),
        Some("turn-event-v1")
    );
    assert_eq!(snapshot.turn_trace_history[0].sequence, Some(4));
    assert_eq!(snapshot.turn_trace_history[0].emitted_at_ms, Some(4242));
    assert_eq!(
        snapshot.turn_trace_history[0].provider_model.as_deref(),
        Some("gpt-5.4")
    );
    assert_eq!(
        snapshot.turn_trace_history[0].provider_call_records.len(),
        1
    );
    assert_eq!(
        snapshot.turn_trace_history[0].provider_call_records[0].cache_miss_input_tokens,
        Some(7)
    );
    assert_eq!(
        snapshot.turn_trace_history[0].tool_activities[0]
            .capability_invocation
            .as_ref()
            .and_then(|record| record.source_id.as_deref()),
        Some("mcp-local")
    );
    assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 1);
    assert_eq!(
        snapshot.turn_trace_history[0].hook_trace_records[0].hook_name,
        "audit.observe"
    );
    assert_eq!(
        snapshot.turn_trace_history[0].trace_timeline[0].kind,
        "return_result"
    );
    assert_eq!(
        serde_json::to_value(&snapshot.turn_trace_history[0])
            .expect("reloaded trace should serialize"),
        expected_trace
    );

    let _ = fs::remove_file(path);
}

#[test]
fn file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.record_turn_trace(
        Some("trace-multi-hook"),
        TurnTraceRecord {
            turn_id: "turn-multi-hook".to_string(),
            session_id: Some("trace-multi-hook".to_string()),
            event_id: Some("turn-multi-hook:11".to_string()),
            event_type: Some("turn.completed".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(11),
            emitted_at_ms: Some(1111),
            title: "multi boundary hook roundtrip".to_string(),
            phase: "completed".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-return".to_string(),
                label: "Return result".to_string(),
                state: "completed".to_string(),
            }],
            trace_timeline: Vec::new(),
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: vec![
                HookTraceRecord {
                    hook_name: "audit.observe".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Observe,
                    hook_point: crate::agent::hooks::TurnHookPoint::ModelCallStart,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Observe,
                    structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                        summary: "model boundary observed".to_string(),
                    },
                    blocked: false,
                    elapsed_ms: 2,
                    input_summary: Some("model".to_string()),
                    persistence_evidence_ref: None,
                    summary: "model hook summary".to_string(),
                },
                HookTraceRecord {
                    hook_name: "guard.tool".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Guard,
                    hook_point: crate::agent::hooks::TurnHookPoint::ToolCallEnd,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Allow,
                    structured_result: crate::agent::hooks::HookStructuredResult::Allow {
                        summary: "tool boundary allowed".to_string(),
                    },
                    blocked: false,
                    elapsed_ms: 4,
                    input_summary: Some("tool".to_string()),
                    persistence_evidence_ref: None,
                    summary: "tool hook summary".to_string(),
                },
            ],
            provider_requested_name: Some("ppx".to_string()),
            provider_name: Some("ppx".to_string()),
            provider_protocol: Some("openai".to_string()),
            provider_model: Some("gpt-5.4".to_string()),
            provider_source: Some("provider_decision".to_string()),
            provider_mode: Some("live".to_string()),
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("多边界 hook 持久化".to_string()),
            fallback_reason: None,
            error: None,
            input_tokens: Some(12),
            cache_hit_input_tokens: Some(5),
            reasoning_tokens: Some(3),
            output_tokens: Some(34),
            total_tokens: Some(46),
            first_token_latency_ms: Some(180),
            turn_duration_ms: Some(920),
            updated_at: 0,
        },
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("trace-multi-hook"), &[]);

    assert_eq!(snapshot.turn_trace_history.len(), 1);
    assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 2);
    assert_eq!(
        snapshot.turn_trace_history[0].hook_trace_records[0].hook_name,
        "audit.observe"
    );
    assert_eq!(
        snapshot.turn_trace_history[0].hook_trace_records[1].hook_name,
        "guard.tool"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn file_backend_roundtrip_restores_terminal_event_annotation_after_trace_update() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.record_turn_trace(
        Some("trace-annotated"),
        TurnTraceRecord {
            turn_id: "turn-annotated".to_string(),
            session_id: Some("trace-annotated".to_string()),
            event_id: None,
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            title: "等待 terminal event".to_string(),
            phase: "completed".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-return".to_string(),
                label: "Return result".to_string(),
                state: "completed".to_string(),
            }],
            trace_timeline: vec![TraceTimelineEntry {
                id: "return-1".to_string(),
                kind: "return".to_string(),
                label: "RETURN RESULT".to_string(),
                state: "completed".to_string(),
                sequence: 1,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                build_context_observation_ref: None,
                tool_activities: Vec::new(),
                text: Some("ok".to_string()),
                reasoning_content: None,
                fallback_reason: None,
                error: None,
                input_tokens: Some(8),
                cache_hit_input_tokens: Some(3),
                reasoning_tokens: Some(1),
                output_tokens: Some(13),
                total_tokens: Some(21),
                first_token_latency_ms: Some(90),
                turn_duration_ms: Some(420),
            }],
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: vec![HookTraceRecord {
                hook_name: "guard.input".to_string(),
                hook_class: crate::agent::hooks::HookClass::Guard,
                hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildStart,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Allow,
                structured_result: crate::agent::hooks::HookStructuredResult::Allow {
                    summary: "guard allowed runtime to continue".to_string(),
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("context-window".to_string()),
                persistence_evidence_ref: None,
                summary: "guard hook summary".to_string(),
            }],
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("等待回写".to_string()),
            fallback_reason: None,
            error: None,
            input_tokens: Some(8),
            cache_hit_input_tokens: Some(3),
            reasoning_tokens: Some(1),
            output_tokens: Some(13),
            total_tokens: Some(21),
            first_token_latency_ms: Some(90),
            turn_duration_ms: Some(420),
            updated_at: 0,
        },
    );

    let annotation_snapshot = store
        .annotate_turn_trace_terminal_event(
            Some("trace-annotated"),
            "turn-annotated",
            Some("turn-annotated:7".to_string()),
            Some("turn.completed".to_string()),
            Some("turn-event-v1".to_string()),
            Some(7),
            Some(7007),
        )
        .expect("annotation should update trace");
    assert_eq!(
        annotation_snapshot.turn_trace_history[0]
            .event_id
            .as_deref(),
        Some("turn-annotated:7")
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("trace-annotated"), &[]);

    assert_eq!(snapshot.turn_trace_history.len(), 1);
    assert_eq!(
        snapshot.turn_trace_history[0].event_id.as_deref(),
        Some("turn-annotated:7")
    );
    assert_eq!(
        snapshot.turn_trace_history[0].event_type.as_deref(),
        Some("turn.completed")
    );
    assert_eq!(
        snapshot.turn_trace_history[0].event_version.as_deref(),
        Some("turn-event-v1")
    );
    assert_eq!(snapshot.turn_trace_history[0].sequence, Some(7));
    assert_eq!(snapshot.turn_trace_history[0].emitted_at_ms, Some(7007));
    assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 1);
    assert_eq!(
        snapshot.turn_trace_history[0].hook_trace_records[0].hook_name,
        "guard.input"
    );
    assert_eq!(
        snapshot.turn_trace_history[0].trace_timeline[0].kind,
        "return_result"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn append_turn_trace_hook_records_updates_existing_trace_and_roundtrips() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.record_turn_trace(
        Some("trace-append-hook"),
        TurnTraceRecord {
            turn_id: "turn-append-hook".to_string(),
            session_id: Some("trace-append-hook".to_string()),
            event_id: Some("turn-append-hook:3".to_string()),
            event_type: Some("turn.completed".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(3),
            emitted_at_ms: Some(3003),
            title: "等待 graph decision evidence".to_string(),
            phase: "completed".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-return".to_string(),
                label: "Return result".to_string(),
                state: "completed".to_string(),
            }],
            trace_timeline: Vec::new(),
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: vec![HookTraceRecord {
                hook_name: "planner.preflight.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::PlannerTurnPreflight,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Observe,
                structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                    summary: "planner preflight summary".to_string(),
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("message=hello".to_string()),
                persistence_evidence_ref: None,
                summary: "planner preflight summary".to_string(),
            }],
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("append hook trace".to_string()),
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            updated_at: 0,
        },
    );

    let appended = store
        .append_turn_trace_hook_records(
            Some("trace-append-hook"),
            "turn-append-hook",
            vec![HookTraceRecord {
                hook_name: "planner.graph_decision.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::PlannerGraphDecision,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Observe,
                structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                    summary: "planner graph decision summary".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("run_phase=ready".to_string()),
                persistence_evidence_ref: None,
                summary: "planner graph decision summary".to_string(),
            }],
        )
        .expect("append should update existing trace");

    assert_eq!(appended.turn_trace_history.len(), 1);
    assert_eq!(appended.turn_trace_history[0].hook_trace_records.len(), 2);
    assert_eq!(
        appended.turn_trace_history[0].hook_trace_records[1].hook_name,
        "planner.graph_decision.observe"
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("trace-append-hook"), &[]);
    assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 2);
    assert_eq!(
        snapshot.turn_trace_history[0].hook_trace_records[1].hook_point,
        crate::agent::hooks::TurnHookPoint::PlannerGraphDecision
    );

    let _ = fs::remove_file(path);
}

#[test]
fn persisted_mcp_source_snapshots_roundtrip_through_store() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.persist_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
        source: crate::agent::capability_bridge::CapabilitySourceView {
            source_id: "mcp-local".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            display_name: "Local MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "mcp://local".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 7,
            last_ingress_observation: Some(
                crate::agent::capability_bridge::SourceIngressObservation {
                    boundary: "control_plane.apply_mcp_source_snapshot".to_string(),
                    summary:
                        "mcp source ingress registered `mcp-local` with 1 capability candidates"
                            .to_string(),
                    candidate_ids: vec!["mcp:tool:workspace-search".to_string()],
                    observed_at_ms: 77,
                },
            ),
        },
        capabilities: vec![crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:workspace-search".to_string(),
            source_id: "mcp-local".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "workspace.search".to_string(),
            description: "Search workspace files".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        }],
    });

    let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshots = reloaded.list_persisted_mcp_source_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].source.source_id, "mcp-local");
    assert_eq!(
        snapshots[0]
            .source
            .last_ingress_observation
            .as_ref()
            .expect("ingress observation should persist")
            .candidate_ids,
        vec!["mcp:tool:workspace-search".to_string()]
    );

    let _ = fs::remove_file(path);
}

#[test]
fn persisted_skill_source_snapshots_roundtrip_through_store() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    store.persist_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
        source: crate::agent::capability_bridge::SkillSourceView {
            source_id: "host-skills".to_string(),
            source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
            display_name: "Host Skills".to_string(),
            availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
            transport_kind: "host".to_string(),
            server_identity: "skills://host".to_string(),
            updated_at_ms: 9,
            last_ingress_observation: Some(
                crate::agent::capability_bridge::SourceIngressObservation {
                    boundary: "control_plane.apply_skill_source_snapshot".to_string(),
                    summary:
                        "skill source ingress registered `host-skills` with 1 skill candidates"
                            .to_string(),
                    candidate_ids: vec!["skill:search".to_string()],
                    observed_at_ms: 99,
                },
            ),
        },
        skills: vec![crate::agent::capability_bridge::SkillDescriptor {
            skill_id: "skill:search".to_string(),
            source_id: "host-skills".to_string(),
            source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
            label: "search".to_string(),
            description: "Search workspace".to_string(),
            input_schema_summary: "{}".to_string(),
            safety_class: "".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["host".to_string()],
            requires_approval: false,
            host_mediated: false,
            permission_scope: "".to_string(),
            composed_capability_refs: vec!["mcp:tool:workspace-search".to_string()],
            composed_capability_kinds: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
            executable_in_v1: true,
        }],
    });

    let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshots = reloaded.list_persisted_skill_source_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].source.source_id, "host-skills");
    assert_eq!(
        snapshots[0]
            .source
            .last_ingress_observation
            .as_ref()
            .expect("ingress observation should persist")
            .candidate_ids,
        vec!["skill:search".to_string()]
    );

    let _ = fs::remove_file(path);
}

#[test]
fn file_backend_roundtrip_restores_checkpoint_persist_evidence() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.record_turn_trace(
        Some("trace-checkpoint-boundary"),
        TurnTraceRecord {
            turn_id: "turn-checkpoint-boundary".to_string(),
            session_id: Some("trace-checkpoint-boundary".to_string()),
            event_id: Some("turn-checkpoint-boundary:9".to_string()),
            event_type: Some("turn.completed".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(9),
            emitted_at_ms: Some(9009),
            title: "checkpoint boundary roundtrip".to_string(),
            phase: "completed".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-return".to_string(),
                label: "Return result".to_string(),
                state: "completed".to_string(),
            }],
            trace_timeline: vec![
                TraceTimelineEntry {
                    id: "return-1".to_string(),
                    kind: "return".to_string(),
                    label: "RETURN RESULT".to_string(),
                    state: "completed".to_string(),
                    sequence: 1,
                    provider_requested_name: Some("openai".to_string()),
                    provider_name: Some("openai".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5".to_string()),
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    build_context_observation_ref: None,
                    tool_activities: Vec::new(),
                    text: Some("final answer".to_string()),
                    reasoning_content: None,
                    fallback_reason: None,
                    error: None,
                    input_tokens: Some(11),
                    cache_hit_input_tokens: Some(4),
                    reasoning_tokens: Some(2),
                    output_tokens: Some(19),
                    total_tokens: Some(30),
                    first_token_latency_ms: Some(120),
                    turn_duration_ms: Some(860),
                },
                TraceTimelineEntry {
                    id: "checkpoint-2".to_string(),
                    kind: "checkpoint_persist".to_string(),
                    label: "PERSIST CHECKPOINT".to_string(),
                    state: "completed".to_string(),
                    sequence: 2,
                    provider_requested_name: Some("openai".to_string()),
                    provider_name: Some("openai".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5".to_string()),
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    build_context_observation_ref: None,
                    tool_activities: Vec::new(),
                    text: None,
                    reasoning_content: None,
                    fallback_reason: None,
                    error: None,
                    input_tokens: Some(11),
                    cache_hit_input_tokens: Some(4),
                    reasoning_tokens: Some(2),
                    output_tokens: Some(19),
                    total_tokens: Some(30),
                    first_token_latency_ms: Some(120),
                    turn_duration_ms: Some(860),
                },
            ],
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: Vec::new(),
            provider_requested_name: Some("openai".to_string()),
            provider_name: Some("openai".to_string()),
            provider_protocol: Some("openai".to_string()),
            provider_model: Some("gpt-5".to_string()),
            provider_source: Some("provider_decision".to_string()),
            provider_mode: Some("live".to_string()),
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("checkpoint boundary summary".to_string()),
            fallback_reason: None,
            error: None,
            input_tokens: Some(11),
            cache_hit_input_tokens: Some(4),
            reasoning_tokens: Some(2),
            output_tokens: Some(19),
            total_tokens: Some(30),
            first_token_latency_ms: Some(120),
            turn_duration_ms: Some(860),
            updated_at: 0,
        },
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("trace-checkpoint-boundary"), &[]);

    assert_eq!(snapshot.turn_trace_history.len(), 1);
    assert_eq!(
        snapshot.turn_trace_history[0].event_type.as_deref(),
        Some("turn.completed")
    );
    assert_eq!(snapshot.turn_trace_history[0].phase, "completed");
    assert_eq!(
        snapshot.turn_trace_history[0]
            .trace_timeline
            .last()
            .map(|entry| entry.kind.as_str()),
        Some("checkpoint_persist")
    );
    assert_eq!(
        snapshot.turn_trace_history[0]
            .trace_timeline
            .last()
            .map(|entry| entry.label.as_str()),
        Some("PERSIST CHECKPOINT")
    );
    assert_eq!(
        snapshot.turn_trace_history[0]
            .trace_timeline
            .last()
            .and_then(|entry| entry.turn_duration_ms),
        Some(860)
    );

    let _ = fs::remove_file(path);
}

#[test]
fn file_backend_roundtrip_restores_failed_terminal_envelope_and_existing_evidence() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.record_turn_trace(
        Some("trace-failed-terminal"),
        TurnTraceRecord {
            turn_id: "turn-failed-terminal".to_string(),
            session_id: Some("trace-failed-terminal".to_string()),
            event_id: Some("turn-failed-terminal:8".to_string()),
            event_type: Some("turn.failed".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(8),
            emitted_at_ms: Some(8008),
            title: "failed terminal roundtrip".to_string(),
            phase: "failed".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-return".to_string(),
                label: "Return result".to_string(),
                state: "failed".to_string(),
            }],
            trace_timeline: vec![TraceTimelineEntry {
                id: "return-1".to_string(),
                kind: "return".to_string(),
                label: "RETURN RESULT".to_string(),
                state: "failed".to_string(),
                sequence: 1,
                provider_requested_name: Some("openai".to_string()),
                provider_name: Some("openai".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                build_context_observation_ref: None,
                tool_activities: Vec::new(),
                text: Some("failed terminal".to_string()),
                reasoning_content: None,
                fallback_reason: None,
                error: Some("hook blocked finalize".to_string()),
                input_tokens: Some(11),
                cache_hit_input_tokens: Some(4),
                reasoning_tokens: Some(2),
                output_tokens: Some(0),
                total_tokens: Some(13),
                first_token_latency_ms: Some(120),
                turn_duration_ms: Some(640),
            }],
            tool_activities: vec![TurnToolActivity {
                id: "tool-1".to_string(),
                name: "workspace_list_files".to_string(),
                canonical_tool_name: Some("List".to_string()),
                display_name_zh: Some("列表".to_string()),
                status: "completed".to_string(),
                description: "tool completed before finalize failed".to_string(),
                arguments_text: Some("{\"path\":\".\"}".to_string()),
                result_text: Some("ok".to_string()),
                duration_seconds: Some(0.2),
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: None,
            }],
            provider_call_records: vec![ProviderCallCacheRecord {
                request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                input_tokens: Some(11),
                cache_hit_input_tokens: Some(4),
                cache_hit_source: None,
                cache_miss_input_tokens: Some(7),
                reasoning_tokens: Some(2),
                output_tokens: Some(0),
                total_tokens: Some(13),
                first_token_latency_ms: Some(120),
                turn_duration_ms: Some(640),
                latency_kind: crate::agent::telemetry::ProviderLatencyKind::BufferedResponse,
                prefix_mutation_reasons: vec![
                    crate::agent::provider::PrefixMutationReason::SessionSummaryChanged,
                ],
            }],
            hook_trace_records: vec![HookTraceRecord {
                hook_name: "observe.sync-finalize-failturn".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Deny,
                structured_result: crate::agent::hooks::HookStructuredResult::Deny(
                    crate::agent::hooks::HookDenyDecision {
                        reason_code: "hook_blocked_finalize".to_string(),
                        message: "hook blocked finalize".to_string(),
                    },
                ),
                blocked: true,
                elapsed_ms: 5,
                input_summary: Some("failed".to_string()),
                persistence_evidence_ref: Some("trace://turn-failed-terminal/finalize".to_string()),
                summary: "finalize hook blocked terminal".to_string(),
            }],
            provider_requested_name: Some("openai".to_string()),
            provider_name: Some("openai".to_string()),
            provider_protocol: Some("openai".to_string()),
            provider_model: Some("gpt-5".to_string()),
            provider_source: Some("provider_decision".to_string()),
            provider_mode: Some("live".to_string()),
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("failed summary".to_string()),
            fallback_reason: None,
            error: Some("hook blocked finalize".to_string()),
            input_tokens: Some(11),
            cache_hit_input_tokens: Some(4),
            reasoning_tokens: Some(2),
            output_tokens: Some(0),
            total_tokens: Some(13),
            first_token_latency_ms: Some(120),
            turn_duration_ms: Some(640),
            updated_at: 0,
        },
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("trace-failed-terminal"), &[]);

    assert_eq!(snapshot.turn_trace_history.len(), 1);
    let trace = &snapshot.turn_trace_history[0];
    assert_eq!(trace.phase, "failed");
    assert_eq!(trace.event_id.as_deref(), Some("turn-failed-terminal:8"));
    assert_eq!(trace.event_type.as_deref(), Some("turn.failed"));
    assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
    assert_eq!(trace.sequence, Some(8));
    assert_eq!(trace.emitted_at_ms, Some(8008));
    assert_eq!(trace.error.as_deref(), Some("hook blocked finalize"));
    assert_eq!(trace.provider_call_records.len(), 1);
    assert_eq!(
        trace.provider_call_records[0].request_kind,
        crate::agent::telemetry::ProviderRequestKind::InitialRequest
    );
    assert_eq!(trace.tool_activities.len(), 1);
    assert_eq!(trace.tool_activities[0].name, "workspace_list_files");
    assert_eq!(trace.hook_trace_records.len(), 1);
    assert!(trace.hook_trace_records[0].blocked);
    assert_eq!(
        trace.hook_trace_records[0]
            .persistence_evidence_ref
            .as_deref(),
        Some("trace://turn-failed-terminal/finalize")
    );

    let _ = fs::remove_file(path);
}

#[test]
fn file_backend_roundtrip_persists_failed_turn_into_visible_history() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.append_failed_turn(
        Some("failed-visible-history"),
        "请继续排查 session 跳转问题",
        "hook blocked finalize",
        TurnTraceRecord {
            turn_id: "turn-failed-visible-history".to_string(),
            session_id: Some("failed-visible-history".to_string()),
            event_id: None,
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            title: "failed visible history".to_string(),
            phase: "failed".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-return".to_string(),
                label: "Return result".to_string(),
                state: "failed".to_string(),
            }],
            trace_timeline: vec![TraceTimelineEntry {
                id: "return-1".to_string(),
                kind: "return".to_string(),
                label: "RETURN RESULT".to_string(),
                state: "failed".to_string(),
                sequence: 1,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                build_context_observation_ref: None,
                tool_activities: Vec::new(),
                text: Some("hook blocked finalize".to_string()),
                reasoning_content: None,
                fallback_reason: None,
                error: Some("hook blocked finalize".to_string()),
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            }],
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: Vec::new(),
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("hook blocked finalize".to_string()),
            fallback_reason: None,
            error: Some("hook blocked finalize".to_string()),
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            updated_at: 0,
        },
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("failed-visible-history"), &[]);

    assert_eq!(snapshot.turn_count, 1);
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.history[0].role, "user");
    assert_eq!(snapshot.history[0].content, "请继续排查 session 跳转问题");
    assert_eq!(snapshot.history[1].role, "assistant");
    assert_eq!(snapshot.history[1].content, "hook blocked finalize");
    assert_eq!(snapshot.turn_trace_history.len(), 1);
    assert_eq!(
        snapshot.turn_trace_history[0].error.as_deref(),
        Some("hook blocked finalize")
    );

    let _ = fs::remove_file(path);
}

#[test]
fn file_backend_roundtrip_restores_cancelled_terminal_envelope_and_existing_evidence() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.record_turn_trace(
        Some("trace-cancelled-terminal"),
        TurnTraceRecord {
            turn_id: "turn-cancelled-terminal".to_string(),
            session_id: Some("trace-cancelled-terminal".to_string()),
            event_id: Some("turn-cancelled-terminal:6".to_string()),
            event_type: Some("turn.cancelled".to_string()),
            event_version: Some("turn-event-v1".to_string()),
            sequence: Some(6),
            emitted_at_ms: Some(6006),
            title: "cancelled terminal roundtrip".to_string(),
            phase: "cancelled".to_string(),
            trace_steps: vec![TurnTraceStep {
                id: "step-call-tool".to_string(),
                label: "Call tool".to_string(),
                state: "completed".to_string(),
            }],
            trace_timeline: vec![TraceTimelineEntry {
                id: "tool-1".to_string(),
                kind: "tool".to_string(),
                label: "CALL TOOL #1".to_string(),
                state: "completed".to_string(),
                sequence: 1,
                provider_requested_name: Some("openai".to_string()),
                provider_name: Some("openai".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                build_context_observation_ref: None,
                tool_activities: vec![TurnToolActivity {
                    id: "tool-1".to_string(),
                    name: "workspace_list_files".to_string(),
                    canonical_tool_name: Some("List".to_string()),
                    display_name_zh: Some("列表".to_string()),
                    status: "completed".to_string(),
                    description: "tool completed before cancel".to_string(),
                    arguments_text: Some("{\"path\":\".\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.2),
                    parent_activity_id: None,
                    artifacts: None,
                    error: None,
                    capability_invocation: None,
                }],
                text: None,
                reasoning_content: None,
                fallback_reason: None,
                error: None,
                input_tokens: Some(9),
                cache_hit_input_tokens: Some(3),
                reasoning_tokens: Some(1),
                output_tokens: Some(0),
                total_tokens: Some(10),
                first_token_latency_ms: Some(90),
                turn_duration_ms: Some(510),
            }],
            tool_activities: vec![TurnToolActivity {
                id: "tool-1".to_string(),
                name: "workspace_list_files".to_string(),
                canonical_tool_name: Some("List".to_string()),
                display_name_zh: Some("列表".to_string()),
                status: "completed".to_string(),
                description: "tool completed before cancel".to_string(),
                arguments_text: Some("{\"path\":\".\"}".to_string()),
                result_text: Some("ok".to_string()),
                duration_seconds: Some(0.2),
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: None,
            }],
            provider_call_records: vec![ProviderCallCacheRecord {
                request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                input_tokens: Some(9),
                cache_hit_input_tokens: Some(3),
                cache_hit_source: None,
                cache_miss_input_tokens: Some(6),
                reasoning_tokens: Some(1),
                output_tokens: Some(0),
                total_tokens: Some(10),
                first_token_latency_ms: Some(90),
                turn_duration_ms: Some(510),
                latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                prefix_mutation_reasons: vec![
                    crate::agent::provider::PrefixMutationReason::HistoryBoundaryShifted,
                ],
            }],
            hook_trace_records: vec![HookTraceRecord {
                hook_name: "observe.cancelled-finalize".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Observe,
                structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                    summary: "cancelled terminal observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 3,
                input_summary: Some("cancelled".to_string()),
                persistence_evidence_ref: Some(
                    "trace://turn-cancelled-terminal/finalize".to_string(),
                ),
                summary: "cancelled hook summary".to_string(),
            }],
            provider_requested_name: Some("openai".to_string()),
            provider_name: Some("openai".to_string()),
            provider_protocol: Some("openai".to_string()),
            provider_model: Some("gpt-5".to_string()),
            provider_source: Some("provider_decision".to_string()),
            provider_mode: Some("live".to_string()),
            build_context_observation: None,
            build_context_observation_ref: None,
            session_summary: Some("cancelled summary".to_string()),
            fallback_reason: Some("stopped_by_user".to_string()),
            error: Some("stopped_by_user".to_string()),
            input_tokens: Some(9),
            cache_hit_input_tokens: Some(3),
            reasoning_tokens: Some(1),
            output_tokens: Some(0),
            total_tokens: Some(10),
            first_token_latency_ms: Some(90),
            turn_duration_ms: Some(510),
            updated_at: 0,
        },
    );

    let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = reloaded.snapshot(Some("trace-cancelled-terminal"), &[]);

    assert_eq!(snapshot.turn_trace_history.len(), 1);
    let trace = &snapshot.turn_trace_history[0];
    assert_eq!(trace.phase, "cancelled");
    assert_eq!(trace.event_id.as_deref(), Some("turn-cancelled-terminal:6"));
    assert_eq!(trace.event_type.as_deref(), Some("turn.cancelled"));
    assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
    assert_eq!(trace.sequence, Some(6));
    assert_eq!(trace.emitted_at_ms, Some(6006));
    assert_eq!(trace.error.as_deref(), Some("stopped_by_user"));
    assert_eq!(trace.fallback_reason.as_deref(), Some("stopped_by_user"));
    assert_eq!(trace.provider_call_records.len(), 1);
    assert_eq!(
        trace.provider_call_records[0].request_kind,
        crate::agent::telemetry::ProviderRequestKind::InitialRequest
    );
    assert_eq!(trace.tool_activities.len(), 1);
    assert_eq!(trace.tool_activities[0].status, "completed");
    assert_eq!(trace.hook_trace_records.len(), 1);
    assert_eq!(
        trace.hook_trace_records[0]
            .persistence_evidence_ref
            .as_deref(),
        Some("trace://turn-cancelled-terminal/finalize")
    );

    let _ = fs::remove_file(path);
}

#[test]
fn history_checkout_can_rehydrate_older_node_and_degrade_workspace_restore() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("history-session"),
        "第一问",
        "第一答",
        None,
        Vec::new(),
    );
    store.append_turn(
        Some("history-session"),
        "第二问",
        "第二答",
        None,
        Vec::new(),
    );

    let (nodes, branches, _) = store.load_history_graph(Some("history-session"));
    assert_eq!(nodes.len(), 3);
    assert_eq!(branches.len(), 1);

    let snapshot = store
        .checkout_history_node(
            Some("history-session"),
            &nodes[1].node_id,
            HistoryCheckoutMode::TranscriptAndWorkspace,
            None,
        )
        .expect("history checkout should succeed");

    assert_eq!(
        snapshot.resolved_node_id.as_deref(),
        Some(nodes[1].node_id.as_str())
    );
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.history[0].content, "第一问");
    assert_eq!(snapshot.history[1].content, "第一答");
    assert_eq!(
        snapshot.latest_node_id.as_deref(),
        Some(nodes[1].node_id.as_str())
    );
    // checkout_history_node truncates descendants and makes the target
    // node the branch head, so mode is Live (d6e1fbf semantics).
    assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
    assert_eq!(
        snapshot.history_cursor.checkout_status,
        HistoryCheckoutStatus::DegradedToTranscriptOnly
    );
}

#[test]
fn appending_from_historical_node_creates_a_fork_and_preserves_main_branch_head() {
    let mut store = SessionStore::memory_only();
    store.append_turn(Some("fork-session"), "第一问", "第一答", None, Vec::new());
    store.append_turn(Some("fork-session"), "第二问", "第二答", None, Vec::new());

    let (nodes_before, branches_before, _) = store.load_history_graph(Some("fork-session"));
    let main_head_before = branches_before[0].head_node_id.clone();
    let fork_base = nodes_before[0].node_id.clone();

    store
        .checkout_history_node(
            Some("fork-session"),
            fork_base.as_str(),
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("checkout should succeed");

    let fork_snapshot = store.append_turn(
        Some("fork-session"),
        "历史节点上继续追问",
        "分叉后的回答",
        None,
        Vec::new(),
    );

    let (nodes_after, branches_after, cursor_after) =
        store.load_history_graph(Some("fork-session"));
    assert_eq!(nodes_after.len(), 4);
    assert_eq!(branches_after.len(), 2);
    assert_eq!(cursor_after.mode, HistoryCursorMode::Live);

    let main_branch = branches_after
        .iter()
        .find(|branch| branch.branch_id == DEFAULT_HISTORY_BRANCH_ID)
        .expect("main branch should exist");
    assert_eq!(main_branch.head_node_id, main_head_before);

    let fork_branch = branches_after
        .iter()
        .find(|branch| branch.branch_id != DEFAULT_HISTORY_BRANCH_ID)
        .expect("fork branch should be created");
    assert_eq!(
        fork_branch.base_node_id.as_deref(),
        Some(fork_base.as_str())
    );
    assert_eq!(
        fork_branch.forked_from_node_id.as_deref(),
        Some(fork_base.as_str())
    );
    assert_eq!(
        fork_snapshot.history_cursor.active_branch_id.as_deref(),
        Some(fork_branch.branch_id.as_str())
    );

    let fork_head = nodes_after
        .iter()
        .find(|node| node.branch_id == fork_branch.branch_id)
        .expect("fork branch head node should exist");
    assert_eq!(
        fork_head.parent_node_id.as_deref(),
        Some(fork_base.as_str())
    );
}

#[test]
fn restore_and_switch_history_branch_move_cursor_between_branch_heads() {
    let mut store = SessionStore::memory_only();
    store.append_turn(Some("switch-session"), "第一问", "第一答", None, Vec::new());
    store.append_turn(Some("switch-session"), "第二问", "第二答", None, Vec::new());

    let (nodes_before, branches_before, _) = store.load_history_graph(Some("switch-session"));
    let first_node_id = nodes_before[0].node_id.clone();
    // The graph includes a checkpoint root node before the two turns, so
    // the main branch head is taken from the branch record (the last turn
    // node), not nodes_before[1].
    let second_node_id = branches_before[0]
        .head_node_id
        .clone()
        .expect("main branch head node should exist");

    store
        .fork_from_history_node(Some("switch-session"), first_node_id.as_str(), None)
        .expect("fork should succeed");
    store.append_turn(
        Some("switch-session"),
        "在分叉上继续",
        "分叉回答",
        None,
        Vec::new(),
    );

    let (_, branches_after_fork, _) = store.load_history_graph(Some("switch-session"));
    let fork_branch_id = branches_after_fork
        .iter()
        .find(|branch| branch.branch_id != DEFAULT_HISTORY_BRANCH_ID)
        .map(|branch| branch.branch_id.clone())
        .expect("fork branch should exist");

    let switched = store
        .switch_history_branch(Some("switch-session"), DEFAULT_HISTORY_BRANCH_ID, None)
        .expect("switch to main branch should succeed");
    assert_eq!(
        switched.resolved_node_id.as_deref(),
        Some(second_node_id.as_str())
    );
    assert_eq!(switched.history_cursor.mode, HistoryCursorMode::Live);

    let restored = store
        .restore_branch_head(Some("switch-session"), Some(fork_branch_id.as_str()), None)
        .expect("restore fork branch head should succeed");
    assert_eq!(
        restored.history_cursor.active_branch_id.as_deref(),
        Some(fork_branch_id.as_str())
    );
    assert_eq!(restored.history_cursor.mode, HistoryCursorMode::Live);
    assert_ne!(
        restored.resolved_node_id.as_deref(),
        Some(second_node_id.as_str())
    );
}

#[test]
fn session_state_for_backend_strips_node_traces_and_writes_refs() {
    // PA-088：WriteSeparate + Authoritative 时，持久化副本剥离节点 trace 并生成 refs；
    // 内存副本（原 session）不受影响。
    let session = SessionState {
        conversation_id: "s1".to_string(),
        title: "t".to_string(),
        summary: "s".to_string(),
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: vec![
            TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                updated_at: 100,
                ..TurnTraceRecord::default()
            },
            TurnTraceRecord {
                turn_id: "turn-2".to_string(),
                updated_at: 200,
                ..TurnTraceRecord::default()
            },
        ],
        trace_migration_state: TraceMigrationState::TraceTableAuthoritative,
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 2,
        last_referenced_file: None,
        updated_at_ms: 300,
        history_nodes: vec![HistoryNode {
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
                turn_id: "turn-2".to_string(),
                updated_at: 200,
                ..TurnTraceRecord::default()
            }],
            turn_id: None,
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            turn_count: 2,
            last_referenced_file: None,
            created_at_ms: 250,
            event_seq_range: None,
        }],
        history_branches: Vec::new(),
        history_cursor: HistoryCursor::default(),
        workspace_id: None,
        title_override: None,
        archived: false,
        event_watermark: 0,
        last_commit_watermark: 0,
    };

    let prepared = session_state_for_backend(&session, SeparateTraceTableMode::WriteSeparate);

    // 持久化副本：顶层 + 节点 trace 被剥离，refs 生成
    assert!(prepared.turn_trace_history.is_empty());
    let top_refs = prepared.turn_trace_refs.expect("top-level refs present");
    assert_eq!(top_refs.len(), 2);
    assert_eq!(top_refs[0].turn_id, "turn-1");
    assert_eq!(top_refs[1].turn_id, "turn-2");
    let node = &prepared.history_nodes[0];
    assert!(node.turn_trace_history.is_empty());
    assert_eq!(node.turn_id.as_deref(), Some("turn-2"));
    let node_refs = node.turn_trace_refs.as_ref().expect("node refs present");
    assert_eq!(node_refs.len(), 1);
    assert_eq!(node_refs[0].turn_id, "turn-2");

    // 内存副本：完整 trace 保留（运行中 checkout 语义不变）
    assert_eq!(session.turn_trace_history.len(), 2);
    assert_eq!(session.history_nodes[0].turn_trace_history.len(), 1);
    assert!(session.turn_trace_refs.is_none());
}

#[test]
fn session_state_for_backend_keeps_legacy_blob_intact_under_write_separate() {
    // PA-088：WriteSeparate + LegacyBlob 存量会话不晋升、不剥离（保持可读写）。
    let session = SessionState {
        conversation_id: "s1".to_string(),
        title: "t".to_string(),
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

fn pa093_sqlite_store(tag: &str) -> (SessionStore, std::path::PathBuf, String) {
    use crate::agent::session::SeparateTraceTableMode;
    use crate::agent::sqlite_session::SqliteSessionBackend;
    let dir = std::env::temp_dir().join(format!(
        "pa093-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
        db_path.clone(),
        SeparateTraceTableMode::WriteSeparate,
    ));
    let session_id = format!("pa093-{tag}");
    (SessionStore::with_backend(backend), dir, session_id)
}

fn pa093_flush(
    store: &mut SessionStore,
    session_id: &str,
    turn_id: &str,
    branch_id: &str,
    events: &[TurnEvent],
) {
    let branch = branch_id.to_string();
    assert!(store.persist_events(session_id, turn_id, &branch, events.to_vec()));
    store.finalize_event_watermark(session_id, turn_id);
}

fn pa093_turn_events(turn_id: &str, user_text: &str, assistant_text: &str) -> Vec<TurnEvent> {
    vec![
        TurnEvent::TurnStart {
            turn_id: turn_id.to_string(),
        },
        TurnEvent::UserMessage {
            turn_id: turn_id.to_string(),
            text: user_text.to_string(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantMessage {
            turn_id: turn_id.to_string(),
            step: 0,
            text: assistant_text.to_string(),
            reasoning_content: None,
            usage: None,
            chunk_missing: None,
        },
        TurnEvent::TurnEnd {
            turn_id: turn_id.to_string(),
            reason: TurnEndReason::Completed,
            turn_duration_ms: None,
        },
    ]
}

fn pa093_first_turn_node_id(store: &SessionStore, session_id: &str) -> String {
    let session = store.sessions.get(session_id).expect("session");
    session
        .history_nodes
        .iter()
        .find(|n| n.kind == HistoryNodeKind::TurnCommitted)
        .expect("turn node")
        .node_id
        .clone()
}

#[test]
fn pa093_commit_finalize_upgrades_node_to_referenced() {
    let (mut store, dir, sid) = pa093_sqlite_store("finalize");
    store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
    let head = {
        let s = store.sessions.get(&sid).expect("session");
        s.history_cursor.branch_head_node_id.clone().expect("head")
    };
    let s = store.sessions.get(&sid).expect("session");
    let node = s
        .history_nodes
        .iter()
        .find(|n| n.node_id == head)
        .expect("node");
    assert!(node.event_seq_range.is_none(), "pre-flush legacy");
    assert_eq!(node.history.len(), 2, "pre-flush keeps snapshot");
    let _ = s;
    pa093_flush(
        &mut store,
        &sid,
        "turn-1",
        "branch-main",
        &pa093_turn_events("turn-1", "first", "reply1"),
    );
    let s = store.sessions.get(&sid).expect("session");
    let node = s
        .history_nodes
        .iter()
        .find(|n| n.node_id == head)
        .expect("node");
    assert_eq!(node.event_seq_range, Some((0, 3)), "range covered");
    assert!(node.history.is_empty(), "snapshot cleared");
    assert!(node.turn_trace_history.is_empty(), "trace cleared");
    assert_eq!(s.event_watermark, 4);
    assert_eq!(s.last_commit_watermark, 4);
    let _ = s;
    store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-2",
        "branch-main",
        &pa093_turn_events("turn-2", "second", "reply2"),
    );
    let s = store.sessions.get(&sid).expect("session");
    let nodes = &s.history_nodes;
    assert_eq!(
        nodes[nodes.len() - 1].event_seq_range,
        Some((4, 7)),
        "contiguous range"
    );
    let _ = s;
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pa093_checkout_referenced_node_folds_events() {
    let (mut store, dir, sid) = pa093_sqlite_store("checkout");
    store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-1",
        "branch-main",
        &pa093_turn_events("turn-1", "first", "reply1"),
    );
    store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-2",
        "branch-main",
        &pa093_turn_events("turn-2", "second", "reply2"),
    );
    let node1 = pa093_first_turn_node_id(&store, &sid);
    let snapshot = store
        .checkout_history_node(
            Some(&sid),
            &node1,
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("checkout");
    assert_eq!(snapshot.history.len(), 2, "folded view has turn 1 only");
    assert!(
        snapshot.history.iter().all(|m| m.content != "reply2"),
        "watermark rollback"
    );
    assert_eq!(
        snapshot.history_cursor.event_watermark, 8,
        "cursor watermark"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pa093_restore_branch_head_folds_referenced_branch() {
    // P1-1：restore_branch_head 对引用化分支头节点的折叠路径
    let (mut store, dir, sid) = pa093_sqlite_store("restore");
    store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-1",
        "branch-main",
        &pa093_turn_events("turn-1", "first", "reply1"),
    );
    let node1 = pa093_first_turn_node_id(&store, &sid);
    let fork_snapshot = store
        .fork_from_history_node(Some(&sid), &node1, None)
        .expect("fork");
    let fork_branch = fork_snapshot
        .history_cursor
        .active_branch_id
        .expect("active branch");
    store.append_turn(Some(&sid), "fork-q", "fork-answer", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "fork-turn",
        &fork_branch,
        &pa093_turn_events("fork-turn", "fork-q", "fork-answer"),
    );
    let restored = store
        .restore_branch_head(Some(&sid), Some(&fork_branch), None)
        .expect("restore");
    assert!(
        restored.history.iter().any(|m| m.content == "fork-answer"),
        "restore folds fork events"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pa093_fork_from_referenced_node_folds_source_view() {
    // P1-1：fork_from_history_node 对引用化源节点的折叠路径
    let (mut store, dir, sid) = pa093_sqlite_store("fork-fold");
    store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-1",
        "branch-main",
        &pa093_turn_events("turn-1", "first", "reply1"),
    );
    let node1 = pa093_first_turn_node_id(&store, &sid);
    let fork_snapshot = store
        .fork_from_history_node(Some(&sid), &node1, None)
        .expect("fork");
    assert_eq!(
        fork_snapshot.history.len(),
        2,
        "fork view folds source node events"
    );
    assert_eq!(fork_snapshot.history[0].content, "first");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pa093_checkout_legacy_node_falls_back_to_snapshot() {
    let (mut store, dir, sid) = pa093_sqlite_store("legacy");
    store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
    store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
    let node1 = pa093_first_turn_node_id(&store, &sid);
    let snapshot = store
        .checkout_history_node(
            Some(&sid),
            &node1,
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("legacy checkout");
    assert_eq!(snapshot.history.len(), 2, "legacy snapshot");
    assert!(snapshot.history.iter().all(|m| m.content != "reply2"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pa093_time_travel_loads_referenced_node_state() {
    let (mut store, dir, sid) = pa093_sqlite_store("travel");
    store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-1",
        "branch-main",
        &pa093_turn_events("turn-1", "first", "reply1"),
    );
    store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-2",
        "branch-main",
        &pa093_turn_events("turn-2", "second", "reply2"),
    );
    let node1 = pa093_first_turn_node_id(&store, &sid);
    let snapshot = store.snapshot_for_session_at(&sid, Some(&node1));
    assert_eq!(snapshot.history.len(), 2, "traveled = node1");
    assert!(snapshot.history.iter().all(|m| m.content != "reply2"));
    let current = store.snapshot_for_session(&sid);
    assert_eq!(current.history.len(), 4, "live untouched");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pa093_fork_visibility_filters_events() {
    let (mut store, dir, sid) = pa093_sqlite_store("fork-vis");
    store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "turn-1",
        "branch-main",
        &pa093_turn_events("turn-1", "first", "reply1"),
    );
    let node1 = pa093_first_turn_node_id(&store, &sid);
    let fork_snapshot = store
        .fork_from_history_node(Some(&sid), &node1, None)
        .expect("fork");
    let fork_branch = fork_snapshot
        .history_cursor
        .active_branch_id
        .expect("active branch");
    assert_ne!(fork_branch, "branch-main");
    store.append_turn(Some(&sid), "fork-q", "fork-answer", None, Vec::new());
    pa093_flush(
        &mut store,
        &sid,
        "fork-turn",
        &fork_branch,
        &pa093_turn_events("fork-turn", "fork-q", "fork-answer"),
    );
    let main_snapshot = store
        .switch_history_branch(Some(&sid), "branch-main", None)
        .expect("switch to main");
    assert!(
        main_snapshot
            .history
            .iter()
            .all(|m| m.content != "fork-answer"),
        "fork invisible after switch"
    );
    let fork_again = store
        .switch_history_branch(Some(&sid), &fork_branch, None)
        .expect("switch back");
    assert!(
        fork_again
            .history
            .iter()
            .any(|m| m.content == "fork-answer"),
        "fork visible again"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pa093_fold_10k_events_stays_under_budget() {
    let mut events = Vec::with_capacity(10_000);
    for i in 0..1_000u32 {
        let tid = format!("t{i}");
        events.push((
            "branch-main".to_string(),
            TurnEvent::TurnStart {
                turn_id: tid.clone(),
            },
        ));
        events.push((
            "branch-main".to_string(),
            TurnEvent::UserMessage {
                turn_id: tid.clone(),
                text: format!("q{i}"),
                attachments: Vec::new(),
            },
        ));
        events.push((
            "branch-main".to_string(),
            TurnEvent::AssistantMessage {
                turn_id: tid.clone(),
                step: 0,
                text: format!("a{i}"),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ));
        events.push((
            "branch-main".to_string(),
            TurnEvent::TurnEnd {
                turn_id: tid.clone(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
            },
        ));
    }
    let with_seq: Vec<(u64, String, TurnEvent)> = events
        .into_iter()
        .enumerate()
        .map(|(seq, (b, e))| (seq as u64, b, e))
        .collect();
    let started = std::time::Instant::now();
    let (history, _) = fold_session_views(&with_seq, "branch-main", &[], &[]);
    let elapsed = started.elapsed();
    assert_eq!(history.len(), 24, "window truncated");
    let budget = if cfg!(debug_assertions) { 2000 } else { 100 };
    assert!(
        elapsed.as_millis() < budget,
        "10k refold took {}ms",
        elapsed.as_millis()
    );
}

/// PA-094：集成验证——真实 turn 的 trace 视图与消息视图同源（同一份事件派生）。
/// fold_session_views 同时折叠 HistoryProjection 与 TraceProjection（+ MetricsProjection
/// 的 ProviderCallCacheRecord 挂载），消息与 trace 来自同一事件流。
#[test]
fn pa094_trace_and_message_views_same_source() {
    
    let events: Vec<(u64, String, TurnEvent)> = vec![
        (
            0,
            "branch-main".to_string(),
            TurnEvent::TurnStart {
                turn_id: "turn-1".into(),
            },
        ),
        (
            1,
            "branch-main".to_string(),
            TurnEvent::UserMessage {
                turn_id: "turn-1".into(),
                text: "hello".into(),
                attachments: Vec::new(),
            },
        ),
        (
            2,
            "branch-main".to_string(),
            TurnEvent::AssistantChunk {
                turn_id: "turn-1".into(),
                step: 0,
                text: "hi".into(),
            },
        ),
        (
            3,
            "branch-main".to_string(),
            TurnEvent::ToolCall {
                turn_id: "turn-1".into(),
                step: 1,
                call_id: "c1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
                started_at_ms: None,
            },
        ),
        (
            4,
            "branch-main".to_string(),
            TurnEvent::ToolResult {
                turn_id: "turn-1".into(),
                step: 1,
                call_id: "c1".into(),
                result: Some("ok".into()),
                error: None,
                status: Some("done".into()),
                duration_ms: Some(500),
                artifacts: None,
                capability_invocation: None,
            },
        ),
        (
            5,
            "branch-main".to_string(),
            TurnEvent::ProviderUsage {
                turn_id: "turn-1".into(),
                step: 0,
                request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                usage: crate::agent::provider::TokenUsage {
                    input_tokens: Some(100),
                    cache_hit_input_tokens: Some(40),
                    cache_hit_source: None,
                    reasoning_tokens: Some(10),
                    output_tokens: Some(50),
                    total_tokens: Some(160),
                },
                cache_hit_input_tokens: Some(40),
                cache_miss_input_tokens: Some(60),
                prefix_mutation_reasons: Vec::new(),
                first_token_latency_ms: Some(88),
                turn_duration_ms: Some(500),
                latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                provider: "deepseek".into(),
                model: "deepseek-chat".into(),
            },
        ),
        (
            6,
            "branch-main".to_string(),
            TurnEvent::AssistantMessage {
                turn_id: "turn-1".into(),
                step: 0,
                text: "hi".into(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ),
        (
            7,
            "branch-main".to_string(),
            TurnEvent::TurnEnd {
                turn_id: "turn-1".into(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: Some(1200),
            },
        ),
    ];
    let (history, traces) = fold_session_views(&events, "branch-main", &[], &[]);
    // 消息视图：user + assistant（同源事件派生）。
    assert_eq!(history.len(), 2, "message view from events");
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, "hello");
    assert_eq!(history[1].role, "assistant");
    assert_eq!(history[1].content, "hi");
    // trace 视图：同一事件流派生（timeline 折叠 + tool_activities + token 指标）。
    assert_eq!(traces.len(), 1, "trace view from events");
    let trace = &traces[0];
    assert_eq!(trace.turn_id, "turn-1");
    let kinds: Vec<&str> = trace
        .trace_timeline
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        vec!["call_model", "call_tool", "return_result"],
        "timeline folded from same events"
    );
    assert_eq!(trace.tool_activities.len(), 1);
    assert_eq!(trace.tool_activities[0].status, "done");
    assert_eq!(trace.turn_duration_ms, Some(1200));
    // ProviderCallCacheRecord 由 MetricsProjection 挂载（wire 兼容保留字段）。
    assert_eq!(trace.provider_call_records.len(), 1);
    assert_eq!(
        trace.provider_call_records[0].provider_source.as_deref(),
        Some("deepseek")
    );
    assert_eq!(trace.provider_call_records[0].input_tokens, Some(100));
    // 消息视图与 trace 视图同源：assistant 文本 == timeline call_model 文本。
    let model_entry = trace
        .trace_timeline
        .iter()
        .find(|e| e.kind == "call_model")
        .expect("call_model entry");
    assert_eq!(
        model_entry.text.as_deref(),
        Some("hi"),
        "message and trace share the same event-derived text"
    );
}

// ── 侧边栏三级树：标题 override / 归档 / 删除归属重写 ─────────────────────────

fn find_overview<'a>(
    sessions: &'a [crate::agent::session::SessionOverview],
    conversation_id: &str,
) -> &'a crate::agent::session::SessionOverview {
    sessions
        .iter()
        .find(|session| session.conversation_id == conversation_id)
        .unwrap_or_else(|| panic!("overview missing: {conversation_id}"))
}

#[test]
fn session_rename_writes_override_and_survives_refresh_branches() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("preview"),
        "Please inspect runtime.rs session switching behavior.",
        "I will check it.",
        None,
        Vec::new(),
    );
    // 派生标题先确认生效（首条用户消息预览）。
    assert_eq!(
        find_overview(&store.list_sessions(), "preview").title,
        "Please inspect runtime.rs se..."
    );

    store.rename_session("preview", "我的自定义标题").unwrap();
    // list_sessions 投影走 override。
    assert_eq!(
        find_overview(&store.list_sessions(), "preview").title,
        "我的自定义标题"
    );
    // snapshot（主视图）投影走 override。
    let snapshot = store.snapshot(Some("preview"), &[]);
    assert_eq!(snapshot.title, "我的自定义标题");

    // 再来一轮：history 分支的 build_title 不得覆盖 override。
    store.append_turn(Some("preview"), "second turn message", "ok", None, Vec::new());
    assert_eq!(
        find_overview(&store.list_sessions(), "preview").title,
        "我的自定义标题"
    );
    // 同值重命名 = 成功 no-op。
    store.rename_session("preview", "我的自定义标题").unwrap();
    assert_eq!(
        find_overview(&store.list_sessions(), "preview").title,
        "我的自定义标题"
    );
}

#[test]
fn rename_session_validates_input_and_missing_session() {
    let mut store = SessionStore::memory_only();
    store.append_turn(Some("s1"), "first", "reply", None, Vec::new());

    assert!(store.rename_session("s1", "   ").is_err());
    assert!(store
        .rename_session("s1", &"长".repeat(65))
        .unwrap_err()
        .contains("64"));
    assert!(store.rename_session("missing-session", "任意").is_err());

    // 校验失败不产生副作用。
    assert!(store.sessions["s1"].title_override.is_none());
}

#[test]
fn override_beats_both_refresh_branches() {
    // 分支一：history 为空但 trace 存在 → trace.title 派生分支。
    // 分支二：history 非空 → build_title 派生分支。
    // 两分支在 override 存在时都必须跳过 title 赋值：effective_title 恒取
    // override，且底层 title 字段不被派生值污染。
    let mut store = SessionStore::memory_only();

    store.ensure_session("trace-only");
    {
        let session = store.sessions.get_mut("trace-only").unwrap();
        session.turn_trace_history.push(TurnTraceRecord {
            title: "trace 派生标题".to_string(),
            ..Default::default()
        });
        session.title_override = Some("用户命名".to_string());
    }
    {
        let session = store.sessions.get_mut("trace-only").unwrap();
        super::store::refresh_session_metadata(session, false);
        assert_eq!(session.effective_title(), "用户命名");
        assert_ne!(
            session.title, "trace 派生标题",
            "override 存在时 trace 分支不得改写 title 字段"
        );
    }

    store.ensure_session("history-full");
    {
        let session = store.sessions.get_mut("history-full").unwrap();
        session.history.push(TurnHistoryMessage {
            role: "user".to_string(),
            content: "please derive a title from this".to_string(),
            attachments: Vec::new(),
            ..Default::default()
        });
        session.title_override = Some("另一命名".to_string());
    }
    {
        let session = store.sessions.get_mut("history-full").unwrap();
        super::store::refresh_session_metadata(session, false);
        assert_eq!(session.effective_title(), "另一命名");
        assert_ne!(
            session.title, "please derive a title fro...",
            "override 存在时 build_title 分支不得改写 title 字段"
        );
    }
}

#[test]
fn archive_is_idempotent_and_projects_flag() {
    let mut store = SessionStore::memory_only();
    store.append_turn(Some("arch"), "first message", "reply", None, Vec::new());

    store.archive_session("arch").unwrap();
    assert!(
        find_overview(&store.list_sessions(), "arch").archived,
        "归档后 overview 投影 archived=true"
    );
    // 幂等：重复归档成功且仍为 true。
    store.archive_session("arch").unwrap();
    assert!(find_overview(&store.list_sessions(), "arch").archived);
    // 未归档会话恒为 false。
    store.append_turn(Some("live"), "another", "reply", None, Vec::new());
    assert!(!find_overview(&store.list_sessions(), "live").archived);
    assert!(store.archive_session("missing").is_err());
}

#[test]
fn workspace_delete_rewrites_member_sessions_to_default() {
    let root = std::env::temp_dir().join(format!("pa-tree-del-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut store = SessionStore::memory_only();
    let record = store.create_workspace("Doomed", &root.display().to_string()).unwrap();

    store.append_turn(Some("member"), "hello workspace", "reply", None, Vec::new());
    store.stamp_workspace_id("member", &record.id);
    assert_eq!(
        find_overview(&store.list_sessions(), "member")
            .workspace_id
            .as_deref(),
        Some(record.id.as_str())
    );

    store.delete_workspace(&record.id).unwrap();
    // 注册表已无该 id；resolve 失败（注册层面）。
    assert!(!store.list_workspaces().iter().any(|w| w.id == record.id));
    // 名下会话归属被重写为 default；默认根解析可用（附件导入路径恢复）。
    assert_eq!(
        find_overview(&store.list_sessions(), "member")
            .workspace_id
            .as_deref(),
        Some(crate::agent::workspace::DEFAULT_WORKSPACE_ID)
    );
    assert!(store.resolve_workspace_root(None).is_ok());
    // default 拒删。
    assert!(store
        .delete_workspace(crate::agent::workspace::DEFAULT_WORKSPACE_ID)
        .is_err());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn session_state_new_fields_roundtrip_and_default_for_legacy_blobs() {
    let mut store = SessionStore::memory_only();
    store.append_turn(Some("rt"), "round trip", "reply", None, Vec::new());
    store.rename_session("rt", "改名后").unwrap();
    store.archive_session("rt").unwrap();

    let serialized = serde_json::to_value(&store.sessions["rt"]).unwrap();
    assert_eq!(serialized["titleOverride"], "改名后");
    assert_eq!(serialized["archived"], true);

    // 旧 blob（两键皆无）→ serde default 还原为 None/false。
    let mut legacy = serialized.clone();
    legacy.as_object_mut().unwrap().remove("titleOverride");
    legacy.as_object_mut().unwrap().remove("archived");
    let restored: crate::agent::session::types::SessionState =
        serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.title_override, None);
    assert!(!restored.archived);
}

#[test]
fn checkout_of_pre_rename_node_shows_override_title_and_keeps_frozen_nodes() {
    // B6-T1（P1 级回归钉）：选中历史节点分支（snapshot_from_state 的
    // selected_node 分支）必须走 effective_title——改名前提交的节点冻结了
    // 旧派生标题，直取 node.title 会在此处复活旧标题。
    let mut store = SessionStore::memory_only();
    store.append_turn(Some("ck"), "first message text here", "reply", None, Vec::new());
    {
        let session = store.sessions.get_mut("ck").unwrap();
        session.history_nodes.push(HistoryNode {
            node_id: "node-old".to_string(),
            session_id: "ck".to_string(),
            branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
            title: "first message text here".to_string(),
            summary: String::new(),
            ..Default::default()
        });
        session.history_cursor.visible_node_id = Some("node-old".to_string());
    }
    store.rename_session("ck", "用户命名").unwrap();

    let snapshot = store.snapshot_for_session_at("ck", Some("node-old"));
    assert_eq!(
        snapshot.title, "用户命名",
        "checkout 改名前节点时顶层标题必须是 override"
    );
    // 半边契约：会话的历史节点列表保留提交时刻的派生标题。
    assert_eq!(
        store.sessions["ck"].history_nodes[0].title,
        "first message text here"
    );
}

#[test]
fn hydrate_backfill_is_neutralized_by_override() {
    // B6-T2：hydrate 两处把 node.title 回灌 session.title——override 字段
    // 独立使其无害化；本测试钉住"回灌后投影仍取 override"。
    let mut store = SessionStore::memory_only();
    store.append_turn(Some("hy"), "derive me please", "reply", None, Vec::new());
    {
        let session = store.sessions.get_mut("hy").unwrap();
        session.history_nodes.push(HistoryNode {
            node_id: "n1".to_string(),
            session_id: "hy".to_string(),
            branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
            title: "derive me please".to_string(),
            summary: String::new(),
            ..Default::default()
        });
        session.title_override = Some("用户命名".to_string());
    }
    let node = store.sessions["hy"].history_nodes[0].clone();

    {
        let session = store.sessions.get_mut("hy").unwrap();
        super::store::hydrate_session_from_node(session, &node);
        assert_eq!(session.effective_title(), "用户命名");
        assert_eq!(
            session.title, "derive me please",
            "回灌照常覆盖底层 title 字段（无害化的前提）"
        );
    }
    {
        let session = store.sessions.get_mut("hy").unwrap();
        super::store::hydrate_session_from_projection(session, &node, Vec::new(), Vec::new());
        assert_eq!(session.effective_title(), "用户命名");
    }
}

#[test]
fn workspace_rename_delete_survive_sqlite_restart() {
    // B6-T3：rename/delete 经真实 SQLite 后端跨重启持久（此前仅 memory_only 覆盖）。
    let dir = std::env::temp_dir().join(format!("pa-tree-t3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("store.db");
    let ws_root = dir.join("ws-root");
    std::fs::create_dir_all(&ws_root).unwrap();

    // 阶段一：创建 W1/W2 → rename W1 → 成员盖章 W1 → 删除 W2。
    {
        let mut store = SessionStore::with_backend(Box::new(
            crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                db_path.clone(),
                crate::agent::session::SeparateTraceTableMode::DualWrite,
            ),
        ));
        let w1 = store.create_workspace("One", &ws_root.display().to_string()).unwrap();
        std::fs::create_dir_all(&dir.join("two")).unwrap();
        let w2 = store.create_workspace("Two", &dir.join("two").display().to_string()).unwrap();
        store.append_turn(Some("member"), "hello", "reply", None, Vec::new());
        store.stamp_workspace_id("member", &w1.id);
        store.rename_workspace(&w1.id, "Renamed").unwrap();
        store.delete_workspace(&w2.id).unwrap();
    }

    // 阶段二：重启断言——新名在册、W2 消失、成员归属保持 W1。
    {
        let mut store = SessionStore::with_backend(Box::new(
            crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                db_path.clone(),
                crate::agent::session::SeparateTraceTableMode::DualWrite,
            ),
        ));
        let registry = store.list_workspaces();
        assert!(registry.iter().any(|w| w.name == "Renamed"));
        assert!(!registry.iter().any(|w| w.name == "Two"));
        let member_id = store
            .list_sessions()
            .iter()
            .find(|s| s.conversation_id == "member")
            .unwrap()
            .workspace_id
            .clone()
            .unwrap();
        assert_eq!(registry.iter().find(|w| w.id == member_id).unwrap().name, "Renamed");

        // 阶段三：删除成员所在工作区 → 归属重写 default → 再次重启仍保持。
        store.delete_workspace(&member_id).unwrap();
        drop(store);
        let store = SessionStore::with_backend(Box::new(
            crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                db_path,
                crate::agent::session::SeparateTraceTableMode::DualWrite,
            ),
        ));
        assert_eq!(
            store
                .list_sessions()
                .iter()
                .find(|s| s.conversation_id == "member")
                .unwrap()
                .workspace_id
                .as_deref(),
            Some(crate::agent::workspace::DEFAULT_WORKSPACE_ID)
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn workspace_rename_delete_survive_sqlite_restart_write_separate() {
    // T3 生产模式变体：SessionStore::new 默认 WriteSeparate——同一生命周期
    // 在该模式下复跑，钉住 blob 剥离语义与注册表/归属重写的组合行为。
    let dir = std::env::temp_dir().join(format!("pa-tree-t3-ws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("store.db");
    let ws_root = dir.join("ws-root");
    std::fs::create_dir_all(&ws_root).unwrap();

    {
        let mut store = SessionStore::with_backend(Box::new(
            crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                db_path.clone(),
                crate::agent::session::SeparateTraceTableMode::WriteSeparate,
            ),
        ));
        let w1 = store.create_workspace("One", &ws_root.display().to_string()).unwrap();
        store.append_turn(Some("member"), "hello", "reply", None, Vec::new());
        store.stamp_workspace_id("member", &w1.id);
        store.rename_workspace(&w1.id, "Renamed").unwrap();
        store.delete_workspace(&w1.id).unwrap();
    }

    {
        let store = SessionStore::with_backend(Box::new(
            crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                db_path.clone(),
                crate::agent::session::SeparateTraceTableMode::WriteSeparate,
            ),
        ));
        assert!(!store.list_workspaces().iter().any(|w| w.name == "Renamed"));
        assert_eq!(
            store
                .list_sessions()
                .iter()
                .find(|s| s.conversation_id == "member")
                .unwrap()
                .workspace_id
                .as_deref(),
            Some(crate::agent::workspace::DEFAULT_WORKSPACE_ID)
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}
