//! ask_resume_commands: C1 —— 宿主流式轮次在**真实 graph run** 内执行（ask-resume 工单）。
//!
//! 根因（`.dev-team/plan-ask-resume.md` §0）：TUI 每轮走 `HostControlPlane::start_turn_stream` →
//! `runtime.start_turn_stream_with_control`，该入口硬编码 `RunTurnFacts::default()` ⇒ 持久化的
//! `PendingControlRequest.run_id = None` ⇒ `bind_ask_wait` 无 run 可绑 ⇒ 用户作答后本轮永久停住。
//!
//! 本模块把该入口升级为「Ask 可恢复」形态：先解析本轮应落在哪个 graph run（可续则复用，否则新建），
//! 再走既有的 `prepare_start_graph_run_stream` / `prepare_continue_graph_run_stream` +
//! `execute_graph_run_stream_step` 单步执行。这样 `RunTurnFacts.run_id` 有真实值，
//! `PendingControlRequest` 带上 run 身份，作业后 `graph_resume_ask` 才能把答案作为**一次性终结
//! 工具结果**注入下一轮。
//!
//! 纪律：
//! - 只做**单步**执行（`execute_graph_run_stream_step`），**不用** `execute_graph_run_stream`
//!   （后者内含 goal 自动续轮循环，会与 TUI 自己的续轮循环叠加）。
//! - 副作用与旧入口对齐：`RecordingTurnEventSink` 终态 trace 注解 + 返回前
//!   `flush_session_buffered_events`（内联在 `execute_graph_run_stream_step` 内，此处再幂等补一次 flush）。
//! - 失败语义：run 解析/准备失败 ⇒ 结构化日志 + 回落旧路径（无 run 事实，交给 C3 的
//!   `ask_resume_unavailable` fail-closed 兜底）；执行失败 ⇒ **只记日志，绝不重跑**（turn 已执行过）。
use super::*;

/// 新建 run 时的兜底 goal：`prepare_start_graph_run_stream` 对空 goal 直接 `Err`，绝不留空串。
const ASK_AWARE_FALLBACK_GOAL: &str = "宿主流式轮次（Ask 可恢复）";

impl HostControlPlane {
    /// C1：Ask 可感知的宿主流式轮入口（`start_turn_stream` 的实现体，签名与语义向后兼容）。
    ///
    /// run 解析规则（最小增长，避免持久化 store 无界膨胀）：取该 `session_id` 下**最近更新**的
    /// 可续 run（非 `Completed/Failed/Cancelled`、`active_turn_id` 为空、无未决 ask wait）复用；
    /// 否则新建一个 run。`session_id == None` 时一律新建。
    pub fn start_ask_aware_turn_stream<S: TurnEventSink>(
        &self,
        sink: &S,
        command: StartTurnStreamCommand,
    ) -> Result<(), String> {
        let session_id_for_flush = command
            .input
            .session_id
            .clone()
            .unwrap_or_else(|| crate::agent::session::DEFAULT_SESSION_ID.to_string());
        // PA-095 #3：入口返回前补提交缓冲事件（终态发射时 sessions 锁可能被 trace 持久化
        // worker 占用而被 try_write 保留；此时锁已释放）。
        match self.prepare_ask_aware_graph_run_stream(&command) {
            Some(prepared) => {
                let run_id = prepared.run_id.clone();
                let turn_id = prepared.turn_id.clone();
                let step = self.execute_graph_run_stream_step(sink, prepared);
                let _ = crate::agent::turn_flow::flush_session_buffered_events(&session_id_for_flush);
                match step {
                    Ok(response) => {
                        crate::agent::otel::log::log_json_with_fields(
                            "debug",
                            "ask-aware turn finished inside a real graph run",
                            None,
                            &[
                                ("run_id", serde_json::json!(run_id)),
                                ("turn_id", serde_json::json!(turn_id)),
                                (
                                    "decision",
                                    serde_json::json!(format!("{:?}", response.decision.kind)),
                                ),
                                (
                                    "run_phase",
                                    serde_json::json!(format!("{:?}", response.run.phase)),
                                ),
                            ],
                        );
                        Ok(())
                    }
                    Err(error) => {
                        // F2：错误必须**回传宿主**（否则 TUI 只 unregister sink、不补发终态，
                        // `is_streaming` 会永久为真、输入与排队派发彻底卡死）。同时：单步执行失败
                        // 意味着 turn **已经跑过**（可能已派发终态事件）⇒ 绝不重跑、绝不回落旧入口。
                        crate::agent::otel::log::log_json_with_fields(
                            "error",
                            "ask-aware graph run step failed after turn execution; not retried",
                            None,
                            &[
                                ("run_id", serde_json::json!(run_id)),
                                ("turn_id", serde_json::json!(turn_id)),
                                ("error", serde_json::json!(error)),
                            ],
                        );
                        Err(error)
                    }
                }
            }
            None => {
                // run 解析/准备失败：落结构化日志后回落旧路径（无 run 事实）。此形态下若模型发起
                // Ask，C3 会把挂起收口成可读失败终态 `ask_resume_unavailable`，不再静默死等。
                crate::agent::otel::log::log_json_with_fields(
                    "warn",
                    "ask-aware graph run unavailable; falling back to legacy no-run turn path",
                    None,
                    &[
                        (
                            "session_id",
                            serde_json::json!(command.input.session_id.clone()),
                        ),
                        ("turn_id", serde_json::json!(command.turn_id.clone())),
                        ("fallback", serde_json::json!("start_turn_stream_legacy")),
                    ],
                );
                self.start_turn_stream_legacy(sink, command);
                // 旧路径自身负责派发终态（含 C3 的 fail-closed 收口），此处没有可回传的错误。
                Ok(())
            }
        }
    }

