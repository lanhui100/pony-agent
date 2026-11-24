//! Acceptance tests for sandbox runtime and host-approved execution (PA-113 / Stage 1 Red Phase).

use pony_agent_core::agent::sandbox::{
    enforce_sandbox, HostApprovedUnsandboxedBackend, NativeSandboxBackend,
    SandboxAvailability, SandboxBackend, SandboxRequest,
};
use pony_agent_core::agent::tools::{ToolCall, ToolRouter};

fn sample_request() -> SandboxRequest {
    SandboxRequest {
        workspace_root: ".".to_string(),
        allow_network: false,
        environment_allowlist: Vec::new(),
        isolate_environment: true,
    }
}

#[test]
fn test_host_approved_unsandboxed_backend_contract() {
    let backend = HostApprovedUnsandboxedBackend::new();
    assert_eq!(
        backend.availability(),
        SandboxAvailability::HostApprovedUnsandboxed
    );

    let req = sample_request();
    assert!(backend.validate(&req).is_ok());
    assert!(enforce_sandbox(&backend, &req).is_ok());
}

#[test]
fn test_native_sandbox_backend_detection_and_fallback() {
    let backend = NativeSandboxBackend::detect();
    let availability = backend.availability();

    // Verify availability contract
    match availability {
        SandboxAvailability::Available => {
            let req = sample_request();
            assert!(backend.validate(&req).is_ok());
            assert!(enforce_sandbox(&backend, &req).is_ok());
        }
        SandboxAvailability::Unavailable => {
            let req = sample_request();
            assert!(backend.validate(&req).is_err());
            assert!(enforce_sandbox(&backend, &req).is_err());
        }
        SandboxAvailability::HostApprovedUnsandboxed => {
            panic!("NativeSandboxBackend must never implicitly report HostApprovedUnsandboxed");
        }
    }
}

#[test]
fn test_tool_router_executes_with_host_approved_backend() {
    let router = ToolRouter::new().with_sandbox_backend(HostApprovedUnsandboxedBackend::new());

    #[cfg(windows)]
    let cmd = "cmd /c echo hello";
    #[cfg(not(windows))]
    let cmd = "echo hello";

    let call = ToolCall {
        call_id: Some("call-1".to_string()),
        name: "Run".to_string(),
        arguments: serde_json::json!({
            "command": cmd,
            "description": "test echo"
        }),
        plan: None,
    };

    let result = router.execute(&call);
    assert_eq!(result.status, "ok", "Expected ok status, got: {:?}", result);
    assert!(
        result.output.contains("hello"),
        "Expected output containing hello, got: {}",
        result.output
    );
}
