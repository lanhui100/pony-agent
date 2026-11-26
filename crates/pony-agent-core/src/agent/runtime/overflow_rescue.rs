// overflow_rescue: 上游返回 UpstreamContextOverflow / RequiresRequestMutation 时的受控溢出自救。
use super::compaction_ops::{
    compact_session_with_retain_override, concurrent_suffix, read_session_snapshot, CompactionMode,
};
use super::{
    provider_event_meta, runtime_log, AgentRuntime, GraphAskResumeInjection, PlannedTurn,
    PreparedTurn, ProviderEventMeta, ProviderManager, SessionStore, TurnInput,
};
use crate::agent::compression::{
    apply_compression, estimate_history_tokens, estimate_native_transcript_tokens,
    trim_native_transcript_to_token_budget, wrap_summary_to_message,
};
use crate::agent::retry::{BackoffConfig, ProviderRetryPolicy};
use crate::agent::session::TurnHistoryMessage;
use serde_json::Value;
use std::sync::{Arc, RwLock};

const FALLBACK_TRIM_SUMMARY_XML: &str =
    "<session_summary>\nContext trimmed automatically after upstream context overflow.\n</session_summary>";

#[derive(Clone, Copy)]
pub(super) enum EmptyInputMode {
    AllowEmpty,
    RejectEmpty,
}

impl EmptyInputMode {
    fn reject_empty(self) -> bool {
        matches!(self, Self::RejectEmpty)
    }
}

/// 复用 `retry.rs` 单一码表判断错误是否属于可压缩自救的上下文/载荷溢出。
pub(super) fn is_context_overflow_error(error: &str) -> bool {
    ProviderRetryPolicy::new(BackoffConfig::default())
        .classify(error)
        .requires_request_mutation()
}

impl AgentRuntime {
    /// 捕获溢出错误后执行一次会话瘦身并重建 `PreparedTurn`；未收缩则返回原错误。
    pub(super) fn rescue_prepared_after_overflow(
        &self,
        input: &TurnInput,
        empty_mode: EmptyInputMode,
        ask_injection: Option<&GraphAskResumeInjection>,
        prepared: &mut PreparedTurn,
        provider_meta: &mut ProviderEventMeta,
        error: String,
    ) -> Result<(), String> {
        match rescue_session_after_context_overflow(
            &self.sessions,
            &prepared.provider,
            input.session_id.as_deref(),
        ) {
            Ok(true) => {
                *prepared = self.prepare_turn(input, empty_mode.reject_empty(), ask_injection)?;
                *provider_meta = provider_event_meta(&prepared.provider);
                Ok(())
            }
            _ => Err(error),
        }
    }

    /// 同步 `plan_turn` 包装：首跳遇 `RequiresRequestMutation` 时自救一次并重规划。
    pub(super) fn plan_turn_with_overflow_rescue(
        &self,
        input: &TurnInput,
        empty_mode: EmptyInputMode,
        ask_injection: Option<&GraphAskResumeInjection>,
        prepared: &mut PreparedTurn,
        provider_meta: &mut ProviderEventMeta,
    ) -> Result<PlannedTurn, String> {
        match self.plan_turn(prepared) {
            Err(error) if is_context_overflow_error(&error) => {
                self.rescue_prepared_after_overflow(
                    input,
                    empty_mode,
                    ask_injection,
                    prepared,
                    provider_meta,
                    error,
                )?;
                self.plan_turn(prepared)
            }
            other => other,
        }
    }
}

fn commit_fallback_trim(
    sessions: &Arc<RwLock<SessionStore>>,
    session_id: &str,
    base_history: &[TurnHistoryMessage],
    base_transcript: &[Value],
    mut target_history: Vec<TurnHistoryMessage>,
    mut target_transcript: Vec<Value>,
    summary_for_log: String,
) -> Result<(), String> {
    let mut store = sessions.write().unwrap_or_else(|e| {
        tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
        e.into_inner()
    });
    let current = store.snapshot_at_readonly(Some(session_id), None, &[]);
    let appended_history = concurrent_suffix(base_history, &current.history, "history")?;
    let appended_transcript = concurrent_suffix(
        base_transcript,
        &current.provider_native_transcript,
        "transcript",
    )?;
    target_history.extend(appended_history);
    target_transcript.extend(appended_transcript);
    store.commit_compaction(
        Some(session_id),
        target_history,
        target_transcript,
        summary_for_log,
    );
    Ok(())
}