    /// F1：**把续轮钉死在指定 run 上**（Ask 作答后的续轮专用，不再重新挑选 run）。
    ///
    /// 为什么必须显式传 run：`answer_ask` + `graph_resume_ask` 成功后，待消费的一次性注入
    /// （`take_pending_ask_injection`）只存在于**刚被恢复的那个 run** 上。若此处再按「最近可续 run」
    /// 重选一遍（`start_ask_aware_turn_stream` 的解析规则），一旦期间出现更新的合格 run（例如 goal
    /// 轮次循环在挂起轮返回后立刻以新 run 起一轮），续轮会落到别的 run ⇒ 注入取不到 ⇒ **答案被静默
    /// 丢弃**，正是本工单要根治的症状。
    ///
    /// 失败语义：
    /// - `prepare_continue_graph_run_stream` 失败（run 期间被并发轮占用/已终态/已不存在）⇒ 记 warn
    ///   并**回落到通用 ask-aware 入口** `start_ask_aware_turn_stream`（至少保证这一轮跑完、终态可见），
    ///   本函数此时返回 `Ok(())`（回落路径自身负责派发终态）；
    /// - `execute_graph_run_stream_step` 失败 ⇒ 只留痕并**回传错误**（turn 已执行过，绝不重跑），
    ///   由宿主负责补发终态（TUI 收到 `Err` 即投递 `UiEvent::StreamFailed`，避免 `is_streaming` 永真）。
    ///
    /// 出货路径（唯一调用者）：`crates/pony-agent-core/src/bin/tui.rs` 的 `UiEvent::AskResumeTurn`
    /// 分支——Ask 作答成功（`answer_ask_in_session` + `graph_resume_ask` 均 Ok）后，宿主带着
    /// `graph_resume_ask` 用过的 `run_id` 调本入口跑续轮。其他入口一律不要调用（不要为普通用户消息
    /// 钉 run：普通轮次必须走 `start_ask_aware_turn_stream` 的「最近可续 run」解析规则）。
    pub fn continue_ask_resume_turn_stream<S: TurnEventSink>(
        &self,
        sink: &S,
        run_id: &str,
        command: StartTurnStreamCommand,
    ) -> Result<(), String> {
        let session_id_for_flush = command
            .input
            .session_id
            .clone()
            .unwrap_or_else(|| crate::agent::session::DEFAULT_SESSION_ID.to_string());
        let turn_id = command.turn_id.clone();
        let continue_command = ContinueGraphRunStreamCommand {
            run_id: run_id.to_string(),
            turn_id: turn_id.clone(),
            input: command.input.clone(),
        };
        match self.prepare_continue_graph_run_stream(continue_command) {
            Ok((_, prepared)) => {
                let step = self.execute_graph_run_stream_step(sink, prepared);
                let _ = crate::agent::turn_flow::flush_session_buffered_events(&session_id_for_flush);
                match step {
                    Ok(response) => {
                        crate::agent::otel::log::log_json_with_fields(
                            "debug",
                            "ask-resume continuation finished on the bound run",
                            None,
                            &[
                                ("run_id", serde_json::json!(run_id)),
                                ("turn_id", serde_json::json!(turn_id)),
                                (
                                    "decision",
                                    serde_json::json!(format!("{:?}", response.decision.kind)),
                                ),
                                (
                                    "run_phase",
                                    serde_json::json!(format!("{:?}", response.run.phase)),
                                ),
                            ],
                        );
                        Ok(())
                    }
                    Err(error) => {
                        crate::agent::otel::log::log_json_with_fields(
                            "error",
                            "ask-resume continuation step failed after turn execution; not retried",
                            None,
                            &[
                                ("run_id", serde_json::json!(run_id)),
                                ("turn_id", serde_json::json!(turn_id)),
                                ("error", serde_json::json!(error)),
                            ],
                        );
                        Err(error)
                    }
                }
            }
            Err(error) => {
                crate::agent::otel::log::log_json_with_fields(
                    "warn",
                    "ask-resume bound run is no longer continuable; falling back to the generic ask-aware entry",
                    None,
                    &[
                        ("run_id", serde_json::json!(run_id)),
                        ("turn_id", serde_json::json!(turn_id)),
                        ("error", serde_json::json!(error)),
                    ],
                );
                self.start_ask_aware_turn_stream(sink, command)
            }
        }
    }

