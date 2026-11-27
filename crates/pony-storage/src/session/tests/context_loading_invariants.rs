//! B-CTX-LOAD：上下文装载、撤回状态语义分离、事件物化隔离与原子落盘不变量测试（T1–T5）。

use super::*;
use crate::session::backend::{PersistCommand, SeparateTraceTableMode};
use crate::sqlite_session::SqliteSessionBackend;
use pony_protocol::turn_event::TurnEvent;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(1);

fn temp_sqlite_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pony-storage-ctx-{tag}-{}-{}",
        std::process::id(),
        DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn new_sqlite_backend(dir: &Path, mode: SeparateTraceTableMode) -> Box<dyn SessionBackend> {
    let db_path = dir.join("sessions.db");
    Box::new(SqliteSessionBackend::new_with_trace_mode(db_path, mode))
}

/// T1：`DualWrite` 会话在 `checkout_history_node` 后，被撤回后缀记为 `TurnRewound`，
/// 图状态与会话元数据写进日志流；重启全量装载 / 稀疏定向装载均不得复活已撤回节点。
#[test]
fn t1_dual_write_checkout_persists_turn_rewound_and_survives_restart() {
    let dir = temp_sqlite_dir("t1");
    let sid = "sess-t1-dual";

    {
        let mut store =
            SessionStore::with_backend(new_sqlite_backend(&dir, SeparateTraceTableMode::DualWrite));
        store.append_turn_with_kind_and_trace(
            Some(sid),
            "turn 1 user",
            "turn 1 assistant",
            None,
            Vec::new(),
            None,
            Some("t1-u1"),
            Some(TurnTraceRecord {
                turn_id: "t1-u1".to_string(),
                session_id: Some(sid.to_string()),
                phase: "completed".to_string(),
                ..TurnTraceRecord::default()
            }),
        );
        let (nodes, _, _) = store.load_history_graph(Some(sid));
        assert_eq!(nodes.len(), 2, "load_history_graph 应补齐根 Checkpoint + Turn 1 节点");
        let root_node_id = nodes[0].node_id.clone();
        let node1_id = nodes[1].node_id.clone();

        let checked_out = store
            .checkout_history_node(
                Some(sid),
                &root_node_id,
                HistoryCheckoutMode::TranscriptOnly,
                None,
            )
            .expect("checkout to root should succeed");
        assert_eq!(checked_out.turn_count, 0);
        assert!(checked_out.history.is_empty());
        let rewound_node = checked_out
            .history_nodes
            .iter()
            .find(|n| n.node_id == node1_id)
            .expect("node1 still exists as rewound tombstone");
        assert_eq!(rewound_node.kind, HistoryNodeKind::TurnRewound);
        assert_eq!(
            checked_out.history_cursor.visible_node_id.as_deref(),
            Some(root_node_id.as_str())
        );
        assert_eq!(
            checked_out.history_cursor.branch_head_node_id.as_deref(),
            Some(root_node_id.as_str())
        );
    }

    // 重启全量装载：不得把 Turn 1 复活。
    {
        let reloaded =
            SessionStore::with_backend(new_sqlite_backend(&dir, SeparateTraceTableMode::WriteSeparate));
        let snap = reloaded.snapshot_for_session(sid);
        assert_eq!(snap.turn_count, 0, "重启后 turn_count 必须保持 0");
        assert!(snap.history.is_empty(), "撤回到根节点后历史必须保持空，实际={:?}", snap.history);
        let main_head = snap
            .history_branches
            .iter()
            .find(|b| b.branch_id == DEFAULT_HISTORY_BRANCH_ID)
            .and_then(|b| b.head_node_id.clone());
        assert_eq!(main_head, snap.history_cursor.visible_node_id);
        assert!(snap
            .history_nodes
            .iter()
            .any(|n| n.kind == HistoryNodeKind::TurnRewound));
    }

    // 重启稀疏定向装载：同样不得复活 Turn 1。
    {
        let mut lazy = SessionStore::open_lazy_with_backend(new_sqlite_backend(
            &dir,
            SeparateTraceTableMode::WriteSeparate,
        ));
        assert!(lazy.hydrate_session(sid));
        let snap = lazy.snapshot_for_session(sid);
        assert!(snap.history.is_empty(), "稀疏重启也不得复活已撤回历史");
    }

    let _ = fs::remove_dir_all(&dir);
}

