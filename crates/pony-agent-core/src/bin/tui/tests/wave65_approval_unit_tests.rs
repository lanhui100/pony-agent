//! 原 `tui.rs` 内联测试模块 `mod wave65_approval_unit_tests`（Step1 机械外移，断言零改动）。

use super::*;

/// 隔离进程级数据目录（`Once` 保证只做一次，且早于任何 control plane 构造）。
fn isolate_process_data_dir() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let sandbox = std::env::temp_dir().join(format!(
            "pony-agent-tui-approval-unit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
    });
}

fn isolated_control_plane() -> HostControlPlane {
    isolate_process_data_dir();
    HostControlPlane::new()
}

#[test]
fn approval_continuation_prompt_is_non_empty_and_distinct() {
    let approved = approval_continuation_prompt(PermissionChoice::AllowOnce);
    let approved_session = approval_continuation_prompt(PermissionChoice::AllowSession);
    let denied = approval_continuation_prompt(PermissionChoice::Deny);

    // 非空：`prepare_turn` 的 reject_empty 会拒绝空 message ⇒ 空文案等于续轮永远失败。
    assert!(
        !approved.trim().is_empty(),
        "批准续轮文案不得为空串/空白，实际 = {approved:?}"
    );
    assert!(
        !denied.trim().is_empty(),
        "拒绝续轮文案不得为空串/空白，实际 = {denied:?}"
    );
    // 不同：批准与拒绝必须是两句话（否则模型无法区分「已放行」与「被拒绝」）。
    assert_ne!(
        approved, denied,
        "批准与拒绝的续轮文案必须不同，实际两者都是 {approved:?}"
    );
    // 两种批准档位（单次 / 会话）共用同一条「已批准」文案。
    assert_eq!(
        approved, approved_session,
        "AllowOnce 与 AllowSession 必须共用同一条批准续轮文案"
    );
}

#[test]
fn apply_approval_decision_without_pending_handle_returns_none() {
    let mut state = TuiState::default();
    assert!(
        state.pending_request.is_none(),
        "前置条件：默认 TuiState 不得带任何待决句柄"
    );
    let control_plane = isolated_control_plane();

    // 无待决审批句柄 ⇒ 不产生续轮（既不假装批准成功，也不伪造文案）。
    assert_eq!(
        apply_approval_decision(&mut state, &control_plane, PermissionChoice::AllowOnce),
        None,
        "无待决审批句柄时批准必须返回 None"
    );
    assert_eq!(
        apply_approval_decision(&mut state, &control_plane, PermissionChoice::Deny),
        None,
        "无待决审批句柄时拒绝必须返回 None"
    );
    assert!(
        state.pending_request.is_none(),
        "无句柄路径不得凭空造出一个待决句柄"
    );
}