    /// 解析并**准备**本轮要落的 graph run（不执行）；返回 `None` 表示必须回落旧路径。
    fn prepare_ask_aware_graph_run_stream(
        &self,
        command: &StartTurnStreamCommand,
    ) -> Option<PreparedGraphRunStream> {
        if let Some(run_id) = self.resolve_reusable_ask_resume_run_id(command.input.session_id.as_deref())
        {
            let continue_command = ContinueGraphRunStreamCommand {
                turn_id: command.turn_id.clone(),
                run_id: run_id.clone(),
                input: command.input.clone(),
            };
            match self.prepare_continue_graph_run_stream(continue_command) {
                Ok((_, prepared)) => return Some(prepared),
                Err(error) => {
                    // 复用的 run 不再可续（并发/竞态）：留痕后按「新建」继续，绝不静默放弃。
                    crate::agent::otel::log::log_json_with_fields(
                        "warn",
                        "ask-aware run reuse failed; creating a fresh graph run instead",
                        None,
                        &[
                            ("run_id", serde_json::json!(run_id)),
                            ("turn_id", serde_json::json!(command.turn_id.clone())),
                            ("error", serde_json::json!(error)),
                        ],
                    );
                }
            }
        }

        // `next_graph_run_id()` is millisecond-based, so two turns prepared inside the same
        // millisecond would mint the same id and the second `prepare_start_graph_run_stream`
        // would fail ("run already exists"). Disambiguate against the live run store.
        let fresh_run_id = {
            let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] graph run lock poisoned");
                e.into_inner()
            });
            let base = next_graph_run_id();
            let mut candidate = base.clone();
            let mut suffix = 1;
            while graph_runs.load_run(&candidate).is_some() {
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
            candidate
        };
        let start_command = StartGraphRunStreamCommand {
            turn_id: command.turn_id.clone(),
            run_id: Some(fresh_run_id),
            goal: ask_aware_goal(&command.input),
            input: command.input.clone(),
        };
        match self.prepare_start_graph_run_stream(start_command) {
            Ok((_, prepared)) => Some(prepared),
            Err(error) => {
                crate::agent::otel::log::log_json_with_fields(
                    "error",
                    "ask-aware graph run preparation failed",
                    None,
                    &[
                        ("turn_id", serde_json::json!(command.turn_id.clone())),
                        ("error", serde_json::json!(error)),
                    ],
                );
                None
            }
        }
    }

    /// 该 `session_id` 下**最近更新**的可续 run id。
    ///
    /// 可续判据（与 `GraphRunner::begin_turn` 的守卫同源，避免"选中了却续不上"）：
    /// 非终态、`active_turn_id` 为空、该 run **没有未决 ask wait**（有未决 wait 的 run 必须经
    /// `graph_resume_ask` 恢复，绝不能被普通轮次抢走）。
    fn resolve_reusable_ask_resume_run_id(&self, session_id: Option<&str>) -> Option<String> {
        let session_id = session_id?;
        let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] graph run lock poisoned");
            e.into_inner()
        });
        // `GraphRunStore::list_runs` 已按 `updated_at_ms` 倒序（最近更新在前）。
        graph_runs
            .list_runs()
            .into_iter()
            .find(|run| {
                run.session_id.as_deref() == Some(session_id)
                    && !matches!(
                        run.phase,
                        GraphRunPhase::Completed
                            | GraphRunPhase::Failed
                            | GraphRunPhase::Cancelled
                    )
                    && run.active_turn_id.is_none()
                    && self
                        .graph_runner
                        .list_ask_waits(&graph_runs, &run.id)
                        .is_empty()
            })
            .map(|run| run.id)
    }

    /// 旧路径（无 run 事实）—— `start_turn_stream` 的历史实现**逐字**搬移，仅函数名变化。
    ///
    /// 它是 C1 的兜底：run 解析/准备失败时保证宿主仍能跑完本轮；Ask 语义由 C3 收口。
    fn start_turn_stream_legacy<S: TurnEventSink>(
        &self,
        sink: &S,
        command: StartTurnStreamCommand,
    ) {
        self.execution_control.register_turn(
            &command.turn_id,
            command.input.session_id.as_deref(),
            None,
        );

        let runtime = self.runtime.read().expect("runtime lock poisoned");
        // PA-095 #3：入口返回前补提交缓冲事件（终态发射时 sessions 锁可能被
        // trace 持久化 worker 占用而被 try_write 保留；此时锁已释放）。
        let session_id_for_flush = command
            .input
            .session_id
            .clone()
            .unwrap_or_else(|| crate::agent::session::DEFAULT_SESSION_ID.to_string());
        // ADR 0074: user message supersedes pending Asks of the session (see `run_turn`).
        self.note_user_message_entry(&command.input);
        // 审核 P0：普通 streaming 路径补 terminal trace annotation（与 graph
        // streaming 路径一致）——终态信封（event_id/sequence 等）写回 trace 记录，
        // 否则 trace 缓存行 watermark 停留 0，无法可靠判断缓存是否覆盖事件流。
        let recording_sink = RecordingTurnEventSink::new(sink);
        runtime.start_turn_stream_with_control(
            &recording_sink,
            &self.execution_control,
            command.turn_id,
            command.input,
        );
        if let Some((
            session_id,
            turn_id,
            event_id,
            event_type,
            event_version,
            sequence,
            emitted_at_ms,
        )) = recording_sink.terminal_trace_annotation()
        {
            let _ = runtime.annotate_turn_trace_terminal_event(
                session_id.as_deref(),
                &turn_id,
                event_id,
                event_type,
                event_version,
                sequence,
                emitted_at_ms,
            );
        }
        drop(runtime);
        let _ = crate::agent::turn_flow::flush_session_buffered_events(&session_id_for_flush);
    }
}

