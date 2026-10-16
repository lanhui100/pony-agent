use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor};
use pony_agent_core::agent::tool_runtime::ToolDispatcher;
use serde_json::json;
use std::fs;

#[test]
fn test_default_governed_executor_is_fail_closed_for_write_and_execute() {
    let dir = std::env::temp_dir().join(format!("pony-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("create dir");

    let executor = build_governed_executor(Some(dir.clone()), None);

    // Write file call should require approval under fail-closed default evaluator
    let call = ToolCall {
        call_id: Some("call-1".to_string()),
        name: "workspace_write_file".to_string(),
        arguments: json!({
            "path": "test.txt",
            "content": "hello",
            "description": "写入测试文件"
        }),
        plan: None,
    };

    let result = executor.execute(&call);
    assert_eq!(result.status, "error");
    let out: serde_json::Value = serde_json::from_str(&result.output).expect("valid json");
    assert_eq!(out["error"]["code"], "control_outcome_pending");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_control_plane_approve_control_request_flow() {
    use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;
    use pony_agent_core::agent::tool_runtime::{InvocationOrigin, ToolDispatchRequest};
    let cp = HostControlPlaneBuilder::new().build();

    let req = ToolDispatchRequest {
        origin: InvocationOrigin::Model,
        descriptor_id: "builtin:workspace_write_file".to_string(),
        call_id: "call-approve-1".to_string(),
        arguments: json!({
            "path": "test_appr.txt",
            "content": "hello",
            "description": "写入待审批文件"
        }),
    };
    let _ = cp.ask_dispatcher.dispatch(req);
    let raw_requests = cp.ask_dispatcher.pending_requests();
    let approval_req = raw_requests.iter().find(|r| r.call_id == "call-approve-1").expect("must exist");

    let approve_res = cp.approve_control_request(&approval_req.request_id, approval_req.version);
    assert!(approve_res.is_ok(), "approve_control_request must succeed with CAS version: {:?}", approve_res);
}
