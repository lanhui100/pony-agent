//! Wave-4.4 验收（B084/B085/B086）。
//!
//! 2026-11-14 由 Test Agent 按 Lead 批准替换三条**空壳断言**（原实现只断言字符串常量恒真，
//! 属假测试；`pony-agent-sdk` 已整目录删除 ⇒ B085 的 SDK 会话隔离改为在 ACP 层验证）：
//! - B084：`protocolVersion` 必须是**整数 1**，并对真实 fake ACP agent 完成 `initialize` 握手；
//! - B085：两个独立 ACP 客户端（不同 fake agent / 不同 sessionId）不串会话、不串响应；
//! - B086：`session/request_permission` 无 handler ⇒ fail-closed（cancelled/error，绝不 allow）。
//!
//! 其余用例未改动；夹具复用 `tests/fixtures/acp_fake_agent.sh`。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use pony_agent_core::acp::client::AcpAgentClient;
use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::{
    ClientCapabilities, ContentBlock, InitializeRequest, PROTOCOL_VERSION,
};
use pony_agent_core::acp::transport::{
    AcpNotificationHandler, AcpTransport, ACP_REQUEST_TIMEOUT,
};
use serde_json::{json, Value};

/// 会话隔离用例不需要观测 update：吞掉通知即行为等价（仅用于本文件的 prompt 调用）。
struct NoopNotificationHandler;

impl AcpNotificationHandler for NoopNotificationHandler {
    fn handle_notification(&self, _method: &str, _params: Value) {}
}

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-wave4-4-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn fake_spec(mode: &str, extra: &[&str]) -> AcpAgentSpec {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    let mut args = vec![script.to_string_lossy().into_owned(), mode.to_string()];
    args.extend(extra.iter().map(|value| (*value).to_string()));
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
    }
}

fn initialize_params() -> Value {
    serde_json::to_value(InitializeRequest {
        protocol_version: PROTOCOL_VERSION,
        client_capabilities: ClientCapabilities::default(),
        client_info: None,
    })
    .expect("initialize 请求必须可序列化")
}

/// B084：ACP 握手的真契约——`protocolVersion` 是整数 `1`，且能对真实 agent 完成 initialize 往返。
#[test]
fn test_b084_acp_handshake_contract() {
    let value = initialize_params();
    assert!(
        value["protocolVersion"].is_number(),
        "B084：protocolVersion 必须是 JSON number，实际 {}",
        value["protocolVersion"]
    );
    assert_eq!(
        value["protocolVersion"],
        json!(1),
        "B084：protocolVersion 必须是整数 1（v1）"
    );
    assert_eq!(PROTOCOL_VERSION, 1);

    let cwd = temp_dir("handshake");
    let spec = fake_spec("flow", &["end_turn", "acp-sess-1"]);
    let client = AcpAgentClient::connect(&spec, &cwd);
    let mut client = match client {
        Ok(client) => client,
        Err(error) => panic!("B084：对 fake agent 的 initialize 握手必须成功，实际 {error:?}"),
    };
    let session = client
        .new_session(&cwd, Vec::new())
        .expect("B084：握手后的 session/new 必须成功");
    assert_eq!(session.session_id, "acp-sess-1", "B084：sessionId 必须来自 agent");
    client.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

/// B085：两个独立 ACP 客户端（不同 agent 实例 / 不同 sessionId）并发跑一轮，会话与响应不串扰。
#[test]
fn test_b085_sdk_session_isolation_adversarial() {
    let cwd = temp_dir("isolation");
    let left = fake_spec("flow", &["end_turn", "sess-left"]);
    let right = fake_spec("flow", &["max_tokens", "sess-right"]);

    let mut left_client = match AcpAgentClient::connect(&left, &cwd) {
        Ok(client) => client,
        Err(error) => panic!("B085：左侧客户端必须可连接，实际 {error:?}"),
    };
    let mut right_client = match AcpAgentClient::connect(&right, &cwd) {
        Ok(client) => client,
        Err(error) => panic!("B085：右侧客户端必须可连接，实际 {error:?}"),
    };

    let left_session = left_client
        .new_session(&cwd, Vec::new())
        .expect("B085：左侧 session/new 必须成功");
    let right_session = right_client
        .new_session(&cwd, Vec::new())
        .expect("B085：右侧 session/new 必须成功");
    assert_eq!(left_session.session_id, "sess-left", "B085：左侧会话 id 必须独立");
    assert_eq!(right_session.session_id, "sess-right", "B085：右侧会话 id 必须独立");
    assert_ne!(
        left_session.session_id, right_session.session_id,
        "B085：两客户端会话 id 不得串扰"
    );

    // 两侧各跑一轮，响应必须回到各自客户端（左侧 end_turn、右侧 max_tokens）。
    let left_result = left_client.prompt(
        &left_session.session_id,
        vec![ContentBlock::Text {
            text: "left task".to_string(),
        }],
        Arc::new(NoopNotificationHandler),
    );
    let right_result = right_client.prompt(
        &right_session.session_id,
        vec![ContentBlock::Text {
            text: "right task".to_string(),
        }],
        Arc::new(NoopNotificationHandler),
    );
    let left_reason = left_result.expect("B085：左侧 prompt 必须成功").stop_reason;
    let right_reason = right_result.expect("B085：右侧 prompt 必须成功").stop_reason;
    assert_eq!(
        left_reason,
        pony_agent_core::acp::protocol::StopReason::EndTurn,
        "B085：左侧响应串到了别的客户端"
    );
    assert_eq!(
        right_reason,
        pony_agent_core::acp::protocol::StopReason::MaxTokens,
        "B085：右侧响应串到了别的客户端"
    );

    left_client.close();
    right_client.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

/// B086：headless 授权默认拒绝——无 handler 的权限请求必须 fail-closed（cancelled/error）。
#[test]
fn test_b086_headless_auth_fail_closed() {
    let cwd = temp_dir("auth-fail-closed");
    let spec = fake_spec("permission", &[]);
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("B086：fake agent 应可启动，实际 {error:?}"),
    };
    // 刻意不注入 AcpInboundRequestHandler：权限请求必须被拒，绝不默认 allow。
    let result = transport
        .request(
            "initialize",
            initialize_params(),
            ACP_REQUEST_TIMEOUT,
        )
        .expect("B086：fail-closed 应答不得让握手挂死到 deadline");
    let recorded = result["recordedOutcome"].as_str().unwrap_or("<missing>");
    assert!(
        recorded == "cancelled" || recorded == "error",
        "B086 违约：无 handler 时必须回 cancelled 或 error，实际 {recorded}"
    );
    assert_ne!(
        recorded, "selected",
        "B086 硬约束：无 handler 时绝不能默认 allow"
    );

    transport.close();
    let _ = std::fs::remove_dir_all(&cwd);
}