/// 新建 run 的 goal：优先 `display_message`，其次 `message`，trim 后为空则用固定兜底串
/// （`prepare_start_graph_run_stream` 拒绝空 goal）。
fn ask_aware_goal(input: &TurnInput) -> String {
    input
        .display_message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let message = input.message.trim();
            if message.is_empty() {
                None
            } else {
                Some(message)
            }
        })
        .map(str::to_string)
        .unwrap_or_else(|| ASK_AWARE_FALLBACK_GOAL.to_string())
}

#[cfg(test)]
mod ask_resume_fallback_tests {
    use super::*;

    use crate::agent::config::{
        ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
        ThinkingParamPattern,
    };
    use crate::agent::provider::{ProviderAuthType, ProviderProtocol};
    use crate::agent::runtime::AgentRuntimeBuilder;
    use crate::agent::session::SessionStore;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// 记录型 sink：同时保留事件名与载荷（`turn:completed` 的正文要逐字对拍）。
    #[derive(Default)]
    struct RecordingSink {
        captured: Mutex<Vec<(String, TurnStreamEvent)>>,
    }

    impl RecordingSink {
        fn names(&self) -> Vec<String> {
            self.captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .map(|(name, _)| name.clone())
                .collect()
        }

        fn events_named(&self, name: &str) -> Vec<TurnStreamEvent> {
            self.captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .filter(|(captured, _)| captured == name)
                .map(|(_, payload)| payload.clone())
                .collect()
        }
    }

