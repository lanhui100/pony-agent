//! W56-A 验收（T9）：provider 流式读循环的**取消观测**与**总时长上界**。
//!
//! 缺陷面（wave-55）：`provider` 的帧循环 `block_on(stream.next())` 内既没有取消检查，也没有
//! 总时长上界 —— 只要上游持续吐字节，连接与 turn 就被永久占用，`stop_turn` 的取消标志不被
//! 观测；长推理流没有任何兜底。
//!
//! 注入缝（实现侧已交付，见 `provider/mod.rs:517` `ProviderStreamGuard`）：
//!   - `ProviderStreamGuard::new(max_elapsed, cancel_tick, Option<&dyn Fn() -> bool>)`
//!   - 错误码常量 `DEADLINE_ERROR_CODE` = `provider_stream_deadline_exceeded`
//!                   `CANCELLED_ERROR_CODE` = `provider_stream_cancelled`
//!
//! 本文件用**本机临时 TCP 服务**造慢速分块 SSE 流，秒级完成、绝不 sleep 长时长：
//!   1. 取消在流中途生效 → 以 `provider_stream_cancelled` 终止，且取消后不再产生任何 delta；
//!   2. 极短 `max_elapsed`（50ms）+ 慢速分块 → 以 `provider_stream_deadline_exceeded` 退出，
//!      且错误信息包含已用时长与已解析字节数（不是静默截断）。

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ThinkingParamPattern,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderClient, ProviderManager, ProviderMessage, ProviderProtocol,
    ProviderRequest, ProviderStreamGuard,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 分块 SSE 服务：每 `gap` 吐一个 `data:` 帧，共 `chunks` 个，最后发 `[DONE]`。
/// 返回监听端口；线程随进程结束，用例结束即断开。
fn spawn_slow_sse_server(chunks: usize, gap: Duration) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local sse server");
    let port = listener.local_addr().expect("local addr").port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // 读完请求头即可（不需要解析 body）
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);

            let headers = "HTTP/1.1 200 OK\r\n\
                 content-type: text/event-stream\r\n\
                 cache-control: no-cache\r\n\
                 connection: close\r\n\r\n";
            if stream.write_all(headers.as_bytes()).is_err() {
                continue;
            }
            let _ = stream.flush();

            for index in 0..chunks {
                let frame = format!(
                    "data: {{\"id\":\"chunk-{index}\",\"object\":\"chat.completion.chunk\",\
                     \"model\":\"guard-probe\",\"choices\":[{{\"index\":0,\
                     \"delta\":{{\"content\":\"piece-{index}\"}},\"finish_reason\":null}}]}}\n\n"
                );
                if stream.write_all(frame.as_bytes()).is_err() {
                    break;
                }
                let _ = stream.flush();
                // 慢速分块：让上限/取消判据有机会在流中途触发
                std::thread::sleep(gap);
            }
            let _ = stream.write_all(b"data: [DONE]\n\n");
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });

    port
}

fn selection_for(port: u16) -> pony_agent_core::agent::config::ResolvedProviderSelection {
    pony_agent_core::agent::config::ResolvedProviderSelection {
        requested_name: "guard-probe".to_string(),
        provider_name: "guard-probe".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: format!("http://127.0.0.1:{port}/v1"),
        auth_type: ProviderAuthType::Bearer,
        api_key_env_var: "PROVIDER_GUARD_PROBE_KEY".to_string(),
        api_key: Some("test-token".to_string()),
        model: "guard-probe".to_string(),
        temperature: 0.0,
        max_output_tokens: 64,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming: true,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    }
}

