//! 流式重试防重复 / Retry-After 窗口 / 字节闸门 / 短窗口 429 回归测试（从 `provider/tests.rs` 外移，守住单文件规模棘轮）。

use super::*;

#[test]
fn followup_backoff_honors_upstream_retry_after_window() {
    let sleeper = crate::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0);
    let result: Result<(), String> = retry_provider_scoped_with_sleeper(
        "followup_sync",
        ProviderRetryScope::Followup,
        || {
            let next = attempts.get() + 1;
            attempts.set(next);
            if next == 1 {
                Err(
                    "provider 返回错误状态：503 Service Unavailable；耗时=120ms；\
                     retry_after=8；响应正文：temporarily unavailable"
                        .to_string(),
                )
            } else {
                Ok(())
            }
        },
        &sleeper,
        None,
    );

    assert!(result.is_ok(), "第 2 跳成功，整条链路必须 Ok");
    let slept = sleeper
        .total_slept
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        slept, 8_000,
        "退避必须逐字等于上游窗口 8s（无窗口逻辑时为曲线 500ms）"
    );
    assert_eq!(attempts.get(), 2, "窗口可等待时必须恰好重试一次");
}

#[test]
fn followup_always_mode_waits_through_window_larger_than_budget() {
    use crate::retry::{BackoffConfig, JitterKind, RetryMode};

    let sleeper = crate::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0);
    let config = BackoffConfig {
        initial_delay_ms: 20_000,
        multiplier: 2.0,
        max_delay_ms: 40_000,
        max_retries: 2,
        total_budget_ms: 60_000,
        jitter_kind: JitterKind::None,
    };
    let result: Result<(), String> = retry_provider_scoped_with_sleeper_and_config(
        "followup_sync",
        ProviderRetryScope::Followup,
        config,
        RetryMode::Always,
        || {
            let next = attempts.get() + 1;
            attempts.set(next);
            if next == 1 {
                Err(
                    "provider 返回错误状态：503 Service Unavailable；耗时=80ms；\
                     retry_after=90；响应正文：maintenance"
                        .to_string(),
                )
            } else {
                Ok(())
            }
        },
        &sleeper,
        None,
    );

    assert!(
        result.is_ok(),
        "Always 必须等到窗口到期后成功，实际={result:?}"
    );
    assert_eq!(
        sleeper
            .total_slept
            .load(std::sync::atomic::Ordering::Relaxed),
        90_000,
        "Always 档位的退避必须等于上游窗口 90s（预算 60s 不得把它截断成 Abort）"
    );
    assert_eq!(attempts.get(), 2, "Always 必须真的重试一次");
}

#[test]
fn followup_aborts_fast_when_retry_after_exceeds_budget() {
    let sleeper = crate::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0);
    let result: Result<(), String> = retry_provider_scoped_with_sleeper(
        "followup_sync",
        ProviderRetryScope::Followup,
        || {
            attempts.set(attempts.get() + 1);
            Err(
                "provider 返回错误状态：429 Too Many Requests；耗时=90ms；\
                 retry_after=3600；响应正文：quota window exhausted"
                    .to_string(),
            )
        },
        &sleeper,
        None,
    );

    assert!(result.is_err());
    assert_eq!(
        sleeper
            .total_slept
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "窗口装不进预算时不得 sleep（等不到就别白等）"
    );
    assert_eq!(attempts.get(), 1, "窗口超预算必须首跳即收口");
}