    impl TurnEventSink for RecordingSink {
        fn emit(&self, name: &str, payload: TurnStreamEvent) {
            self.captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((name.to_string(), payload));
        }
    }

    /// 终态事件名（本轮只允许出现**一个**终态）。
    const TERMINAL_EVENTS: [&str; 3] = ["turn:completed", "turn:failed", "turn:suspended"];

    fn terminal_events(sink: &RecordingSink) -> Vec<String> {
        sink.names()
            .into_iter()
            .filter(|name| TERMINAL_EVENTS.contains(&name.as_str()))
            .collect()
    }

    /// 本地回环 mock provider：任何请求都回同一段 assistant 文本，并记录请求次数。
    ///
    /// 为什么必须本地 mock：这些用例要跑**真实 turn**（判据是「回落路径真的执行了一轮」），
    /// 而 `HostControlPlaneBuilder` 未注入 resolver 时会走默认 `ProviderRegistryStore` ——
    /// 单测就会去连用户配置里的真实网关（慢、非确定、烧 token，且违反「单测不出网」）。
    struct MockProviderServer {
        base_url: String,
        address: String,
        requests: Arc<Mutex<Vec<String>>>,
        shutdown: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl MockProviderServer {
        fn start(reply_text: &str) -> Self {
            let listener =
                TcpListener::bind("127.0.0.1:0").expect("mock provider 必须能绑定回环端口");
            let address = listener.local_addr().expect("mock provider 地址");
            let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let shutdown = Arc::new(AtomicBool::new(false));
            let response_body = json!({
                "choices": [{ "message": { "role": "assistant", "content": reply_text } }],
                "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
            })
            .to_string();
            let requests_for_thread = Arc::clone(&requests);
            let shutdown_for_thread = Arc::clone(&shutdown);

            let handle = std::thread::spawn(move || loop {
                let Ok((mut stream, _)) = listener.accept() else {
                    if shutdown_for_thread.load(Ordering::SeqCst) {
                        return;
                    }
                    continue;
                };
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    return;
                }
                let request_body = read_http_request_body(&mut stream);
                requests_for_thread
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(request_body);
                let payload = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    response_body.as_bytes().len(),
                    response_body
                );
                let _ = stream.write_all(payload.as_bytes());
                let _ = stream.flush();
            });

            Self {
                base_url: format!("http://{address}/v1"),
                address: address.to_string(),
                requests,
                shutdown,
                handle: Some(handle),
            }
        }

