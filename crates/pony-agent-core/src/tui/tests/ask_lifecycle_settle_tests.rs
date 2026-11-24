//! B189 复审修复：Ask 收口只认**真实生命周期**，并按终态选文案（不再靠英文散文猜、不再断言「失效」）。
//!
//! 覆盖四类行为：
//! - 纯函数分派：`ask_settled_notice` 每态一句，Pending 不得说成「已收口」；
//! - 快照缺失 + 终态：静默收口，只落状态行，不注入 System 消息；
//! - 快照缺失 + 仍 `Pending`：**保持面板**（读偏/竞争不得误关）；
//! - 生命周期读失败：**保持面板** + 结构化日志 + 状态行提示（绝不静默收口）。

use super::*;
use crate::agent::ask_persistence::{
    open_ask_store, AskLifecycleState, AskPersistenceResult, AskPersistenceStore, PersistedAsk,
};
use crate::agent::tool_runtime::PendingControlRequestKind;
use std::sync::Arc;

fn temp_db_path(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("tui-ask-lifecycle-{tag}-{nanos}.db"))
}

fn cleanup_sqlite(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(name));
    }
}

fn persisted_ask(state: AskLifecycleState) -> PersistedAsk {
    PersistedAsk {
        session_id: "session-lifecycle".to_string(),
        request_id: "control-lifecycle-0001".to_string(),
        version: 2,
        nonce: "nonce-lifecycle".to_string(),
        run_id: Some("run-lifecycle".to_string()),
        turn_id: "turn-1".to_string(),
        call_id: "call-1".to_string(),
        kind: PendingControlRequestKind::Interaction,
        prompt: "测试问题？".to_string(),
        options: vec!["A".to_string()],
        state,
        created_at_ms: 1_000,
        answered_at_ms: None,
        multi_select: false,
    }
}

fn handle() -> PendingUiRequest {
    PendingUiRequest {
        request_id: "control-lifecycle-0001".to_string(),
        version: 2,
        run_id: Some("run-lifecycle".to_string()),
        expires_at_ms: None,
        is_approval: false,
    }
}

fn state_with_panel() -> TuiState {
    let mut state = TuiState::default();
    state.session_id = "session-lifecycle".to_string();
    state.pending_request = Some(handle());
    state.mode = InteractionMode::AskUserQuestion;
    state
}

fn plane_with_store(store: Arc<dyn AskPersistenceStore>) -> HostControlPlane {
    HostControlPlaneBuilder::new().ask_store(store).build()
}

/// 可注入「快照为空 / 读失败 / 仍 Pending」三种真值的测试存储。
struct ScriptedStore {
    load_ask_result: AskPersistenceResult<Option<PersistedAsk>>,
}

impl AskPersistenceStore for ScriptedStore {
    fn load_pending(&self, _session_id: &str) -> AskPersistenceResult<Vec<PersistedAsk>> {
        // 刻意返回空：模拟「请求已不在待决快照里」这一触发条件。
        Ok(Vec::new())
    }
    fn upsert(&self, _ask: &PersistedAsk) -> AskPersistenceResult<()> {
        Ok(())
    }
    fn load_ask(
        &self,
        _session_id: &str,
        _request_id: &str,
    ) -> AskPersistenceResult<Option<PersistedAsk>> {
        self.load_ask_result.clone()
    }
    fn supersede_pending_on_user_message(
        &self,
        _session_id: &str,
        _at_ms: u64,
    ) -> AskPersistenceResult<usize> {
        Ok(0)
    }
    fn gc_orphans(
        &self,
        _archived_session_ids: &[String],
        _now_ms: u64,
    ) -> AskPersistenceResult<usize> {
        Ok(0)
    }
}

