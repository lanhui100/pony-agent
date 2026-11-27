//! 原 `tui.rs` 内联测试模块 `mod wave65_r2_approval_plan_tests`（Step1 机械外移，断言零改动）。

use super::*;

fn snapshot_with_path() -> serde_json::Value {
    serde_json::json!({
        "requestId": "control-r2-1",
        "descriptorId": "builtin:workspace_read_file",
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
        "descriptorId": "mcp__demo__bash",
        "options": { "approvalKind": "approval", "tool": "Bash" },
    })
}

#[test]
fn approval_plan_authorizes_only_path_approvals_on_approval() {
    // ① 有 path + AllowOnce ⇒ 写单路径 read 授权，不扩大到会话级。
    let once = approval_plan(&snapshot_with_path(), PermissionChoice::AllowOnce);
    assert_eq!(
        once.authorize.as_deref(),
        Some("/tmp/outside/probe.txt"),
        "AllowOnce 路径授权类审批必须带上单路径授权"
    );
    assert!(!once.session_external_read);
    assert_eq!(once.session_tool_descriptor, None);
    assert_eq!(once.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);

    // ② 有 path + AllowSession ⇒ 仅写会话级工作区外读取权限（1A），不落单路径授权。
    let session_path = approval_plan(&snapshot_with_path(), PermissionChoice::AllowSession);
    assert_eq!(
        session_path.authorize, None,
        "AllowSession 不得退化为单路径授权"
    );
    assert!(
        session_path.session_external_read,
        "AllowSession 路径审批必须授予本会话工作区外读取权限"
    );
    assert_eq!(session_path.session_tool_descriptor, None);
    assert_eq!(session_path.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);

    // ③ 无 path + AllowOnce ⇒ 跳过授权直接 CAS（R2 F3：否则该审批永久批不掉）。
    let plan = approval_plan(&snapshot_without_path(), PermissionChoice::AllowOnce);
    assert_eq!(
        plan.authorize, None,
        "无结构化 path 的审批不得给出授权目标（更不得回退 callId）"
    );
    assert!(!plan.session_external_read);
    assert_eq!(plan.session_tool_descriptor, None);
    assert_eq!(plan.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);

    // ④ 无 path + AllowSession ⇒ 授予本会话该工具描述符免审批。
    let session_tool = approval_plan(&snapshot_without_path(), PermissionChoice::AllowSession);
    assert_eq!(session_tool.authorize, None);
    assert!(!session_tool.session_external_read);
    assert_eq!(
        session_tool.session_tool_descriptor.as_deref(),
        Some("mcp__demo__bash")
    );
    assert_eq!(session_tool.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);
}

#[test]
fn approval_plan_never_authorizes_on_deny() {
    // ⑤ 有 path + 拒绝 ⇒ 绝不授权（被拒绝的路径不能被顺手放行）。
    let denied = approval_plan(&snapshot_with_path(), PermissionChoice::Deny);
    assert_eq!(denied.authorize, None, "拒绝时不得写入任何路径授权");
    assert!(!denied.session_external_read);
    assert_eq!(denied.session_tool_descriptor, None);
    assert_eq!(denied.prompt, APPROVAL_DENIED_CONTINUATION_PROMPT);

    // ⑥ 无 path + 拒绝 ⇒ 同样无授权，文案是拒绝文案。
    let denied_without = approval_plan(&snapshot_without_path(), PermissionChoice::Deny);
    assert_eq!(denied_without.authorize, None);
    assert!(!denied_without.session_external_read);
    assert_eq!(denied_without.session_tool_descriptor, None);
    assert_eq!(denied_without.prompt, APPROVAL_DENIED_CONTINUATION_PROMPT);

    // 批准与拒绝文案必须不同（否则模型分不清「已放行」与「被拒绝」）。
    assert_ne!(denied.prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);
}

