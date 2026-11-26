//! wave-60 C4 黑盒验收（独立测试者，红相先写）：provider 读通道**无 chunk 期间可中断**。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C4。
//! 判据：`stop_turn` 被接受后，读循环必须在 `stop_poll_tick_ms(250) + 250ms` 内退出；
//! 本机 CI 硬上界 **≤ 1000ms**（避免抖动误判）；退出沿用既有取消码
//! （`ProviderStreamGuard::CANCELLED_ERROR_CODE`），**不得**新增对外错误码；
//! 正常长流（持续 chunk）不得因 tick 机制被切断。
//!
//! 缺陷面：`block_on(stream.next())` 在**无 chunk 到达**时不可中断（只有 30min 总时长上界），
//! 因此本文件用「只发响应头、随后完全静默」的本地 TCP SSE 服务造真实停滞读，
//! 并把被测调用放进工作线程 + `recv_timeout` 上界观测 —— 用例**永不挂死**，
//! 超时即判红（现状：取消标志永远观测不到）。

use pony_agent_core::agent::config::{ProviderModelCapabilities, ThinkingParamPattern};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderClient, ProviderManager, ProviderMessage, ProviderProtocol,
    ProviderRequest, ProviderStreamGuard,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// 契约 C4 的本机 CI 硬上界。
const CI_EXIT_BOUND_MS: u64 = 1_000;

/// 停滞 SSE 服务：回 200 + `text/event-stream` 头后**完全静默** `hold` 时长，再关闭。
fn spawn_stalled_sse_server(hold: Duration) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind stalled sse server");
    let port = listener.local_addr().expect("local addr").port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
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
            // 无 chunk 期间：这正是 C4 要求「仍能被取消观测」的窗口。
            std::thread::sleep(hold);
            let _ = stream.write_all(b"data: [DONE]\n\n");
            let _ = stream.flush();
        }
    });

    port
}

/// 正常快速流：10 个 chunk 后 `[DONE]`（用于「tick 机制不得切断正常长流」回归判据）。
fn spawn_burst_sse_server(chunks: usize) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind burst sse server");
    let port = listener.local_addr().expect("local addr").port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let headers = "HTTP/1.1 200 OK\r\n\
                 content-type: text/event-stream\r\n\
                 cache-control: no-cache\r\n\
                 connection: close\r\n\r\n";
            if stream.write_all(headers.as_bytes()).is_err() {
                continue;
            }
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
            }
            let _ = stream.write_all(b"data: [DONE]\n\n");
            let _ = stream.flush();
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

/// C4：NFR 取消观测节奏必须是 250ms（可注入，但缺省值不得漂移）。
#[test]
fn c4_default_stop_poll_tick_is_250ms() {
    // Arrange
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_guard = Arc::clone(&cancel);
    let cancel_check = move || cancel_for_guard.load(Ordering::SeqCst);

    // Act
    let guard = ProviderStreamGuard::with_cancel_check(&cancel_check);

    // Assert
    assert_eq!(
        guard.cancel_tick(),
        Duration::from_millis(250),
        "C4：缺省 stop_poll_tick_ms 必须为 250ms"
    );
    assert!(
        !guard.cancel_requested(),
        "C4：未请求停止时不得谎报已取消"
    );
}

/// C4：**无 chunk 期间**被停止 ⇒ 读循环在 CI 硬上界（1000ms）内以既有取消码退出。
#[test]
fn c4_stalled_stream_exits_with_cancel_code_within_ci_bound() {
    let _rt_guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    // Arrange：服务端只回头、随后静默 3s ⇒ 客户端 `stream.next()` 长时间无 chunk
    let port = spawn_stalled_sse_server(Duration::from_secs(3));
    let manager = ProviderManager::new(selection_for(port));
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = Arc::clone(&cancel);
    let cancel_for_trigger = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel::<Result<(), String>>();

    let started = Instant::now();
    std::thread::spawn(move || {
        // 工作线程必须自带 tokio reactor 上下文（guard 只对当前线程生效）。
        let _worker_rt = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
        let cancel_check = move || cancel_for_worker.load(Ordering::SeqCst);
        let guard = ProviderStreamGuard::with_cancel_check(&cancel_check);
        let outcome =
            manager.decide_with_tools_stream_guarded(&request_probe(), &[], &guard, |_chunk| {});
        let _ = tx.send(outcome.map(|_| ()));
    });
    // 60ms 后等价于「stop_turn 被接受」——读循环必须在无 chunk 期间观测到它。
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        cancel_for_trigger.store(true, Ordering::SeqCst);
    });

    // Act：上界观测（用例永不挂死；超时即为红相证据）
    let observed = rx.recv_timeout(Duration::from_millis(CI_EXIT_BOUND_MS + 500));
    let elapsed = started.elapsed();

    // Assert
    let result = match observed {
        Ok(result) => result,
        Err(error) => panic!(
            "C4：无 chunk 期间请求停止后，读循环必须在 {CI_EXIT_BOUND_MS}ms 内退出；\
             实际 {}ms 仍在阻塞（{error}）——现状 block_on(stream.next()) 不可中断，\
             stop_turn 的取消标志在无 chunk 期间永不被观测",
            elapsed.as_millis()
        ),
    };
    assert!(
        elapsed < Duration::from_millis(CI_EXIT_BOUND_MS),
        "C4：退出时延必须 ≤ {CI_EXIT_BOUND_MS}ms（tick 250ms + 250ms 语义），实际 {}ms",
        elapsed.as_millis()
    );
    let error = result.expect_err("C4：被停止的读循环必须以错误（取消码）终止，而不是返回成功");
    assert!(
        error.contains(ProviderStreamGuard::CANCELLED_ERROR_CODE),
        "C4：必须沿用既有取消码 `{}`（不得新增对外错误码），实际 = {error}",
        ProviderStreamGuard::CANCELLED_ERROR_CODE
    );
}

/// C4 回归：有持续 chunk 的正常流不得因 tick/取消观测机制被切断。
#[test]
fn c4_healthy_stream_is_not_truncated_by_cancel_tick() {
    let _rt_guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    // Arrange
    let port = spawn_burst_sse_server(10);
    let manager = ProviderManager::new(selection_for(port));
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = Arc::clone(&cancel);
    let mut deltas = 0usize;

    // Act
    let result = {
        let cancel_check = move || cancel_for_worker.load(Ordering::SeqCst);
        let guard = ProviderStreamGuard::with_cancel_check(&cancel_check);
        manager.decide_with_tools_stream_guarded(&request_probe(), &[], &guard, |_chunk| {
            deltas += 1;
        })
    };

    // Assert
    assert!(
        result.is_ok(),
        "C4 回归：未请求停止的正常流必须完整读完，实际 = {result:?}"
    );
    assert!(
        deltas > 0,
        "C4 回归：正常流必须产生 delta（观测到 {deltas} 个，禁止 tick 机制吞掉 chunk）"
    );
    assert!(
        !cancel.load(Ordering::SeqCst),
        "C4 回归：正常流期间不得自我置位取消"
    );
}