#[test]
fn ask_settled_notice_is_lifecycle_specific_and_never_says_expired() {
    assert_eq!(
        ask_settled_notice(Some(AskLifecycleState::Answered)),
        ASK_SETTLED_ANSWERED_NOTICE
    );
    assert_eq!(
        ask_settled_notice(Some(AskLifecycleState::Cancelled)),
        ASK_SETTLED_CANCELLED_NOTICE
    );
    assert_eq!(
        ask_settled_notice(Some(AskLifecycleState::Superseded)),
        ASK_SETTLED_SUPERSEDED_NOTICE
    );
    assert_eq!(
        ask_settled_notice(Some(AskLifecycleState::Orphaned)),
        ASK_SETTLED_ORPHANED_NOTICE
    );
    assert_eq!(
        ask_settled_notice(Some(AskLifecycleState::Pending)),
        ASK_STILL_PENDING_NOTICE,
        "仍 Pending 时绝不能宣称已收口"
    );
    assert_eq!(ask_settled_notice(None), ASK_STALE_NOTICE);

    for notice in [
        ASK_STALE_NOTICE,
        ASK_SETTLED_ANSWERED_NOTICE,
        ASK_SETTLED_CANCELLED_NOTICE,
        ASK_SETTLED_SUPERSEDED_NOTICE,
        ASK_SETTLED_ORPHANED_NOTICE,
        ASK_STILL_PENDING_NOTICE,
    ] {
        assert!(
            !notice.contains("失效"),
            "DSH 对齐后 Ask 文案不得再出现「失效」，实际 = {notice:?}"
        );
    }
}

#[test]
fn sync_settles_answered_ask_with_lifecycle_notice_without_stale_system_message() {
    let db_path = temp_db_path("answered");
    let store: Arc<dyn AskPersistenceStore> =
        Arc::from(open_ask_store(&db_path).expect("open durable ask store"));
    store
        .upsert(&persisted_ask(AskLifecycleState::Answered))
        .expect("upsert answered ask");
    let plane = plane_with_store(Arc::clone(&store));

    let mut state = state_with_panel();
    let changed = sync_pending_control_request(&mut state, &plane);

    assert!(changed, "收口必须置脏重绘");
    assert!(state.pending_request.is_none(), "句柄必须被清除");
    assert_ne!(state.mode, InteractionMode::AskUserQuestion, "覆盖层必须退出");
    assert_eq!(
        state.status_notice.as_ref().map(|(text, _)| text.clone()),
        Some(ASK_SETTLED_ANSWERED_NOTICE.to_string()),
        "已作答必须给「已完成作答」文案，而不是兜底句"
    );
    assert!(
        state
            .messages
            .iter()
            .all(|message| message.content != ASK_STALE_NOTICE),
        "收口不得注入「已收口/失效」System 消息"
    );

    cleanup_sqlite(&db_path);
}

#[test]
fn sync_settles_cancelled_superseded_and_orphaned_asks_distinctly() {
    for (lifecycle, expected) in [
        (AskLifecycleState::Cancelled, ASK_SETTLED_CANCELLED_NOTICE),
        (AskLifecycleState::Superseded, ASK_SETTLED_SUPERSEDED_NOTICE),
        (AskLifecycleState::Orphaned, ASK_SETTLED_ORPHANED_NOTICE),
    ] {
        let db_path = temp_db_path("terminal");
        let store: Arc<dyn AskPersistenceStore> =
            Arc::from(open_ask_store(&db_path).expect("open durable ask store"));
        store
            .upsert(&persisted_ask(lifecycle))
            .expect("upsert terminal ask");
        let plane = plane_with_store(Arc::clone(&store));

        let mut state = state_with_panel();
        assert!(sync_pending_control_request(&mut state, &plane));
        assert_eq!(
            state.status_notice.as_ref().map(|(text, _)| text.clone()),
            Some(expected.to_string()),
            "{lifecycle:?} 必须用专属文案收口"
        );
        cleanup_sqlite(&db_path);
    }
}

#[test]
fn sync_keeps_panel_when_host_still_reports_pending() {
    let store: Arc<dyn AskPersistenceStore> = Arc::from(ScriptedStore {
        load_ask_result: Ok(Some(persisted_ask(AskLifecycleState::Pending))),
    });
    let plane = plane_with_store(store);

    let mut state = state_with_panel();
    let changed = sync_pending_control_request(&mut state, &plane);

    assert!(!changed, "真值仍 Pending 时不得声称界面变化");
    assert!(
        state.pending_request.is_some(),
        "快照缺失只是读偏/竞争，绝不能误关用户的提问面板"
    );
    assert_eq!(state.mode, InteractionMode::AskUserQuestion, "覆盖层必须保留");
}