#[test]
fn streaming_followup_retry_does_not_duplicate_accumulated_messages() {
    let rt = tokio::runtime::Runtime::new().expect("create Tokio runtime");
    let _guard = rt.enter();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock sse server");
    let address = listener.local_addr().expect("read mock address");
    let accepted = Arc::new(Mutex::new(0_usize));
    let accepted_for_thread = Arc::clone(&accepted);
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0_u8; 4096];
            let mut consumed = Vec::new();
            loop {
                let read = match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(_) => break,
                };
                consumed.extend_from_slice(&buf[..read]);
                if consumed.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let mut count = accepted_for_thread.lock().expect("lock accepted");
            *count += 1;
            let attempt = *count;
            drop(count);
            if attempt == 1 {
                let body = "{\"error\":{\"message\":\"upstream exploded\"}}";
                let response = format!(
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                continue;
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.write_all(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\r\n\r\n",
            );
            let _ = stream.write_all(b"data: [DONE]\r\n\r\n");
            let _ = stream.flush();
        }
    });

    let config = ResolvedProviderSelection {
        requested_name: "b1795488338107".to_string(),
        provider_name: "b1795488338107".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: format!("http://{address}/v1"),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "B1795488338107_API_KEY".to_string(),
        api_key: Some("dummy".to_string()),
        model: "b1795488338107-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 64,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    };
    let manager = ProviderManager::new(config).with_policy(
        crate::config::UpstreamPolicy {
            decision_max_attempts: 3,
            retry_budget_ms: 5_000,
            ..Default::default()
        },
    );
    let request = ProviderRequest {
        model: "b1795488338107-model".to_string(),
        input: vec![ProviderMessage::user("继续总结工具结果")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: ProviderRequestObservation::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    };
    let tool_call = ToolCall {
        call_id: Some("call_b1795488338107".to_string()),
        name: "workspace.read_file".to_string(),
        arguments: json!({ "path": "src/main.rs" }),
        plan: None,
    };
    let tool_result = ToolResult {
        tool_name: "workspace.read_file".to_string(),
        status: "ok".to_string(),
        output: "fn main() {}".to_string(),
        duration_ms: 3,
    };

    let mut accumulated: Vec<Value> = Vec::new();
    let response = manager
        .continue_with_tool_result_stream(
            &request,
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &mut accumulated,
            None,
            &tool_call,
            &tool_result,
            |_| {},
        )
        .expect("第 2 次尝试必须成功");
    assert_eq!(response.output_text, "done");
    assert_eq!(
        *accepted.lock().expect("lock accepted"),
        2,
        "用例必须真的走到重试（否则断言无证明力）"
    );
    assert_eq!(
        accumulated.len(),
        3,
        "流式重试不得重复追加 assistant+tool（几何膨胀根因），实际转录={}",
        accumulated.len()
    );
    let _ = handle.join();
}

#[test]
fn stream_retry_hint_blocks_retry_after_visible_delta() {
    let rt = tokio::runtime::Runtime::new().expect("create Tokio runtime");
    let _guard = rt.enter();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind truncated sse server");
    let address = listener.local_addr().expect("read mock address");
    let accepted = Arc::new(Mutex::new(0_usize));
    let accepted_for_thread = Arc::clone(&accepted);
    let handle = thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("set truncated sse server nonblocking");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let mut buf = [0_u8; 4096];
                    let mut consumed = Vec::new();
                    loop {
                        let read = match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(read) => read,
                            Err(_) => break,
                        };
                        consumed.extend_from_slice(&buf[..read]);
                        if consumed.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    *accepted_for_thread.lock().expect("lock accepted") += 1;
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 65536\r\nConnection: close\r\n\r\n",
                    );
                    let _ = stream.write_all(
                        b"data: {\"choices\":[{\"delta\":{\"content\":\"half\"}}]}\r\n\r\n",
                    );
                    let _ = stream.flush();
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        }
    });

    let config = ResolvedProviderSelection {
        requested_name: "b1795488338107-b1".to_string(),
        provider_name: "b1795488338107-b1".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: format!("http://{address}/v1"),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "B1795488338107_B1_API_KEY".to_string(),
        api_key: Some("dummy".to_string()),
        model: "b1795488338107-b1-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 64,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    };
    let manager = ProviderManager::new(config).with_policy(
        crate::config::UpstreamPolicy {
            decision_max_attempts: 4,
            retry_budget_ms: 5_000,
            ..Default::default()
        },
    );
    let request = ProviderRequest {
        model: "b1795488338107-b1-model".to_string(),
        input: vec![ProviderMessage::user("继续总结工具结果")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: ProviderRequestObservation::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    };
    let tool_call = ToolCall {
        call_id: Some("call_b1795488338107_b1".to_string()),
        name: "workspace.read_file".to_string(),
        arguments: json!({ "path": "src/main.rs" }),
        plan: None,
    };
    let tool_result = ToolResult {
        tool_name: "workspace.read_file".to_string(),
        status: "ok".to_string(),
        output: "fn main() {}".to_string(),
        duration_ms: 3,
    };

    let mut accumulated: Vec<Value> = Vec::new();
    let mut visible_chunks = 0_usize;
    let result = manager.continue_with_tool_result_stream(
        &request,
        &[ToolDefinition {
            name: "workspace.read_file",
            description: "read file",
            input_schema: json!({ "type": "object" }),
        }],
        &mut accumulated,
        None,
        &tool_call,
        &tool_result,
        |chunk| {
            if matches!(chunk, ProviderStreamChunk::Text(_)) {
                visible_chunks += 1;
            }
        },
    );

    assert!(
        result.is_err(),
        "截断流必须上抛错误（不得伪装成功），实际={result:?}"
    );
    assert_eq!(
        visible_chunks, 1,
        "首跳必须真的发出过可见增量（否则本用例无证明力）"
    );
    assert_eq!(
        *accepted.lock().expect("lock accepted"),
        1,
        "已发出可见增量后断流**不得重试**（重连会把半截正文与新输出重复上屏）"
    );
    let _ = handle.join();
}

#[test]
fn stream_retry_hint_allows_retry_when_no_visible_delta() {
    let sleeper = crate::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0_u32);
    let emitted_visible = std::cell::Cell::new(false);
    let hint = || emitted_visible.get();

    let result: Result<(), String> = retry_provider_scoped_with_sleeper_and_config(
        "followup_stream",
        ProviderRetryScope::Followup,
        crate::retry::BackoffConfig {
            initial_delay_ms: 10,
            multiplier: 2.0,
            max_delay_ms: 40,
            max_retries: 3,
            total_budget_ms: 5_000,
            jitter_kind: crate::retry::JitterKind::None,
        },
        crate::retry::RetryMode::Normal,
        || {
            let next = attempts.get() + 1;
            attempts.set(next);
            if next == 1 {
                Err("error reading a body: unexpected eof".to_string())
            } else {
                Ok(())
            }
        },
        &sleeper,
        Some(&hint),
    );

    assert!(result.is_ok(), "无可见增量时断流必须可重试，实际={result:?}");
    assert_eq!(attempts.get(), 2, "恰好重试一次");
}