/// 捕获 `RequiresRequestMutation` 后的单次受控瘦身入口（`Ok(true)` 表示会话已实质收缩）。
pub(super) fn rescue_session_after_context_overflow(
    sessions: &Arc<RwLock<SessionStore>>,
    provider: &ProviderManager,
    session_id: Option<&str>,
) -> Result<bool, String> {
    let Some(session_id) = session_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(false);
    };
    let initial = read_session_snapshot(sessions, session_id);
    let before_transcript_tokens =
        estimate_native_transcript_tokens(&initial.provider_native_transcript);

    // 1) 优先尝试标准摘要压缩（保留预算收紧为 0 = 只保最后一轮，压掉所有更早轮次）。
    if let Ok(outcome) = compact_session_with_retain_override(
        sessions,
        provider.clone(),
        session_id,
        CompactionMode::Forced,
        Some(0),
    ) {
        if outcome.compacted {
            let after = read_session_snapshot(sessions, session_id);
            let after_transcript_tokens =
                estimate_native_transcript_tokens(&after.provider_native_transcript);
            if before_transcript_tokens == 0
                || after_transcript_tokens < (before_transcript_tokens * 8 / 10).max(1)
            {
                runtime_log(format!(
                    "overflow_rescue:session={session_id} mode=compact_retain_zero \
                     compressed={} transcript_tokens={before_transcript_tokens}->{after_transcript_tokens}",
                    outcome.compressed_count
                ));
                return Ok(true);
            }
        }
    }

    // 2) 单轮超长（`history_too_short`）或摘要调用自身失败：按协议闭合边界缩减原生转录 / 历史。
    let base = read_session_snapshot(sessions, session_id);
    if !base.provider_native_transcript.is_empty() {
        let before_tokens = estimate_native_transcript_tokens(&base.provider_native_transcript);
        let target_tokens = (before_tokens / 2).max(2_048);
        let trimmed =
            trim_native_transcript_to_token_budget(&base.provider_native_transcript, target_tokens);
        let after_tokens = estimate_native_transcript_tokens(&trimmed);
        if trimmed.len() < base.provider_native_transcript.len() || after_tokens < before_tokens {
            commit_fallback_trim(
                sessions,
                session_id,
                &base.history,
                &base.provider_native_transcript,
                base.history.clone(),
                trimmed,
                FALLBACK_TRIM_SUMMARY_XML.to_string(),
            )?;
            runtime_log(format!(
                "overflow_rescue:session={session_id} mode=transcript_trim \
                 transcript_tokens={before_tokens}->{after_tokens}"
            ));
            return Ok(true);
        }
    }

    if base.history.len() > 2 {
        let summary_message = wrap_summary_to_message(FALLBACK_TRIM_SUMMARY_XML);
        let summary_for_log = summary_message.content.clone();
        let before_tokens = estimate_history_tokens(&base.history);
        let compressed = apply_compression(base.history.clone(), summary_message, 0);
        if compressed.compressed_count > 0 {
            let after_tokens = estimate_history_tokens(&compressed.history);
            commit_fallback_trim(
                sessions,
                session_id,
                &base.history,
                &base.provider_native_transcript,
                compressed.history,
                Vec::new(),
                summary_for_log,
            )?;
            runtime_log(format!(
                "overflow_rescue:session={session_id} mode=history_trim \
                 history_tokens={before_tokens}->{after_tokens}"
            ));
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::AgentRuntimeBuilder;
    use crate::agent::config::{
        ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
        ThinkingParamPattern,
    };
    use crate::agent::provider::{ProviderAuthType, ProviderProtocol};
    use crate::agent::runtime_helper::TestRuntimeGuard;
    use crate::agent::session::TurnHistoryMessage;
    use crate::agent::tools::{ToolCall, ToolExecutor, ToolResult};
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex, RwLock};
    use std::thread;

    struct DummyResolver(ResolvedProviderSelection);
    impl ProviderSelectionResolver for DummyResolver {
        fn resolve_provider_selection(
            &self,
            _provider_id: Option<&str>,
            _model_id: Option<&str>,
        ) -> ResolvedProviderSelection {
            self.0.clone()
        }
    }

    struct NoopExecutor;
    impl ToolExecutor for NoopExecutor {
        fn execute(&self, _call: &ToolCall) -> ToolResult {
            ToolResult {
                tool_name: "noop".to_string(),
                status: "ok".to_string(),
                output: String::new(),
                duration_ms: 0,
            }
        }
    }

    fn make_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "overflow-test".to_string(),
            provider_name: "overflow-test".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url,
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "OVERFLOW_TEST_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "deepseek-free".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(1_000_000),
                supports_tools: true,
                supports_streaming: false,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        }
    }

    fn read_http_request_body(stream: &mut std::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut header_end = None;
        let mut content_length = 0usize;
        loop {
            let n = stream.read(&mut chunk).expect("read http chunk");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if header_end.is_none() {
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = Some(pos + 4);
                    let headers = String::from_utf8_lossy(&buf[..pos]);
                    for line in headers.lines() {
                        if let Some((name, val)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                content_length = val.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
            }
            if let Some(end) = header_end {
                if buf.len() >= end + content_length {
                    return String::from_utf8_lossy(&buf[end..end + content_length]).to_string();
                }
            }
        }
        String::new()
    }

    #[test]
    fn rescue_trims_single_giant_turn_native_transcript() {
        let _guard = TestRuntimeGuard::new();
        let sessions = Arc::new(RwLock::new(SessionStore::memory_only()));
        let sid = "rescue-single-turn";
        let mut transcript = vec![json!({ "role": "user", "content": "start giant task" })];
        for i in 0..12 {
            transcript.push(json!({
                "role": "assistant",
                "content": "",
                "reasoning_content": "r".repeat(600),
                "tool_calls": [{
                    "id": format!("call_{i}"),
                    "type": "function",
                    "function": { "name": "Run", "arguments": "{}" }
                }]
            }));
            transcript.push(json!({
                "role": "tool",
                "tool_call_id": format!("call_{i}"),
                "content": "o".repeat(2_000)
            }));
        }
        {
            let mut store = sessions.write().unwrap_or_else(|e| e.into_inner());
            store.commit_compaction(
                Some(sid),
                vec![TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "start giant task".to_string(),
                    ..Default::default()
                }],
                transcript.clone(),
                String::new(),
            );
        }

        let before_tokens = estimate_native_transcript_tokens(&transcript);
        let provider = ProviderManager::new(make_selection("http://127.0.0.1:1/v1".to_string()));
        let rescued = rescue_session_after_context_overflow(&sessions, &provider, Some(sid))
            .expect("single-turn fallback trim must succeed without network");
        assert!(rescued, "must shrink oversized single-turn transcript");

        let after = read_session_snapshot(&sessions, sid);
        let after_tokens = estimate_native_transcript_tokens(&after.provider_native_transcript);
        assert!(after_tokens < before_tokens, "{after_tokens} < {before_tokens}");
        assert_eq!(
            after.provider_native_transcript.first().and_then(|v| v.get("role")).and_then(|v| v.as_str()),
            Some("user")
        );
    }

    #[test]
    fn run_turn_rescues_400_context_overflow_and_retries_with_smaller_payload() {
        let _guard = TestRuntimeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let seen_bodies: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_bodies_srv = Arc::clone(&seen_bodies);

        let handle = thread::spawn(move || {
            // 1) Initial turn call -> 400 context_length_exceeded
            // 2) Compaction summary call -> 200 summary
            // 3) Retried turn call -> 200 final answer
            for step in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let body = read_http_request_body(&mut stream);
                seen_bodies_srv
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(body.len());

                let (status, resp_json) = if step == 0 {
                    (
                        "400 Bad Request",
                        json!({
                            "error": {
                                "message": "This model's maximum context length is 128000 tokens.",
                                "code": "context_length_exceeded"
                            }
                        })
                        .to_string(),
                    )
                } else if body.contains("context compression task") {
                    (
                        "200 OK",
                        json!({
                            "choices": [{
                                "message": {
                                    "role": "assistant",
                                    "content": "<session_summary>\ngoal: recover overflow\n</session_summary>"
                                }
                            }]
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "200 OK",
                        json!({
                            "choices": [{
                                "message": {
                                    "role": "assistant",
                                    "content": "溢出自救成功"
                                }
                            }]
                        })
                        .to_string(),
                    )
                };

                let http = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_json}",
                    resp_json.len()
                );
                stream.write_all(http.as_bytes()).expect("write response");
            }
        });

        let base_url = format!("http://{addr}/v1");
        let runtime = AgentRuntimeBuilder::new()
            .session_store(SessionStore::memory_only())
            .provider_resolver(Box::new(DummyResolver(make_selection(base_url))))
            .tool_executor(Box::new(NoopExecutor))
            .build();
        let sessions = runtime.sessions_handle();
        let sid = "overflow-e2e-session";
        {
            let mut history = Vec::new();
            let mut transcript = Vec::new();
            for i in 0..4 {
                history.push(TurnHistoryMessage {
                    role: "user".to_string(),
                    content: format!("turn {i}"),
                    ..Default::default()
                });
                history.push(TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: format!("reply {i}"),
                    ..Default::default()
                });
                transcript.push(json!({ "role": "user", "content": format!("turn {i}") }));
                transcript.push(json!({
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "z".repeat(4_000),
                    "tool_calls": [{
                        "id": format!("call_{i}"),
                        "type": "function",
                        "function": { "name": "Run", "arguments": "{}" }
                    }]
                }));
                transcript.push(json!({
                    "role": "tool",
                    "tool_call_id": format!("call_{i}"),
                    "content": "d".repeat(6_000)
                }));
                transcript.push(json!({
                    "role": "assistant",
                    "content": format!("reply {i}"),
                    "reasoning_content": format!("final {i}")
                }));
            }
            let mut store = sessions.write().unwrap_or_else(|e| e.into_inner());
            store.commit_compaction(Some(sid), history, transcript, String::new());
        }

        let result = runtime.run_turn(TurnInput {
            session_id: Some(sid.to_string()),
            message: "继续".to_string(),
            display_message: None,
            images: Vec::new(),
            workspace_mode: None,
            tool_authorization_mode: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            node_id: None,
            history: Vec::new(),
            workspace_id: None,
            plan_execution_approval: None,
        });
        handle.join().expect("mock server finished");

        assert_eq!(result.assistant_message, "溢出自救成功");
        let bodies = seen_bodies.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(bodies.len(), 3, "must issue overflowed + summary + retried request");
        assert!(
            bodies[2] < bodies[0],
            "retried payload ({}) must be strictly smaller than initial overflowed payload ({})",
            bodies[2],
            bodies[0]
        );
    }
}
