//! provider 请求净化 / 流式读超时 / 冷却窗口的回归用例（从 `provider/tests.rs` 外移，避开单文件规模棘轮）。
//!
//! 断言口径：这些用例锁的是「出网前的请求形状」与「读超时/冷却的判定边界」——
//! 不是实现细节：请求里能否出现图片、读超时是 idle 还是 total、冷却窗口是否短路到不发 HTTP。

use super::*;

    #[test]
    fn sanitize_request_strips_images_for_non_vision_model() {
        let manager = ProviderManager::new(ResolvedProviderSelection {
            requested_name: "test-deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.deepseek.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "deepseek-chat".to_string(),
            temperature: 0.0,
            max_output_tokens: 4096,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_image_input: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        });

        let request = ProviderRequest {
            model: "deepseek-chat".to_string(),
            input: vec![ProviderMessage::user("请看图".to_string())],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,abc".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("test.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: Default::default(),
            temperature: 0.0,
            max_output_tokens: 4096,
        };

        let sanitized = manager.sanitize_request(&request);
        assert!(sanitized.images.is_empty(), "images must be stripped by sanitize_request");

        // 如果模型支持图片，则不剥离
        let vision_manager = ProviderManager::new(ResolvedProviderSelection {
            capabilities: ProviderModelCapabilities {
                supports_image_input: true,
                ..Default::default()
            },
            ..manager.config.clone()
        });
        let vision_sanitized = vision_manager.sanitize_request(&request);
        assert_eq!(vision_sanitized.images.len(), 1, "images must be preserved for vision model");
    }

    /// F7 真实验证：`build_streaming_http_client` 的 180s 语义必须是「读空闲」而不是
    /// 「总 deadline」。用一个小 idle 预算 + 真本地 SSE 服务复现两件事：
    /// ① 总时长 > idle（2.5s vs 1s）但间隙 < idle 的流必须完整读完（旧实现会被总 deadline 截断）；
    /// ② 单次间隙 > idle 的连接必须失败为 timeout（真正卡死仍然有兜底）。
    #[test]
    fn streaming_client_uses_idle_read_timeout_not_total_deadline() {
        fn spawn_dribbling_sse_server(frames: usize, gap: Duration) -> (String, thread::JoinHandle<Vec<u8>>) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind dribbling sse server");
            let address = listener.local_addr().expect("read address");
            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept sse request");
                let mut buf = [0_u8; 4096];
                let mut consumed = Vec::new();
                loop {
                    let read = stream.read(&mut buf).expect("read request");
                    if read == 0 {
                        break;
                    }
                    consumed.extend_from_slice(&buf[..read]);
                    if consumed.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                    )
                    .expect("write headers");
                stream.flush().expect("flush headers");
                for index in 0..frames {
                    // 先睡后写：模拟「首包前后都有静默」的长推理流。
                    thread::sleep(gap);
                    let frame = format!(
                        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"c{index}\"}}}}]}}\n\n"
                    );
                    stream.write_all(frame.as_bytes()).expect("write frame");
                    stream.flush().expect("flush frame");
                }
                consumed
            });
            (format!("http://{}", address), handle)
        }

        let _guard = pony_protocol::runtime_helper::TestRuntimeGuard::new();

        // ① 总耗时 10 × 250ms = 2.5s ≫ idle 1s，但每次间隙只有 250ms → 必须成功。
        let (success_endpoint, success_handle) =
            spawn_dribbling_sse_server(10, Duration::from_millis(250));
        let client = build_streaming_http_client(
            Duration::from_secs(5),
            Duration::from_secs(1),
            &success_endpoint,
        )
        .expect("build streaming client");
        let started = Instant::now();
        let response = block_on(client.get(&success_endpoint).send()).expect("stream must connect");
        assert!(response.status().is_success());
        let body = block_on(async move { response.text().await }).expect("long stream must not be cut by a total deadline");
        let elapsed = started.elapsed();
        assert!(
            elapsed > Duration::from_secs(1),
            "用例必须真的超过 idle 预算才有证明力，实际 {elapsed:?}"
        );
        assert!(
            body.contains("c9"),
            "10 帧必须全部读完（旧的总 deadline 会截断），实际正文={body:?}"
        );
        let _ = success_handle.join();

        // ② 单次静默 1.5s > idle 300ms → 必须 timeout 失败（卡死兜底仍在）。
        let (stalled_endpoint, stalled_handle) =
            spawn_dribbling_sse_server(1, Duration::from_millis(1500));
        let client = build_streaming_http_client(
            Duration::from_secs(5),
            Duration::from_millis(300),
            &stalled_endpoint,
        )
        .expect("build stalled client");
        let error = match block_on(client.get(&stalled_endpoint).send()) {
            Ok(response) => block_on(async move { response.text().await }).expect_err("stalled stream must time out"),
            Err(error) => error,
        };
        assert!(
            error.is_timeout(),
            "180s 语义必须是读空闲超时，卡死的流要报 timeout，实际错误={error:?}"
        );
        let _ = stalled_handle.join();
    }

    // ── PA-171 L3-B：冷却期全 scope 短路（ADVISORY） ────────────────────────────────

    /// L3-B ADVISORY：冷却标记错误在 **Followup** 作用域同样一失败即 Abort（不 sleep、不再发请求）。
    #[test]
    fn followup_cooldown_active_error_aborts_without_sleep() {
        let sleeper = crate::retry::FakeSleeper::new();
        let attempts = std::cell::Cell::new(0);
        let cooldown_error = crate::retry::append_cooldown_active_marker(
            "provider test 处于限流冷却中（quota），预计 60s 后恢复；本次未发起请求；retry_after=60",
        );
        let (result, report): (Result<(), String>, SingleHopRetryReport) =
            retry_provider_scoped_with_report(
                "followup_sync",
                ProviderRetryScope::Followup,
                || {
                    attempts.set(attempts.get() + 1);
                    Err(cooldown_error.clone())
                },
                &sleeper,
                None,
            );

        println!(
            "[L3-B Followup abort] attempts={} slept_ms={}",
            attempts.get(),
            sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed)
        );
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1, "冷却期 Followup 必须只尝试一次（不得 sleep 后重放）");
        assert_eq!(
            sleeper.total_slept.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "冷却期不得发生任何退避 sleep"
        );
        assert_eq!(report.attempts_total, 1);
        assert!(!report.switched_to_rl);
    }

    /// L3-B ADVISORY：冷却窗口内第 2 次调用**0 次 HTTP**（用回环 mock 物理计数）。
    #[test]
    fn cooldown_window_short_circuits_without_http() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind cooldown mock");
        let address = listener.local_addr().expect("read cooldown mock address");
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set cooldown mock nonblocking");
            let mut buffer = [0_u8; 4096];
            while !shutdown_for_thread.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = stream.read(&mut buffer);
                        requests_for_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let body = "{\"error\":{\"type\":\"exceeded_current_quota_error\"}}";
                        let response = format!(
                            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 60\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        });

        let config = ResolvedProviderSelection {
            requested_name: "pa171-cooldown-test".to_string(),
            provider_name: "pa171-cooldown-test".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: format!("http://{address}/v1"),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PA171_COOLDOWN_TEST_API_KEY".to_string(),
            api_key: Some("pa171-dummy".to_string()),
            model: "pa171-cooldown-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 64,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities::default(),
            thinking_param_pattern: ThinkingParamPattern::None,
        };
        let manager = ProviderManager::new(config);
        let endpoint = format!("http://{address}/v1/chat/completions");
        let body = serde_json::json!({});

        let first = manager.post_openai_json(&endpoint, &body);
        assert!(first.is_err(), "首次 429 必须返回 Err");
        let after_first = requests.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(after_first, 1, "首次 429 只应发 1 次 HTTP");

        let second = manager.post_openai_json(&endpoint, &body);
        let after_second = requests.load(std::sync::atomic::Ordering::SeqCst);
        let error = second.expect_err("冷却窗口内必须直接返回可读失败");
        assert_eq!(
            after_second, after_first,
            "冷却窗口内不得再发起 HTTP，实际新增 {} 次",
            after_second - after_first
        );
        println!(
            "[L3-B Followup] http first={} second={} err={}",
            after_first, after_second, error
        );
        assert!(error.contains("冷却"), "冷却失败文案必须可读：{error}");
        assert!(manager.cooldown_remaining().is_some());
        manager.clear_cooldown();

        shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = handle.join();
    }

    // ============================================================
    // B144：工具面动态来源的缓存突变原因
    // ============================================================

    /// wire 串必须与 TS 侧 `PrefixMutationReason` 联合成员一致（`tool_surface_dynamic`），
    /// 且记账接口幂等（同一轮重复调用只留一条）。
    #[test]
    fn tool_surface_dynamic_reason_wire_string_and_idempotent_note() {
        assert_eq!(
            serde_json::to_string(&PrefixMutationReason::ToolSurfaceDynamic)
                .expect("枚举必须可序列化"),
            "\"tool_surface_dynamic\"",
            "wire 串变更必须同步 src/types/runtime.ts 的联合成员"
        );

        let mut observation = ProviderRequestObservation::default();
        observation.note_dynamic_tool_surface(false);
        assert!(
            observation.prefix_mutation_reasons.is_empty(),
            "工具面静态时不得凭空产生突变原因"
        );

        observation.note_dynamic_tool_surface(true);
        observation.note_dynamic_tool_surface(true);
        assert_eq!(
            observation.prefix_mutation_reasons,
            vec![PrefixMutationReason::ToolSurfaceDynamic],
            "同一轮重复记账必须幂等（只留一条）"
        );
    }

    // ============================================================
    // B1795746927199：批量工具回填（N > 1）真流式 SSE 验收
    // ============================================================

    fn spawn_scripted_http_server(
        responses: Vec<String>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind scripted http server");
        let address = listener.local_addr().expect("local addr");
        let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let bodies_clone = std::sync::Arc::clone(&bodies);
        let handle = thread::spawn(move || {
            for raw_response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 4096];
                let mut header_end: Option<usize> = None;
                let mut content_length = 0usize;
                loop {
                    let Ok(read) = stream.read(&mut chunk) else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&chunk[..read]);
                    if header_end.is_none() {
                        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            let headers = String::from_utf8_lossy(&raw[..pos + 4]).to_lowercase();
                            for line in headers.lines() {
                                if let Some(val) = line.strip_prefix("content-length:") {
                                    content_length = val.trim().parse::<usize>().unwrap_or(0);
                                }
                            }
                        }
                    }
                    if let Some(end) = header_end {
                        if raw.len().saturating_sub(end) >= content_length {
                            break;
                        }
                    }
                }
                let body_start = header_end.unwrap_or(raw.len());
                let body = String::from_utf8_lossy(&raw[body_start..]).to_string();
                bodies_clone.lock().expect("lock recorded bodies").push(body);
                let _ = stream.write_all(raw_response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://{address}/v1"), bodies, handle)
    }

    fn batch_stream_selection(
        base_url: String,
        protocol: ProviderProtocol,
        supports_streaming: bool,
    ) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "batch-stream-test".to_string(),
            provider_name: "batch-stream-test".to_string(),
            protocol,
            base_url,
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "BATCH_STREAM_TEST_KEY".to_string(),
            api_key: Some("dummy-key".to_string()),
            model: "test-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 256,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(32_768),
                supports_tools: true,
                supports_streaming,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        }
    }

    fn batch_stream_probe_calls() -> (Vec<ToolCall>, Vec<ToolResult>) {
        let calls = vec![
            ToolCall {
                call_id: Some("call_a".to_string()),
                name: "Search".to_string(),
                arguments: json!({ "query": "alpha" }),
                plan: None,
            },
            ToolCall {
                call_id: Some("call_b".to_string()),
                name: "Read".to_string(),
                arguments: json!({ "path": "Cargo.toml" }),
                plan: None,
            },
        ];
        let results = vec![
            ToolResult {
                tool_name: "Search".to_string(),
                status: "ok".to_string(),
                output: "{\"matches\":1}".to_string(),
                duration_ms: 4,
            },
            ToolResult {
                tool_name: "Read".to_string(),
                status: "ok".to_string(),
                output: "{\"ok\":true}".to_string(),
                duration_ms: 5,
            },
        ];
        (calls, results)
    }

    #[test]
    fn batch_followup_stream_uses_sse_and_preserves_multi_tool_assistant_message() {
        let _guard = pony_protocol::runtime_helper::TestRuntimeGuard::new();
        let sse_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"分析\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"next_1\",\"type\":\"function\",\"function\":{\"name\":\"Read\",\"arguments\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"next_2\",\"type\":\"function\",\"function\":{\"name\":\"Read\",\"arguments\":\"{\\\"path\\\":\\\"b.rs\\\"}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let http_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            sse_body.len(),
            sse_body
        );
        let (base_url, bodies, handle) = spawn_scripted_http_server(vec![http_response]);
        let manager = ProviderManager::new(batch_stream_selection(
            base_url,
            ProviderProtocol::OpenAiCompletions,
            true,
        ));
        let request = ProviderRequest {
            model: "test-model".to_string(),
            input: vec![ProviderMessage::user("并发执行两个工具".to_string())],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: Default::default(),
            temperature: 0.0,
            max_output_tokens: 256,
        };
        let (calls, results) = batch_stream_probe_calls();
        let mut accumulated = Vec::new();
        let mut deltas = Vec::new();

        let response = manager
            .continue_with_tool_results_stream(
                &request,
                &[],
                &mut accumulated,
                None,
                &calls,
                &results,
                |chunk| deltas.push(chunk),
            )
            .expect("N>1 流式 followup 应走 SSE 并成功");
        handle.join().expect("server thread join");

        let recorded = bodies.lock().expect("lock bodies");
        assert_eq!(recorded.len(), 1, "成功路径应恰好发 1 次请求");
        let sent: Value = serde_json::from_str(&recorded[0]).expect("request body json");
        assert_eq!(
            sent.get("stream").and_then(Value::as_bool),
            Some(true),
            "N>1 在 supports_streaming=true 时必须发送 stream:true"
        );
        let sent_messages = sent
            .get("messages")
            .and_then(Value::as_array)
            .expect("messages array");
        assert_eq!(sent_messages.len(), 4, "user + 1 assistant + 2 tool");
        assert_eq!(response.provider_source, "provider_followup_stream");
        assert_eq!(response.tool_calls.len(), 2, "SSE 返回的 2 个 tool_calls 须全保留");
        let next_assistant = response
            .assistant_message
            .as_ref()
            .expect("assistant_message 必须存在");
        let assistant_tool_calls = next_assistant
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("assistant_message.tool_calls 须为数组");
        assert_eq!(
            assistant_tool_calls.len(),
            2,
            "assistant_message 必须保留全部 2 个流式工具调用，否则下一跳回填会丢调用"
        );
        assert_eq!(
            deltas,
            vec![ProviderStreamChunk::Reasoning("分析".to_string())],
            "SSE 应逐块实时回调增量"
        );
    }

    #[test]
    fn batch_followup_stream_retries_truncated_sse_without_polluting_transcript() {
        let _guard = pony_protocol::runtime_helper::TestRuntimeGuard::new();
        let truncated_sse = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"half";
        let truncated_http = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n{truncated_sse}"
        );
        let ok_sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"汇总完成。\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let ok_http = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            ok_sse.len(),
            ok_sse
        );
        let (base_url, bodies, handle) =
            spawn_scripted_http_server(vec![truncated_http, ok_http]);
        let manager = ProviderManager::new(batch_stream_selection(
            base_url,
            ProviderProtocol::OpenAiCompletions,
            true,
        ))
        .with_policy(crate::config::UpstreamPolicy {
            decision_max_attempts: 2,
            retry_budget_ms: 5_000,
            ..Default::default()
        });
        let request = ProviderRequest {
            model: "test-model".to_string(),
            input: vec![ProviderMessage::user("批量重试".to_string())],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: Default::default(),
            temperature: 0.0,
            max_output_tokens: 256,
        };
        let (calls, results) = batch_stream_probe_calls();
        let mut accumulated = Vec::new();
        let mut deltas = Vec::new();

        let response = manager
            .continue_with_tool_results_stream(
                &request,
                &[],
                &mut accumulated,
                None,
                &calls,
                &results,
                |chunk| deltas.push(chunk),
            )
            .expect("断流后重试应恢复成功");
        handle.join().expect("server thread join");

        assert_eq!(response.output_text, "汇总完成。");
        assert_eq!(
            accumulated.len(),
            4,
            "重试不得向 accumulated_messages 重复追加，实际 {}",
            accumulated.len()
        );
        let recorded = bodies.lock().expect("lock bodies");
        assert_eq!(recorded.len(), 2, "应发生 1 次断流重试共 2 次 HTTP");
        for body in recorded.iter() {
            let parsed: Value = serde_json::from_str(body).expect("request body json");
            assert_eq!(parsed.get("stream").and_then(Value::as_bool), Some(true));
        }
    }

    #[test]
    fn batch_followup_stream_anthropic_and_non_streaming_paths() {
        let _guard = pony_protocol::runtime_helper::TestRuntimeGuard::new();
        let anthropic_sse = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Anthropic批量流完成\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let anthropic_http = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            anthropic_sse.len(),
            anthropic_sse
        );
        let (anthropic_url, anthropic_bodies, anthropic_handle) =
            spawn_scripted_http_server(vec![anthropic_http]);
        let anthropic_manager = ProviderManager::new(batch_stream_selection(
            anthropic_url,
            ProviderProtocol::AnthropicMessages,
            true,
        ));
        let request = ProviderRequest {
            model: "claude-test".to_string(),
            input: vec![ProviderMessage::user("批量".to_string())],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: Default::default(),
            temperature: 0.0,
            max_output_tokens: 256,
        };
        let (calls, results) = batch_stream_probe_calls();
        let mut acc = Vec::new();
        let resp = anthropic_manager
            .continue_with_tool_results_stream(&request, &[], &mut acc, None, &calls, &results, |_| {})
            .expect("Anthropic N>1 流式应成功");
        anthropic_handle.join().expect("anthropic server join");
        assert_eq!(resp.provider_source, "provider_followup_stream");
        assert_eq!(resp.output_text, "Anthropic批量流完成");
        let sent_anthropic: Value = serde_json::from_str(&anthropic_bodies.lock().expect("lock")[0])
            .expect("anthropic request json");
        assert_eq!(sent_anthropic.get("stream").and_then(Value::as_bool), Some(true));

        let sync_json = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "非流式回放完成"
                }
            }]
        })
        .to_string();
        let sync_http = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            sync_json.len(),
            sync_json
        );
        let (sync_url, sync_bodies, sync_handle) = spawn_scripted_http_server(vec![sync_http]);
        let sync_manager = ProviderManager::new(batch_stream_selection(
            sync_url,
            ProviderProtocol::OpenAiCompletions,
            false,
        ));
        let mut sync_acc = Vec::new();
        let mut sync_deltas = Vec::new();
        let sync_resp = sync_manager
            .continue_with_tool_results_stream(
                &request,
                &[],
                &mut sync_acc,
                None,
                &calls,
                &results,
                |chunk| sync_deltas.push(chunk),
            )
            .expect("!supports_streaming 走同步回放");
        sync_handle.join().expect("sync server join");
        assert_eq!(sync_resp.provider_source, "provider_followup_sync");
        assert_eq!(sync_deltas, vec![ProviderStreamChunk::Text("非流式回放完成".to_string())]);
        let sent_sync: Value =
            serde_json::from_str(&sync_bodies.lock().expect("lock")[0]).expect("sync request json");
        assert_eq!(sent_sync.get("stream").and_then(Value::as_bool), Some(false));
    }