#[test]
fn request_body_size_gate_rejects_oversized_body_at_injected_limit() {
    // 上限注入：断言拦截路径不必造 128 MiB 请求体（旧实现用 600 KiB 撞 512 KiB 硬闸）。
    let oversized = "x".repeat(600 * 1024);
    let body = json!({
        "model": "m",
        "messages": [{ "role": "user", "content": oversized }]
    });
    let error = serialize_request_body_with_size_gate_limit(&body, 512 * 1024)
        .expect_err("超注入上限的请求体必须被拦下");
    assert!(
        error.contains("payload too large"),
        "错误文本必须带 `payload too large`，实际={error}"
    );
    assert!(
        error.contains("request_body_bytes="),
        "错误文本必须带真实字节数以便复盘，实际={error}"
    );
}

/// 1M 窗口 80% 压缩线折算的合法请求体（约 3.2M 字符 ≈ 3.2 MB 纯 ASCII）**必须放行**。
///
/// 这是本条修复的反向锁：旧实现 512 KiB 硬闸会把这种完全合法的长会话在本地判死，
/// 请求根本走不到压缩路径。任何人把上限调回 MiB 以下，此用例立刻变红。
#[test]
fn request_body_size_gate_passes_legitimate_long_context_body() {
    let long_context = "x".repeat(3_200_000);
    let body = json!({
        "model": "m",
        "messages": [{ "role": "user", "content": long_context }]
    });
    let bytes = serialize_request_body_with_size_gate(&body)
        .expect("3.2MB 合法长会话请求体必须放行（1M 窗口 80% 线折算量级）");
    assert!(
        bytes.len() > 3_000_000,
        "放行不得改变请求体，实际字节={}",
        bytes.len()
    );
}

#[test]
fn request_body_size_gate_passes_normal_body_with_identical_bytes() {
    let body = json!({ "model": "m", "messages": [{ "role": "user", "content": "hi" }] });
    let bytes = serialize_request_body_with_size_gate(&body).expect("正常体积必须放行");
    assert_eq!(
        bytes,
        serde_json::to_vec(&body).expect("参考序列化"),
        "闸门放行时不得改变请求体字节"
    );
}

