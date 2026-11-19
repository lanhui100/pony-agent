//! B254 红相验收（C1/C2/C4）：流式 tool_followup 断流必须**可重试**，且**绝不回退**到
//! 同步 buffered 通道，也**不得**合成「本地兜底摘要」把回合按成功收口。
//!
//! 契约真值源：`.dev-team/contract-matrix-b254.md`（Lead 冻结 2026-11-19）。
//!
//! 可观察判据（只锁行为，不锁实现）：
//! 1. mock 对端记录**每一条真实 HTTP 请求体**——`"stream": false` 出现即证明发生了同步回退（C1 反证）；
//! 2. 被截断的流式响应（无完整 SSE 帧、body 提前断开）必须触发**同形重试**（同 URL 请求数 ≥ 2，C2）；
//! 3. 最终结果必须是 `Err`（C4），且错误正文不得含「兜底」等合成摘要话术。
//!
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern, UpstreamPolicy,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderManager, ProviderMessage, ProviderProtocol, ProviderRequest,
};
use pony_agent_core::agent::tools::{ToolCall, ToolResult};

/// 被截断的 SSE 响应体：只有一个**未闭合**的 data 帧（无 `\n\n`），且 Content-Length 远大于实发字节。
/// 客户端读到连接提前关闭 ⇒ `error decoding response body` / `unexpected eof`，且**没有任何增量**发出。
const TRUNCATED_SSE_BODY: &str =
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"b254-partial";

/// 声明的长度远大于实发长度，制造 body 读失败（不是干净 EOF）。
const DECLARED_CONTENT_LENGTH: usize = 4096;

/// 固定剧本 mock：记录真实请求体，回一个被截断的 SSE 响应后关闭连接。
struct TruncatingSseProvider {
    base_url: String,
    addr: String,
    bodies: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TruncatingSseProvider {
    fn start() -> Self {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("B254：mock provider 必须能绑定回环端口");
        let port = listener
            .local_addr()
            .expect("B254：mock provider 必须能取本地地址")
            .port();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let bodies_for_thread = Arc::clone(&bodies);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                let body = read_http_request(&mut stream);
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(mut slot) = bodies_for_thread.lock() {
                    slot.push(body);
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {DECLARED_CONTENT_LENGTH}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(TRUNCATED_SSE_BODY.as_bytes());
                let _ = stream.flush();
                // 提前断开：实际字节 < 声明长度 ⇒ 客户端 body 读失败。
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            bodies,
            shutdown,
            handle: Some(handle),
        }
    }

    fn selection(&self) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "b254-provider".to_string(),
            provider_name: "b254-provider".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: self.base_url.clone(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "B254_TEST_API_KEY".to_string(),
            api_key: Some("b254-dummy-key".to_string()),
            model: "b254-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 64,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(8_192),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        }
    }

    fn recorded_bodies(&self) -> Vec<String> {
        self.bodies.lock().map(|slot| slot.clone()).unwrap_or_default()
    }
}

impl Drop for TruncatingSseProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(_) => break,
        }
        if header_end.is_none() {
            if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = Some(index + 4);
                let headers = String::from_utf8_lossy(&buffer[..index + 4]).to_lowercase();
                for line in headers.lines() {
                    if let Some(value) = line.strip_prefix("content-length:") {
                        content_length = value.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
        }
        if let Some(header_end) = header_end {
            if buffer.len().saturating_sub(header_end) >= content_length {
                break;
            }
        }
    }
    let body_start = header_end.unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[body_start..]).to_string()
}

/// 快失败策略：只允许 2 次 attempt、退避预算 1s，避免验收用例在 500ms→8s 曲线上拖到分钟级。
fn fast_fail_policy() -> UpstreamPolicy {
    UpstreamPolicy {
        decision_max_attempts: 2,
        retry_budget_ms: 1_000,
        ..Default::default()
    }
}

fn request_probe() -> ProviderRequest {
    ProviderRequest {
        model: "b254-model".to_string(),
        input: vec![ProviderMessage::user("b254 followup probe")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    }
}

fn tool_probe() -> (ToolCall, ToolResult) {
    (
        ToolCall {
            call_id: Some("call_b254".to_string()),
            name: "Read".to_string(),
            arguments: serde_json::json!({ "path": "src/main.rs" }),
            plan: None,
        },
        ToolResult {
            tool_name: "Read".to_string(),
            status: "ok".to_string(),
            output: "{\"ok\":true}".to_string(),
            duration_ms: 3,
        },
    )
}

/// C1 + C2 + C4：流被截断 ⇒ 同形重试（≥2 次真实请求），全程不出现同步请求，最终为 Err。
#[test]
fn truncated_followup_stream_retries_in_place_and_never_falls_back_to_sync() {
    // Arrange
    let provider = TruncatingSseProvider::start();
    let manager = ProviderManager::new(provider.selection()).with_policy(fast_fail_policy());
    let request = request_probe();
    let (tool_call, tool_result) = tool_probe();
    let mut accumulated = vec![];
    let mut deltas = vec![];

    // Act
    let result = manager.continue_with_tool_result_stream(
        &request,
        &[],
        &mut accumulated,
        None,
        &tool_call,
        &tool_result,
        |delta| deltas.push(delta),
    );
    let bodies = provider.recorded_bodies();

    // Assert：C1 —— 没有任何一条同步（stream:false）请求。
    let sync_requests = bodies
        .iter()
        .filter(|body| body.contains("\"stream\":false") || body.contains("\"stream\": false"))
        .count();
    assert_eq!(
        sync_requests, 0,
        "B254-C1：流式失败后不得回退同步 buffered 通道；实际请求体 = {bodies:?}"
    );

    // Assert：C2 —— 断流（error decoding response body / unexpected eof）必须被判可重试。
    assert!(
        bodies.len() >= 2,
        "B254-C2：被截断的流必须同形重试（≥2 次真实请求）；实际 = {}；请求体 = {bodies:?}",
        bodies.len()
    );

    // Assert：C4 —— 不得把失败合成为「本地兜底摘要」并返回 Ok。
    let error = result.expect_err("B254-C4：流式路径最终失败必须返回 Err，不得合成正文冒充成功");
    assert!(
        !error.contains("兜底"),
        "B254-C4/C5：错误文本不得含合成摘要话术；实际 = {error}"
    );

    // 补充事实：两次请求都必须是流式形态（证明重试走的是同一通道，而不是换了个通道重来）。
    let stream_requests = bodies
        .iter()
        .filter(|body| body.contains("\"stream\":true") || body.contains("\"stream\": true"))
        .count();
    assert_eq!(
        stream_requests,
        bodies.len(),
        "B254-C1：重试必须仍是流式请求；请求体 = {bodies:?}"
    );
}
