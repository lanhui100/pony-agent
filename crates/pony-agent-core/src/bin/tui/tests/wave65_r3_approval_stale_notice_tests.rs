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