/// 含图请求不再单独享受一档上限：同一 `limit` 对纯文本与含图请求一视同仁。
///
/// 旧实现「纯文本 512 KiB / 含图 10 MiB」分档会把纯文本长会话提前判死，而 base64 图像
/// 天然是 MiB 量级，两档口径互不重叠。现统一为网关量级单档。
#[test]
fn request_body_size_gate_applies_one_limit_regardless_of_images() {
    let text_only = json!({ "content": "x".repeat(600 * 1024) });
    assert!(
        serialize_request_body_with_size_gate_limit(&text_only, 1024 * 1024).is_ok(),
        "600KiB 纯文本在 1MiB 单档下必须放行"
    );

    let with_image = json!({
        "content": [
            { "type": "text", "text": "x".repeat(600 * 1024) },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
        ]
    });
    assert!(
        serialize_request_body_with_size_gate_limit(&with_image, 1024 * 1024).is_ok(),
        "含图请求使用同一档上限"
    );
    assert!(
        serialize_request_body_with_size_gate_limit(&with_image, 1024).is_err(),
        "含图请求超同一档上限时同样必须拦截（不再有 10MiB 特例）"
    );
}

#[test]
fn followup_short_window_429_waits_then_retries() {
    let sleeper = crate::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0_u32);
    let rate_limited = crate::retry::append_cooldown_active_marker(
        "provider 返回错误状态：429 Too Many Requests；retry_after=8；响应正文：slow down",
    );
    let (result, report): (Result<(), String>, SingleHopRetryReport) =
        retry_provider_scoped_with_report_config(
            "followup_sync",
            ProviderRetryScope::Followup,
            crate::retry::BackoffConfig {
                initial_delay_ms: 500,
                multiplier: 2.0,
                max_delay_ms: 8_000,
                max_retries: 3,
                total_budget_ms: 60_000,
                jitter_kind: crate::retry::JitterKind::None,
            },
            crate::retry::RetryMode::Normal,
            || {
                let next = attempts.get() + 1;
                attempts.set(next);
                if next == 1 {
                    Err(rate_limited.clone())
                } else {
                    Ok(())
                }
            },
            &sleeper,
            None,
        );

    assert!(
        result.is_ok(),
        "短窗口 429 必须等过冷却后重试成功，实际={result:?}"
    );
    assert_eq!(attempts.get(), 2, "恰好重试一次");
    let slept = sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        slept,
        8_000 + crate::retry::COOLDOWN_EXPIRY_SLACK_MS,
        "必须睡满「上游窗口 + 抖动余量」再重试"
    );
    assert_eq!(report.attempts_total, 2);
}

#[test]
fn followup_long_window_429_still_short_circuits() {
    let sleeper = crate::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0_u32);
    let rate_limited = crate::retry::append_cooldown_active_marker(
        "provider 返回错误状态：429 Too Many Requests；retry_after=90；响应正文：slow down",
    );
    let (result, report): (Result<(), String>, SingleHopRetryReport) =
        retry_provider_scoped_with_report_config(
            "followup_sync",
            ProviderRetryScope::Followup,
            crate::retry::BackoffConfig::default(),
            crate::retry::RetryMode::Normal,
            || {
                attempts.set(attempts.get() + 1);
                Err(rate_limited.clone())
            },
            &sleeper,
            None,
        );

    assert!(result.is_err());
    assert_eq!(attempts.get(), 1, "长窗口必须短路");
    assert_eq!(
        sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(report.attempts_total, 1);
}

#[test]
fn followup_short_circuit_429_never_waits() {
    let sleeper = crate::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0_u32);
    let short_circuit = crate::retry::append_cooldown_marker(
        "provider test 处于限流冷却中，预计 8s 后恢复；本次未发起请求；retry_after=8",
        crate::retry::CooldownMarkerSource::ShortCircuit,
    );
    let (result, _report): (Result<(), String>, SingleHopRetryReport) =
        retry_provider_scoped_with_report_config(
            "followup_sync",
            ProviderRetryScope::Followup,
            crate::retry::BackoffConfig::default(),
            crate::retry::RetryMode::Normal,
            || {
                attempts.set(attempts.get() + 1);
                Err(short_circuit.clone())
            },
            &sleeper,
            None,
        );

    assert!(result.is_err());
    assert_eq!(attempts.get(), 1, "发送前短路必须保持立即 Abort");
    assert_eq!(
        sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}