        fn request_count(&self) -> usize {
            self.requests
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .len()
        }
    }

    impl Drop for MockProviderServer {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::SeqCst);
            // 用一次空连接唤醒阻塞中的 accept，令 mock 线程及时退出。
            let _ = TcpStream::connect(self.address.clone());
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn read_http_request_body(stream: &mut TcpStream) -> String {
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
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

    struct StaticProviderResolver {
        selection: ResolvedProviderSelection,
    }

    impl ProviderSelectionResolver for StaticProviderResolver {
        fn resolve_provider_selection(
            &self,
            _provider_id: Option<&str>,
            _model_id: Option<&str>,
        ) -> ResolvedProviderSelection {
            self.selection.clone()
        }
    }

    /// 非流式（`supports_streaming = false`）：决策与回落轮都走同步 JSON，mock 只需回一段文本。
    fn provider_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "ask-resume-fallback-test".to_string(),
            provider_name: "ask-resume-fallback-test".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url,
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "ASK_RESUME_FALLBACK_TEST_API_KEY".to_string(),
            api_key: Some("ask-resume-fallback-test-key".to_string()),
            model: "ask-resume-fallback-test-model".to_string(),
            temperature: 0.0,
            max_output_tokens: 256,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(8_192),
                supports_tools: true,
                supports_streaming: false,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        }
    }

    /// 隔离控制面：内存会话库 + 本地 mock provider + 内存 graph run store（零外部 IO）。
    fn control_plane_with_mock_provider(
        base_url: String,
        workspace: &std::path::Path,
    ) -> HostControlPlane {
        let runtime = AgentRuntimeBuilder::new()
            .session_store(SessionStore::memory_only())
            .provider_resolver(Box::new(StaticProviderResolver {
                selection: provider_selection(base_url),
            }))
            .workspace_root(workspace)
            .build();
        HostControlPlaneBuilder::new()
            .runtime(runtime)
            .graph_run_store(GraphRunStore::new())
            .build()
    }

    /// 用例独占工作区根（Ask 只做治理判定，不落业务文件）。
    fn isolated_workspace(test_name: &str) -> std::path::PathBuf {
        let unique = format!(
            "pony-agent-ask-fallback-ws-{}-{}-{}",
            std::process::id(),
            test_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).expect("create isolated workspace dir");
        dir
    }

    fn fallback_turn_input(session_id: &str, message: &str) -> TurnInput {
        TurnInput {
            message: message.to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            workspace_mode: None,
            tool_authorization_mode: None,
            session_id: Some(session_id.to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        }
    }

    /// prepare 阶段专用输入（只解析 run、不跑 turn ⇒ 不需要 provider）。
    fn ask_turn_input(session_id: Option<&str>, message: &str) -> TurnInput {
        TurnInput {
            session_id: session_id.map(str::to_string),
            ..fallback_turn_input(session_id.unwrap_or("anonymous"), message)
        }
    }

    fn runs_snapshot(control_plane: &HostControlPlane) -> Vec<(String, Option<String>, String)> {
        let graph_runs = control_plane
            .graph_runs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        graph_runs
            .list_runs()
            .into_iter()
            .map(|run| (run.id, run.session_id, format!("{:?}", run.phase)))
            .collect()
    }

    fn seed_ready_run(
        control_plane: &HostControlPlane,
        run_id: &str,
        session_id: Option<&str>,
    ) {
        let run = crate::agent::graph::GraphEngine::new("state-machine-v1").start_run(
            run_id,
            "seeded run for ask-aware reuse",
            session_id,
        );
        let mut graph_runs = control_plane.graph_runs.lock().unwrap_or_else(|error| {
            error.into_inner()
        });
        control_plane.graph_runner.start_run(&mut graph_runs, run);
    }

    /// C1 分支 A（run 复用）：该会话存在「最近更新的可续 run」（非终态、无 active turn、无未决
    /// ask wait）时，ask-aware 准备必须**复用**它，而不是每轮新建 run（避免持久化 store 无界膨胀）。
    #[test]
    fn prepare_reuses_the_latest_continuable_run_of_the_session() {
        let control_plane = HostControlPlaneBuilder::new()
            .graph_run_store(GraphRunStore::new())
            .build();
        seed_ready_run(&control_plane, "run-reuse-seed", Some("session-reuse-unit"));

        let prepared = control_plane
            .prepare_ask_aware_graph_run_stream(&StartTurnStreamCommand {
                turn_id: "turn-ask-reuse-unit-1".to_string(),
                input: ask_turn_input(Some("session-reuse-unit"), "复用最近可续 run 单测"),
            })
            .expect("ask-aware prepare must find the seeded continuable run");

        assert_eq!(
            prepared.run_id, "run-reuse-seed",
            "C1：同会话存在可续 run 时必须复用，而不是新建"
        );
        let run_count = {
            let graph_runs = control_plane.graph_runs.lock().unwrap_or_else(|error| {
                error.into_inner()
            });
            graph_runs.list_runs().len()
        };
        assert_eq!(run_count, 1, "C1：复用分支不得额外新建 run");
    }

    /// C1 分支 B（新建 run）：无可续 run 时必须新建一个；且 `session_id == None` 时**一律新建**
    /// （两次准备得到两个不同 run id），不得复用任意既有 run。
    #[test]
    fn prepare_creates_a_fresh_run_when_none_is_continuable() {
        let control_plane = HostControlPlaneBuilder::new()
            .graph_run_store(GraphRunStore::new())
            .build();

        // B1：空 store + 具名会话 ⇒ 新建。
        let first = control_plane
            .prepare_ask_aware_graph_run_stream(&StartTurnStreamCommand {
                turn_id: "turn-ask-new-unit-1".to_string(),
                input: ask_turn_input(Some("session-new-unit"), "新建 run 单测"),
            })
            .expect("ask-aware prepare must create a run when none is reusable");
        assert!(
            !first.run_id.is_empty(),
            "C1：无可续 run 时必须新建并返回 run id"
        );

        // B2：session_id == None ⇒ 一律新建（即使是同一进程同一平面）。
        let anonymous_first = control_plane
            .prepare_ask_aware_graph_run_stream(&StartTurnStreamCommand {
                turn_id: "turn-ask-new-unit-2".to_string(),
                input: ask_turn_input(None, "匿名会话新建 run 单测"),
            })
            .expect("ask-aware prepare must create a run for an anonymous session");
        let anonymous_second = control_plane
            .prepare_ask_aware_graph_run_stream(&StartTurnStreamCommand {
                turn_id: "turn-ask-new-unit-3".to_string(),
                input: ask_turn_input(None, "匿名会话再次新建 run 单测"),
            })
            .expect("ask-aware prepare must create another run for an anonymous session");
        assert_ne!(
            anonymous_first.run_id, anonymous_second.run_id,
            "C1：session_id == None 时一律新建 run，不得复用"
        );
    }

    /// F1 回落分支不是死代码：指定的 run 不可续（这里用不存在的 run id）时，
    /// `prepare_continue_graph_run_stream` 必然失败 ⇒ 必须回落到通用 ask-aware 入口，
    /// 并且**返回 Ok**（把「run 不可续」当成回落条件，而不是当成 turn 失败上抛）。
    ///
    /// 封闭性（本次收口的关键）：控制面必须注入**本地回环 mock provider** + 内存会话库 +
    /// 临时工作区。否则 `HostControlPlaneBuilder` 会走默认 `ProviderRegistryStore`，单测真的
    /// 去连用户配置里的线上网关（非确定、烧 token、17 个 tool hop 跑 265s）。
    ///
    /// 判据（真实断言，非空跑）：
    /// 1. 返回值是 `Ok(())`；
    /// 2. 回落路径确实执行了一轮 —— sink 收到事件，且 mock provider 真的被请求过；
    /// 3. 本轮以**恰好一个**终态收口，且是 `turn:completed`（mock 只回文本 ⇒ 不挂起、不失败）；
    /// 4. 终态正文逐字等于 mock 的 assistant 文本（证明跑的是真实 turn，不是空壳）；
    /// 5. 回落新建的 run 绑定到本会话，且**没有**留下 `turn:suspended`（不得静默挂起）。
    #[test]
    fn continue_falls_back_when_bound_run_is_not_continuable() {
        const REPLY_TEXT: &str = "回落轮已在本机 mock provider 上完成。";
        const SESSION_ID: &str = "session-ask-resume-fallback-unit";

        let server = MockProviderServer::start(REPLY_TEXT);
        let workspace = isolated_workspace("fallback-unit");
        let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);
        let sink = RecordingSink::default();
        let command = StartTurnStreamCommand {
            turn_id: "turn-ask-resume-fallback-unit-1".to_string(),
            input: fallback_turn_input(SESSION_ID, "续轮兜底单测"),
        };

        let outcome =
            control_plane.continue_ask_resume_turn_stream(&sink, "run-does-not-exist", command);

        assert!(
            outcome.is_ok(),
            "指定的 run 不可续时必须回落到通用入口并返回 Ok，实际 = {outcome:?}"
        );
        let names = sink.names();
        assert!(
            !names.is_empty(),
            "回落路径必须真的执行了一轮并派发事件（否则 fallback 是死代码），实际 = {names:?}"
        );
        assert!(
            server.request_count() >= 1,
            "回落路径必须真的调用过 provider（否则本轮是空壳），实际请求数 = {}",
            server.request_count()
        );

        let terminals = terminal_events(&sink);
        assert_eq!(
            terminals,
            vec!["turn:completed".to_string()],
            "回落轮必须以**恰好一个** turn:completed 收口（mock 只回文本 ⇒ 不挂起/不失败），\
             实际终态序列 = {terminals:?}，全量事件 = {names:?}"
        );
        let completed = sink.events_named("turn:completed");
        assert_eq!(
            completed[0].text.as_deref(),
            Some(REPLY_TEXT),
            "turn:completed 的正文必须逐字等于 mock 的 assistant 文本（证明跑的是真实 turn）"
        );

        let runs = runs_snapshot(&control_plane);
        assert_eq!(
            runs.len(),
            1,
            "回落必须新建**一个** run（指定 run 不存在，不可能复用），实际 = {runs:?}"
        );
        assert_eq!(
            runs[0].1.as_deref(),
            Some(SESSION_ID),
            "回落新建的 run 必须绑定到本会话，实际 = {runs:?}"
        );
        assert!(
            !matches!(runs[0].2.as_str(), "Failed" | "Cancelled"),
            "回落轮已完成 ⇒ run 不得停在失败/取消态，实际 = {runs:?}"
        );
        assert!(
            !names.iter().any(|name| name == "turn:suspended"),
            "回落轮不得留下无终态的 suspended（静默挂起正是本工单要根治的症状），实际 = {names:?}"
        );
    }

    /// F1 回落的第二种触发形态：**指定的 run 存在但正被另一轮占用**（`prepare_continue_graph_run_stream`
    /// 走的是「already has an active turn」分支，而非「not found」分支）。两条分支都必须回落到通用入口，
    /// 不得把"不可续"当成 turn 失败上抛。
    ///
    /// 用 `GraphRunner::begin_turn` 造这个形态：它是公共 API，且与生产里「并发轮占用同一 run」同源。
    #[test]
    fn continue_falls_back_when_bound_run_is_busy_with_another_turn() {
        const REPLY_TEXT: &str = "被占用 run 上的回落轮同样跑通。";
        const SESSION_ID: &str = "session-ask-resume-fallback-busy";

        let server = MockProviderServer::start(REPLY_TEXT);
        let workspace = isolated_workspace("fallback-busy");
        let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);

        // 预置一个「正在跑别的 turn」的 run（同会话，排除「会话不匹配」这条无关失败因）。
        let busy_run_id = "run-busy-seed";
        {
            let run = crate::agent::graph::GraphEngine::new("state-machine-v1").start_run(
                busy_run_id,
                "seeded busy run",
                Some(SESSION_ID),
            );
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            control_plane.graph_runner.start_run(&mut graph_runs, run);
            let lifecycle = control_plane.graph_runner.begin_turn(
                &mut graph_runs,
                busy_run_id,
                "turn-concurrent-occupier",
                Some(SESSION_ID),
            );
            assert!(
                lifecycle.is_some(),
                "预置夹具必须能把 run 推进到 Running（active_turn_id 非空）"
            );
        }

        let sink = RecordingSink::default();
        let command = StartTurnStreamCommand {
            turn_id: "turn-ask-resume-fallback-busy-1".to_string(),
            input: fallback_turn_input(SESSION_ID, "被占用 run 回落单测"),
        };

        let outcome = control_plane.continue_ask_resume_turn_stream(&sink, busy_run_id, command);

        assert!(
            outcome.is_ok(),
            "绑定的 run 被占用时必须回落到通用入口并返回 Ok，实际 = {outcome:?}"
        );
        let terminals = terminal_events(&sink);
        assert_eq!(
            terminals,
            vec!["turn:completed".to_string()],
            "被占用 run 上的回落轮同样必须以**恰好一个** turn:completed 收口，实际 = {terminals:?}，\
             全量事件 = {:?}",
            sink.names()
        );
        let completed = sink.events_named("turn:completed");
        assert_eq!(
            completed[0].text.as_deref(),
            Some(REPLY_TEXT),
            "回落轮正文必须逐字等于 mock 文本"
        );

        let runs = runs_snapshot(&control_plane);
        assert_eq!(
            runs.len(),
            2,
            "被占用的 run 不可复用 ⇒ 回落必须再新建一个 run（共 2 个），实际 = {runs:?}"
        );
        let busy = runs
            .iter()
            .find(|(id, _, _)| id == busy_run_id)
            .unwrap_or_else(|| panic!("预置的 busy run 必须仍在 store 里，实际 = {runs:?}"));
        assert_eq!(
            busy.2, "Running",
            "回落不得抢占/改写被占用 run 的 phase，实际 = {runs:?}"
        );
        assert!(
            runs.iter().any(|(id, session, _)| id != busy_run_id
                && session.as_deref() == Some(SESSION_ID)),
            "回落新建的 run 必须绑定到本会话，实际 = {runs:?}"
        );
        assert!(
            !sink
                .names()
                .iter()
                .any(|name| name == "turn:suspended"),
            "回落轮不得留下无终态的 suspended（静默挂起正是本工单要根治的症状）"
        );
    }
}
