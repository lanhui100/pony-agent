//! 原 `tui.rs` 内联测试模块 `mod wave65_r2_approval_plan_tests`（Step1 机械外移，断言零改动）。

use super::*;

fn snapshot_with_path() -> serde_json::Value {
    serde_json::json!({
        "requestId": "control-r2-1",
        "options": {
            "approvalKind": "path_authorization",
            "tool": "Read",
            "path": "/tmp/outside/probe.txt",
            "scope": "read",
        },
    })
}

fn snapshot_without_path() -> serde_json::Value {
    serde_json::json!({
        "requestId": "control-r2-2",
        "options": { "approvalKind": "approval", "tool": "Bash" },
    })
}

#[test]
fn approval_plan_authorizes_only_path_approvals_on_approval() {
    // ① 有 path + 批准 ⇒ 必须先授权该路径。
    for choice in [PermissionChoice::AllowOnce, PermissionChoice::AllowSession] {
        let plan = approval_plan(&snapshot_with_path(), choice);
        assert_eq!(
            plan.authorize.as_deref(),
            Some("/tmp/outside/probe.txt"),
            "路径授权类审批在批准时必须带上待授权路径（choice = {choice:?}）"
        );
        assert_eq!(plan.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);
    }

    // ② 无 path + 批准 ⇒ 跳过授权直接 CAS（R2 F3：否则该审批永久批不掉）。
    let plan = approval_plan(&snapshot_without_path(), PermissionChoice::AllowOnce);
    assert_eq!(
        plan.authorize, None,
        "无结构化 path 的审批不得给出授权目标（更不得回退 callId）"
    );
    assert_eq!(plan.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);
}

#[test]
fn approval_plan_never_authorizes_on_deny() {
    // ③ 有 path + 拒绝 ⇒ 绝不授权（被拒绝的路径不能被顺手放行）。
    let denied = approval_plan(&snapshot_with_path(), PermissionChoice::Deny);
    assert_eq!(denied.authorize, None, "拒绝时不得写入任何路径授权");
    assert_eq!(denied.prompt, APPROVAL_DENIED_CONTINUATION_PROMPT);

    // ④ 无 path + 拒绝 ⇒ 同样无授权，文案是拒绝文案。
    let denied_without = approval_plan(&snapshot_without_path(), PermissionChoice::Deny);
    assert_eq!(denied_without.authorize, None);
    assert_eq!(denied_without.prompt, APPROVAL_DENIED_CONTINUATION_PROMPT);

    // 批准与拒绝文案必须不同（否则模型分不清「已放行」与「被拒绝」）。
    assert_ne!(denied.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);
}

#[test]
fn permission_request_takes_approval_path_only_from_structured_options() {
    // 有 `options.path` ⇒ 授权路径就是它。
    let with_path = permission_request_from_pending(&snapshot_with_path(), "sess-r2");
    assert_eq!(
        with_path.approval_path.as_deref(),
        Some("/tmp/outside/probe.txt")
    );

    // 无 `options.path` 但**有** `callId` ⇒ 授权路径必须为 None（callId 不是路径）。
    let call_id_only = serde_json::json!({
        "requestId": "control-r2-3",
        "callId": "call-approval-e2e-1",
        "options": { "approvalKind": "approval", "tool": "Bash" },
    });
    let request = permission_request_from_pending(&call_id_only, "sess-r2");
    assert_eq!(
        request.approval_path, None,
        "授权路径绝不回退 callId（否则 canonicalize 必败 ⇒ 审批永久批不掉）"
    );
    // 展示用的 `command_or_path` 保持既有回退语义（逐字不变）。
    assert_eq!(
        request.command_or_path.as_deref(),
        Some("call-approval-e2e-1"),
        "展示路径的既有回退语义不得被本次收紧改变"
    );
}