/// T2：`TurnCancelled`（失败/取消活跃轮）允许进入双 Esc 撤回菜单与计划；
/// `TurnRewound` 被排除出撤回列表且 `ensure_history_graph` 不会复活 `TurnRewound` 为分支头。
#[test]
fn t2_rewind_includes_turn_cancelled_and_excludes_turn_rewound() {
    let mut store = SessionStore::memory_only();
    let sid = "sess-t2-rewind";

    store.append_turn_with_kind_and_trace(
        Some(sid),
        "failed turn question",
        "[中断轮次检查点]\n失败原因：boom",
        None,
        Vec::new(),
        Some(HistoryMessageKind::Error),
        Some("t2-err"),
        Some(TurnTraceRecord {
            turn_id: "t2-err".to_string(),
            session_id: Some(sid.to_string()),
            phase: "failed".to_string(),
            error: Some("boom".to_string()),
            ..TurnTraceRecord::default()
        }),
    );

    let turns = store.rewind_turns(Some(sid));
    assert_eq!(turns.len(), 1, "失败/取消的 TurnCancelled 轮次必须可撤回");
    assert_eq!(turns[0].kind, HistoryNodeKind::TurnCancelled);

    let err_node_id = turns[0].node_id.clone();
    let plan = store
        .rewind_plan(Some(sid), &err_node_id)
        .expect("TurnCancelled must produce rewind plan");
    assert_eq!(plan.user_message.as_deref(), Some("failed turn question"));

    let keep_node_id = plan
        .keep_node_id
        .clone()
        .expect("TurnCancelled must have parent checkpoint keep_node_id");

    // 撤回后后缀被标为 TurnRewound，不能再次撤回；ensure_history_graph 不得复活它。
    store
        .checkout_history_node(
            Some(sid),
            &keep_node_id,
            HistoryCheckoutMode::TranscriptOnly,
            None,
        )
        .expect("checkout restore target");
    assert!(
        store.rewind_turns(Some(sid)).is_empty(),
        "TurnRewound 不得再次出现在可撤回列表"
    );
    assert!(
        store.rewind_plan(Some(sid), &err_node_id).is_err(),
        "TurnRewound 节点不得再次作为撤回目标"
    );

    let session = store.sessions.get_mut(sid).expect("session");
    session.history_branches[0].head_node_id = None;
    store::ensure_history_graph(session);
    assert_eq!(
        session.history_branches[0].head_node_id.as_deref(),
        Some(keep_node_id.as_str()),
        "ensure_history_graph 回算分支头时绝不能选用 TurnRewound"
    );
}