fn request_probe() -> ProviderRequest {
    ProviderRequest {
        model: "guard-probe".to_string(),
        input: vec![ProviderMessage::user("guard probe")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    }
}

/// (1) 取消在流中途生效：以 `provider_stream_cancelled` 终止，且取消后不再产生 delta。
#[test]
fn cancellation_mid_stream_stops_the_frame_loop() {
    let _rt_guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    // 30 帧 × 20ms ≈ 600ms 的慢速流；取消在第 1 个 delta 到达时触发
    let port = spawn_slow_sse_server(30, Duration::from_millis(20));
    let manager = ProviderManager::new(selection_for(port));

    let cancelled = Arc::new(AtomicBool::new(false));
    let delta_count = Arc::new(AtomicUsize::new(0));
    let cancelled_for_guard = Arc::clone(&cancelled);
    let cancel_check = move || cancelled_for_guard.load(Ordering::SeqCst);
    let guard = ProviderStreamGuard::new(
        ProviderStreamGuard::DEFAULT_MAX_ELAPSED,
        Duration::ZERO, // 零节流：每个 chunk 都观测取消，确定性最高
        Some(&cancel_check as &dyn Fn() -> bool),
    );

    let started = Instant::now();
    let mut observed_after_cancel = 0usize;
    let result = manager.decide_with_tools_stream_guarded(
        &request_probe(),
        &[],
        &guard,
        |chunk| {
            if cancelled.load(Ordering::SeqCst) {
                observed_after_cancel += 1;
            } else {
                delta_count.fetch_add(1, Ordering::SeqCst);
                // 收到第一个 delta 后立刻请求取消（确定性：不依赖 sleep 时序）
                cancelled.store(true, Ordering::SeqCst);
            }
            let _ = chunk;
        },
    );
    let elapsed = started.elapsed();

    let error = result.expect_err(
        "W56-A：流中途请求取消后，帧循环必须以错误终止（现状：取消标志不被观测，循环跑完整个流）",
    );
    assert!(
        error.contains(ProviderStreamGuard::CANCELLED_ERROR_CODE),
        "W56-A：取消必须以 `{}` 终止，实际错误：{error}",
        ProviderStreamGuard::CANCELLED_ERROR_CODE
    );
    assert_eq!(
        observed_after_cancel, 0,
        "W56-A：取消生效后不得再产生任何 delta（观测到 {observed_after_cancel} 个）"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "W56-A：取消必须在流中途秒级生效，实际耗时 {elapsed:?}（禁止跑完 600ms 慢流）"
    );
}

/// (2) 总时长上界：极短 `max_elapsed` + 慢速分块 → 以 `provider_stream_deadline_exceeded` 退出，
/// 且错误信息包含已用时长与已解析字节数（不是静默截断）。
#[test]
fn deadline_upper_bound_exits_with_diagnostics() {
    let _rt_guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    // 40 帧 × 25ms ≈ 1s 的慢速流；上界 50ms 必然在流中途触发
    let port = spawn_slow_sse_server(40, Duration::from_millis(25));
    let manager = ProviderManager::new(selection_for(port));

    let guard = ProviderStreamGuard::new(
        Duration::from_millis(50),
        Duration::from_millis(10),
        None, // 无取消源：只考察总时长上界
    );

    let started = Instant::now();
    let result = manager.decide_with_tools_stream_guarded(&request_probe(), &[], &guard, |_| {});
    let elapsed = started.elapsed();

    let error = result.expect_err(
        "W56-A：超过总时长上界必须以错误退出（现状：无上界，持久吐字节的流会永久占用连接与 turn）",
    );
    assert!(
        error.contains(ProviderStreamGuard::DEADLINE_ERROR_CODE),
        "W56-A：上界超限必须以 `{}` 退出，实际错误：{error}",
        ProviderStreamGuard::DEADLINE_ERROR_CODE
    );
    // 诊断信息：必须带已用时长与已解析字节数，便于定位"是上游太慢还是本地上限过紧"
    let has_elapsed = error.contains("elapsed") || error.contains("已用");
    let has_bytes = error.contains("bytes") || error.contains("字节");
    assert!(
        has_elapsed && has_bytes,
        "W56-A：上界错误必须包含已用时长与已解析字节数（诊断可判定），实际：{error}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "W56-A：上界必须在 50ms 量级退出，实际耗时 {elapsed:?}（禁止 sleep 长时长）"
    );
}

#[allow(dead_code)]
fn _silence_unused(stream: &TcpStream) {
    let _ = stream;
}