#[test]
fn sync_keeps_panel_when_lifecycle_read_fails() {
    let store: Arc<dyn AskPersistenceStore> = Arc::from(ScriptedStore {
        load_ask_result: Err("ask_store_read_failed: simulated disk failure".to_string()),
    });
    let plane = plane_with_store(store);

    let mut state = state_with_panel();
    let changed = sync_pending_control_request(&mut state, &plane);

    assert!(changed, "首次失败要在状态行提示（置脏）");
    assert!(
        state.pending_request.is_some(),
        "读失败 ≠ 已收口：绝不静默关掉面板"
    );
    assert_eq!(
        state.status_notice.as_ref().map(|(text, _)| text.clone()),
        Some(ASK_LIFECYCLE_READ_FAILED_NOTICE.to_string())
    );

    // 同一失败的下一次轮询不得刷屏（不置脏、不重复提示）。
    let changed_again = sync_pending_control_request(&mut state, &plane);
    assert!(!changed_again, "同一失败只提示一次");
    assert!(state.pending_request.is_some());
}

#[test]
fn report_control_request_error_uses_lifecycle_notice_for_cas_conflict() {
    let db_path = temp_db_path("report");
    let store: Arc<dyn AskPersistenceStore> =
        Arc::from(open_ask_store(&db_path).expect("open durable ask store"));
    store
        .upsert(&persisted_ask(AskLifecycleState::Answered))
        .expect("upsert answered ask");
    let plane = plane_with_store(Arc::clone(&store));

    let mut state = state_with_panel();
    // 宿主真实错误串（大小写/措辞由实现决定，客户端不得靠 contains 猜）。
    report_control_request_error(
        &mut state,
        &plane,
        "提交回答失败",
        "ask_state_conflict: Ask `control-lifecycle-0001` is Answered, not Pending (terminal states are final)",
        Some(handle()),
    );

    let content = state
        .messages
        .last()
        .expect("必须上屏一条中文结论")
        .content
        .clone();
    assert_eq!(
        content, ASK_SETTLED_ANSWERED_NOTICE,
        "已作答的 CAS 冲突必须走生命周期真值文案，绝不能落到「失效」兜底"
    );
    assert!(!content.contains("失效"));
    assert!(state.pending_request.is_none(), "CAS 冲突后句柄必须清除");
    cleanup_sqlite(&db_path);
}

#[test]
fn report_control_request_error_without_handle_stays_neutral() {
    let store: Arc<dyn AskPersistenceStore> = Arc::from(ScriptedStore {
        load_ask_result: Ok(None),
    });
    let plane = plane_with_store(store);

    let mut state = TuiState::default();
    state.session_id = "session-lifecycle".to_string();
    report_control_request_error(
        &mut state,
        &plane,
        "恢复任务流失败",
        "unknown control request `control-lifecycle-0001`",
        None,
    );

    let content = state.messages.last().expect("必须上屏").content.clone();
    assert_eq!(content, ASK_STALE_NOTICE);
    assert!(!content.contains("失效"), "句柄未知也只给中性结论");
}

#[test]
fn report_control_request_error_keeps_approval_notice_distinct() {
    let store: Arc<dyn AskPersistenceStore> = Arc::from(ScriptedStore {
        load_ask_result: Ok(None),
    });
    let plane = plane_with_store(store);

    let mut state = TuiState::default();
    state.session_id = "session-lifecycle".to_string();
    let mut approval = handle();
    approval.is_approval = true;
    state.pending_request = Some(approval.clone());

    report_control_request_error(
        &mut state,
        &plane,
        "审批同意失败",
        "control request `control-lifecycle-0001` cannot be consumed: replay",
        Some(approval),
    );

    assert_eq!(
        state.messages.last().expect("必须上屏").content,
        APPROVAL_STALE_NOTICE,
        "审批失效不得复用 Ask 文案（Wave 65 R3）"
    );
}
