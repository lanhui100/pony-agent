use pony_agent_core::agent::context::RetrievedContextState;
use pony_agent_core::agent::tool_runtime::{McpResourceOperation, McpTransport, McpTransportRequest, StdioMcpTransport};
use std::fs;

#[test]
fn test_stdio_mcp_transport_roundtrip_with_echo_sh() {
    let script_path = std::env::temp_dir().join(format!("mcp-echo-{}.sh", uuid::Uuid::new_v4()));
    // Write an echo responder that mimics JSON-RPC 2.0
    fs::write(
        &script_path,
        r#"#!/bin/sh
read line
echo '{"jsonrpc":"2.0","id":1,"result":{"resources":[{"uri":"file:///test","name":"test"}]}}'
"#,
    ).expect("write script");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let transport = StdioMcpTransport::new(script_path.to_str().unwrap(), vec![]);
    let req = McpTransportRequest {
        source_id: "test-src".to_string(),
        source_revision: "1".to_string(),
        operation: McpResourceOperation::ListResources,
        arguments: serde_json::json!({}),
    };

    let res = transport.execute(&req);
    assert!(res.is_ok(), "StdioMcpTransport execute must succeed: {:?}", res);
    let content = res.unwrap().content;
    assert_eq!(content["resources"][0]["name"], "test");

    let _ = fs::remove_file(script_path);
}
