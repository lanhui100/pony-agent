//! 原 `tui.rs` 内联测试模块 `mod wave65_r3_approval_stale_notice_tests`（Step1 机械外移，断言零改动）。

use super::*;

fn handle(is_approval: bool) -> PendingUiRequest {
    PendingUiRequest {
        request_id: "control-r3-000000000001".to_string(),
        version: 1,
        run_id: Some("run-r3".to_string()),
        // 刻意给一个早已过去的期限：文案判定只看句柄种类，不看墙钟（ADR 0074）。
        expires_at_ms: Some(1),
        is_approval,
    }
}

#[test]
fn stale_notice_is_kind_specific() {
    assert_eq!(
        stale_pending_notice(Some(&handle(true))),
        APPROVAL_STALE_NOTICE,
        "审批句柄必须用审批专属失效文案"
    );
    assert_eq!(
        stale_pending_notice(Some(&handle(false))),
        ASK_STALE_NOTICE,
        "提问句柄保持既有 Ask 文案（逐字不变）"
    );
    // 无句柄（如 `--demo-ask-user` 演示面板、无宿主句柄的问答面板）沿用既有 Ask 文案。
    assert_eq!(stale_pending_notice(None), ASK_STALE_NOTICE);

    // 两条文案必须不同，且审批文案要说明「本轮不会自动继续 + 请重新发起」。
    assert_ne!(APPROVAL_STALE_NOTICE, ASK_STALE_NOTICE);
    assert!(
        APPROVAL_STALE_NOTICE.contains("不会自动继续"),
        "审批失效文案必须说明本轮不会自动继续，实际 = {APPROVAL_STALE_NOTICE:?}"
    );
    assert!(
        APPROVAL_STALE_NOTICE.contains("重新发起"),
        "审批失效文案必须给出下一步动作，实际 = {APPROVAL_STALE_NOTICE:?}"
    );
}

#[test]
fn close_stale_pending_request_uses_approval_notice_for_approval_handle() {
    let mut state = TuiState::default();
    state.pending_request = Some(handle(true));

    close_stale_pending_request(&mut state);

    assert!(
        state.pending_request.is_none(),
        "失效收口必须清掉句柄"
    );
    let last = state
        .messages
        .last()
        .expect("失效收口必须在消息区留下一条系统消息");
    assert_eq!(last.role, MessageRole::System);
    assert_eq!(last.content, APPROVAL_STALE_NOTICE);
    assert_eq!(
        state.status_notice.as_ref().map(|(text, _)| text.as_str()),
        Some(APPROVAL_STALE_NOTICE)
    );
}

#[test]
fn sync_pending_control_request_silently_closes_answered_ask_without_stale_notice() {
    use pony_agent_core::agent::ask_persistence::{
        open_ask_store, AskLifecycleState, AskPersistenceStore, PersistedAsk,
    };
    use std::sync::Arc;

    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!(
        "tui-ask-close-test-{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store: Arc<dyn AskPersistenceStore> = Arc::from(
        open_ask_store(&db_path).expect("open durable ask store"),
    );

    let session_id = "test-session-dsh-align";
    let request_id = "control-answered-0001";

    // 写入一条已经 Answered 的 Ask 记录（模拟用户刚作答落地或在其他端作答）
    let ask = PersistedAsk {
        session_id: session_id.to_string(),
        request_id: request_id.to_string(),
        version: 2,
        nonce: "nonce-1".to_string(),
        run_id: Some("run-1".to_string()),
        turn_id: "turn-1".to_string(),
        call_id: "call-1".to_string(),
        kind: pony_agent_core::agent::tool_runtime::PendingControlRequestKind::Interaction,
        prompt: "测试问题？".to_string(),
        options: vec!["A".to_string(), "B".to_string()],
        state: AskLifecycleState::Answered,
        created_at_ms: 1000,
        answered_at_ms: Some(2000),
        multi_select: false,
    };
    store.upsert(&ask).expect("upsert answered ask");

    let runtime = pony_agent_core::agent::runtime::AgentRuntimeBuilder::new()
        .session_store(pony_test_support::memory_only_session_store())
        .build();
    let control_plane = pony_agent_core::agent::control_plane::HostControlPlaneBuilder::new()
        .runtime(runtime)
        .ask_store(Arc::clone(&store))
        .build();

    let mut state = TuiState::default();
    state.session_id = session_id.to_string();
    state.pending_request = Some(PendingUiRequest {
        request_id: request_id.to_string(),
        version: 2,
        run_id: Some("run-1".to_string()),
        expires_at_ms: None,
        is_approval: false,
    });
    state.mode = InteractionMode::AskUserQuestion;

    // Act
    let changed = sync_pending_control_request(&mut state, &control_plane);

    // Assert
    assert!(changed, "状态应发生变化并置脏");
    assert!(state.pending_request.is_none(), "面板句柄应已被清除");
    assert_ne!(state.mode, InteractionMode::AskUserQuestion, "覆盖层应已退出");

    // DSH 对齐关键断言：已作答的提问收口，绝不上屏「该提问已失效」！
    let stale_messages = state
        .messages
        .iter()
        .filter(|msg| msg.content == ASK_STALE_NOTICE)
        .count();
    assert_eq!(
        stale_messages, 0,
        "对齐 DSH：已作答的提问从待决列表移出时，绝不上屏「该提问已失效」"
    );

    let _ = std::fs::remove_file(&db_path);
}