#[test]
fn permission_request_takes_approval_path_only_from_structured_options() {
    // 有 `options.path` ⇒ 授权路径就是它，同时保留 descriptor_id。
    let with_path = permission_request_from_pending(&snapshot_with_path(), "sess-r2");
    assert_eq!(
        with_path.approval_path.as_deref(),
        Some("/tmp/outside/probe.txt")
    );
    assert_eq!(
        with_path.descriptor_id.as_deref(),
        Some("builtin:workspace_read_file")
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

#[test]
fn apply_approval_decision_session_grant_enables_external_read_without_persisting_single_path() {
    use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;
    use pony_agent_core::agent::dispatcher::DispatchContext;
    use pony_agent_core::agent::dispatcher_composites::GovernedToolExecutor;
    use pony_agent_core::agent::governed_executor::build_governed_executor;
    use pony_agent_core::agent::runtime::AgentRuntimeBuilder;
    use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult};
    use std::sync::Arc;

    struct SharedExecutor(Arc<GovernedToolExecutor>);
    impl ToolExecutor for SharedExecutor {
        fn execute(&self, call: &ToolCall) -> ToolResult {
            self.0.execute(call)
        }
        fn execute_with_context(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
            self.0.execute_with_context(call, context)
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            self.0.as_any()
        }
    }

    let tmp_base = std::env::temp_dir().join(format!(
        "pony-tui-r2-session-approval-{}",
        std::process::id()
    ));
    let ws = tmp_base.join("ws");
    let ext = tmp_base.join("ext");
    let _ = std::fs::remove_dir_all(&tmp_base);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&ext).unwrap();
    let ext_1 = ext.join("one.txt");
    let ext_2 = ext.join("two.txt");
    std::fs::write(&ext_1, "first-ext").unwrap();
    std::fs::write(&ext_2, "second-ext").unwrap();

    let sessions = pony_test_support::memory_only_session_store();
    let auth_store = sessions.path_authorizations();
    let governed = Arc::new(build_governed_executor(Some(ws.clone()), Some(auth_store)));
    let runtime = AgentRuntimeBuilder::desktop()
        .host_approval_resume(true)
        .session_store(sessions)
        .workspace_root(&ws)
        .tool_executor(Box::new(SharedExecutor(Arc::clone(&governed))))
        .build();
    let control_plane = HostControlPlaneBuilder::new().runtime(runtime).build();
    let session_id = "sess-tui-allow-session";

    // 1. 首次读取 ext_1 触发真实 Approval 请求（写入 ask_dispatcher）
    let pending_outcome = governed.execute_with_context(
        &ToolCall {
            call_id: Some("call-1".to_string()),
            name: "Read".to_string(),
            arguments: serde_json::json!({
                "path": ext_1.display().to_string(),
                "description": "read ext 1"
            }),
            plan: None,
        },
        &ToolExecutionContext {
            workspace_root: Some(ws.clone()),
            session_id: Some(session_id.to_string()),
            dispatch_context: Some(DispatchContext {
                session_id: Some(session_id.to_string()),
                run_id: Some("run-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                workspace_root: Some(ws.display().to_string()),
                tool_authorization_mode: Some("workspace_write".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    assert_eq!(pending_outcome.status, "error");
    let pending = control_plane.list_pending_approvals(session_id);
    let item = pending
        .as_array()
        .and_then(|arr| arr.first())
        .expect("must have one pending approval");

    let mut state = TuiState::default();
    state.session_id = session_id.to_string();
    state.pending_request = Some(PendingUiRequest {
        request_id: item["requestId"].as_str().unwrap().to_string(),
        version: item["version"].as_u64().unwrap(),
        run_id: Some("run-1".to_string()),
        expires_at_ms: None,
        is_approval: true,
    });
    state.permission_state = Some(PermissionState::new(permission_request_from_pending(
        item,
        session_id,
    )));

    // 2. 选择 AllowSession 执行审批决策
    let prompt = apply_approval_decision(&mut state, &control_plane, PermissionChoice::AllowSession)
        .expect("AllowSession must succeed and return continuation prompt");
    assert_eq!(prompt, APPROVAL_APPROVED_CONTINUATION_PROMPT);
    assert!(
        control_plane.list_authorizations().is_empty(),
        "AllowSession 不得写入持久化单路径授权表"
    );

    // 3. 同一会话随后读取工作区外另一个文件 ext_2 直接成功
    let ok_res = governed.execute_with_context(
        &ToolCall {
            call_id: Some("call-2".to_string()),
            name: "Read".to_string(),
            arguments: serde_json::json!({
                "path": ext_2.display().to_string(),
                "description": "read ext 2"
            }),
            plan: None,
        },
        &ToolExecutionContext {
            workspace_root: Some(ws.clone()),
            session_id: Some(session_id.to_string()),
            dispatch_context: Some(DispatchContext {
                session_id: Some(session_id.to_string()),
                run_id: Some("run-1".to_string()),
                turn_id: Some("turn-2".to_string()),
                workspace_root: Some(ws.display().to_string()),
                tool_authorization_mode: Some("workspace_write".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    assert_eq!(
        ok_res.status, "ok",
        "AllowSession 后读取另一工作区外文件必须直接成功: {}",
        ok_res.output
    );
    assert!(ok_res.output.contains("second-ext"));

    let _ = std::fs::remove_dir_all(&tmp_base);
}