/// T3：稀疏模式下经 `ensure_session` 新建的活跃会话在 `loaded` 集合内；
/// Turn 1 进行中即使已落盘 `turn_events`，后续 `hydrate_session` / `ensure_session` 不得触发重重建并制造重复消息。
#[test]
fn t3_sparse_new_session_marked_loaded_prevents_mid_turn_event_rehydration() {
    let dir = temp_sqlite_dir("t3");
    let sid = "sess-t3-sparse";
    let mut lazy = SessionStore::open_lazy_with_backend(new_sqlite_backend(
        &dir,
        SeparateTraceTableMode::WriteSeparate,
    ));

    // Turn 1 启动阶段先在内存中创建新会话，并写入一条 FlushEvents（模拟步骤级事件落盘）。
    lazy.ensure_session(sid);
    assert!(
        lazy.loaded_session_ids().iter().any(|id| id == sid),
        "新建会话必须立即进入 lazy.loaded"
    );
    let side_backend = new_sqlite_backend(&dir, SeparateTraceTableMode::WriteSeparate);
    let outcome = side_backend.persist_command(PersistCommand::FlushEvents {
        epoch: 1,
        session_id: sid.to_string(),
        turn_id: "turn-live-1".to_string(),
        branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
        events: vec![TurnEvent::UserMessage {
            turn_id: "turn-live-1".to_string(),
            text: "turn 1 midflight".to_string(),
            attachments: Vec::new(),
        }],
        event_times_ms: vec![1_700_000_000_000],
    });
    assert!(matches!(outcome, PersistCommandOutcome::Succeeded));

    // Turn 1 进行中读/水合检查必须命中 already-loaded 短路，而不是从 turn_events 重建 legacy-node-1。
    assert!(lazy.hydrate_session(sid));
    let mid_snap = lazy.snapshot_for_session(sid);
    assert!(
        mid_snap.history.is_empty(),
        "Turn 1 进行中不得由半截 turn_events 覆盖内存空白状态，实际={:?}",
        mid_snap.history
    );
    assert!(
        mid_snap.history_nodes.is_empty(),
        "Turn 1 进行中不得伪造 legacy-node-1"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// T4：`materialize_turn_messages` 按 `expected_turn_id` 精确隔离事件，绝不把 Turn N+1 的消息错归到 Turn N。
#[test]
fn t4_materialize_turn_messages_is_scoped_by_turn_id() {
    let events = vec![
        (
            10,
            DEFAULT_HISTORY_BRANCH_ID.to_string(),
            TurnEvent::UserMessage {
                turn_id: "turn-next".to_string(),
                text: "next turn question".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            11,
            DEFAULT_HISTORY_BRANCH_ID.to_string(),
            TurnEvent::AssistantMessage {
                turn_id: "turn-next".to_string(),
                step: 0,
                text: "next turn answer".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ),
    ];

    let stolen_check = store::materialize_turn_messages(&events, 0, Some("turn-lagging"));
    assert!(
        stolen_check.is_none(),
        "事件流里只有 turn-next 时，turn-lagging 必须返回 None 回退自身原文"
    );

    let matched = store::materialize_turn_messages(&events, 0, Some("turn-next"))
        .expect("matching turn_id should materialize");
    assert_eq!(matched.0.as_deref(), Some("next turn question"));
    assert_eq!(
        matched.1.as_ref().map(|(text, _)| text.as_str()),
        Some("next turn answer")
    );
}

/// T5：`append_turn_with_kind_and_trace_fallible` 在单批中同时落盘历史、`HistoryNode(turn_trace_refs)` 与 trace 行，
/// 重启后节点引用与 trace 明细保持完整一致。
#[test]
fn t5_atomic_append_with_trace_populates_node_refs_and_survives_reload() {
    let dir = temp_sqlite_dir("t5");
    let sid = "sess-t5-atomic";

    {
        let mut store = SessionStore::with_backend(new_sqlite_backend(
            &dir,
            SeparateTraceTableMode::WriteSeparate,
        ));
        let trace = TurnTraceRecord {
            turn_id: "t5-turn-1".to_string(),
            session_id: Some(sid.to_string()),
            phase: "cancelled".to_string(),
            error: Some("stopped_by_user".to_string()),
            tool_activities: vec![pony_protocol::telemetry::TurnToolActivity {
                id: "call-1".to_string(),
                name: "Read".to_string(),
                canonical_tool_name: Some("read".to_string()),
                display_name_zh: None,
                status: "ok".to_string(),
                description: "src/lib.rs".to_string(),
                arguments_text: None,
                result_text: Some("ok".to_string()),
                duration_seconds: Some(0.1),
                parent_activity_id: None,
                artifacts: None,
                error: None,
                capability_invocation: None,
            }],
            ..TurnTraceRecord::default()
        };
        let snap = store
            .append_turn_with_kind_and_trace_fallible(
                Some(sid),
                "读一下 lib.rs",
                "[中断轮次检查点]\n- Read src/lib.rs",
                None,
                Vec::new(),
                Some(HistoryMessageKind::Cancelled),
                Some("t5-turn-1"),
                Some(trace),
            )
            .expect("atomic append should succeed");

        let last_node = snap.history_nodes.last().expect("node committed");
        assert_eq!(last_node.kind, HistoryNodeKind::TurnCancelled);
        assert_eq!(last_node.turn_id.as_deref(), Some("t5-turn-1"));
        assert!(
            last_node
                .turn_trace_refs
                .as_ref()
                .is_some_and(|refs| refs.iter().any(|r| r.turn_id == "t5-turn-1")),
            "HistoryNode 必须自带非空 turn_trace_refs"
        );
        assert_eq!(snap.turn_trace_history.len(), 1);
    }

    {
        let reloaded = SessionStore::with_backend(new_sqlite_backend(
            &dir,
            SeparateTraceTableMode::WriteSeparate,
        ));
        let snap = reloaded.snapshot_for_session(sid);
        assert_eq!(snap.history.len(), 2);
        assert_eq!(snap.turn_trace_history.len(), 1);
        assert_eq!(snap.turn_trace_history[0].tool_activities.len(), 1);
        let last_node = snap.history_nodes.last().expect("reloaded node");
        assert!(
            last_node
                .turn_trace_refs
                .as_ref()
                .is_some_and(|refs| refs.iter().any(|r| r.turn_id == "t5-turn-1")),
            "重启后 HistoryNode.turn_trace_refs 不得丢失"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
