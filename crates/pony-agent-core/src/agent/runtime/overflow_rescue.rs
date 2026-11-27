// overflow_rescue: 上游返回 UpstreamContextOverflow / RequiresRequestMutation 时的受控溢出自救。
use super::compaction_ops::{
    compact_session_with_retain_override, concurrent_suffix, read_session_snapshot, CompactionMode,
};
use super::{
    build_context_observation, provider_event_meta, runtime_log, AgentRuntime,
    BuildContextObservation, GraphAskResumeInjection, PlannedTurn, PreparedTurn,
    ProviderEventMeta, ProviderManager, ProviderRequest, SessionStore, TurnInput,
};
use crate::agent::compression::{
    apply_compression, estimate_history_tokens, estimate_native_message_tokens,
    estimate_native_transcript_tokens, is_session_summary_message,
    trim_native_transcript_to_token_budget, wrap_summary_to_message,
};
use crate::agent::retry::{BackoffConfig, ProviderRetryPolicy};
use crate::agent::session::TurnHistoryMessage;
use crate::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use serde_json::{json, Value};
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

/// 从 followup 响应里提取溢出类错误（覆盖 `provider_failure_message` 与 OpenAI 1:1
/// `local_tool_followup_fallback_response` 的 `provider_mode == "fallback"`）。
pub(super) fn followup_overflow_error(
    response: &crate::agent::provider::ProviderResponse,
) -> Option<String> {
    if let Some(error) = super::provider_failure_message(
        &response.provider_mode,
        response.fallback_reason.as_deref(),
    ) {
        if is_context_overflow_error(&error) {
            return Some(error);
        }
        return None;
    }
    if response.provider_mode == "fallback" {
        if let Some(reason) = response.fallback_reason.as_deref() {
            if is_context_overflow_error(reason) {
                return Some(reason.to_string());
            }
        }
    }
    None
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

    /// ToolFollowup 阶段的受控溢出自救：紧缩会话历史、重跑 `prepare_turn` 刷新
    /// `planning_request`，并基于新请求与本轮前置已闭合批次重建干净 `accumulated_messages`。
    #[expect(clippy::too_many_arguments, reason = "followup rescue updates planning request, meta, observation, and accumulated messages together")]
    pub(super) fn rescue_followup_after_overflow(
        &self,
        input: &TurnInput,
        empty_mode: EmptyInputMode,
        ask_injection: Option<&GraphAskResumeInjection>,
        prior_batches: &[(Option<Value>, Vec<ToolCall>, Vec<ToolResult>)],
        provider: &ProviderManager,
        planning_request: &mut ProviderRequest,
        provider_meta: &mut ProviderEventMeta,
        context_observation: &mut BuildContextObservation,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        error: String,
    ) -> Result<(), String> {
        match rescue_session_after_context_overflow(
            &self.sessions,
            provider,
            input.session_id.as_deref(),
        ) {
            Ok(true) => {
                let rebuilt =
                    self.prepare_turn(input, empty_mode.reject_empty(), ask_injection)?;
                *planning_request = rebuilt.planning_request;
                *provider_meta = provider_event_meta(&rebuilt.provider);
                *context_observation = build_context_observation(planning_request, tools);
                if prior_batches.is_empty() {
                    accumulated_messages.clear();
                } else {
                    *accumulated_messages = rebuilt
                        .provider
                        .rebuild_followup_messages_for_batches(planning_request, prior_batches)?;
                }
                Ok(())
            }
            _ => Err(error),
        }
    }

    #[expect(clippy::too_many_arguments, reason = "sync followup loop wraps provider dispatch and single overflow rescue")]
    pub(super) fn run_sync_tool_followup_with_rescue(
        &self,
        input: &TurnInput,
        ask_injection: Option<&GraphAskResumeInjection>,
        prior_batches: &[(Option<Value>, Vec<ToolCall>, Vec<ToolResult>)],
        provider: &ProviderManager,
        planning_request: &mut ProviderRequest,
        active_provider_meta: &mut ProviderEventMeta,
        context_observation: &mut BuildContextObservation,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        current_assistant_message: Option<&Value>,
        executed_calls: &[ToolCall],
        executed_results: &[ToolResult],
        followup_overflow_rescued: &mut bool,
    ) -> Result<
        (
            crate::agent::provider::ProviderResponse,
            crate::agent::retry::PureCallWindow,
        ),
        (String, bool),
    > {
        loop {
            super::tool_recovery::maybe_soft_trim_accumulated_followup_messages(
                provider,
                accumulated_messages,
            );
            let started_at = crate::agent::retry::begin_provider_attempt_window();
            let followup_result = if executed_calls.len() == 1 {
                super::provider_followup_with_mode(
                    provider,
                    planning_request,
                    tools,
                    accumulated_messages,
                    current_assistant_message,
                    &executed_calls[0],
                    &executed_results[0],
                    crate::agent::turn_flow::LlmRequestMode::ToolFollowup,
                )
            } else {
                super::provider_followup_with_calls(
                    provider,
                    planning_request,
                    tools,
                    accumulated_messages,
                    current_assistant_message,
                    executed_calls,
                    executed_results,
                    crate::agent::turn_flow::LlmRequestMode::ToolFollowup,
                )
            };
            let window = crate::agent::retry::take_pure_call_window(started_at, None);
            match followup_result {
                Ok(response) => {
                    if let Some(error) = followup_overflow_error(&response) {
                        if !*followup_overflow_rescued
                            && self
                                .rescue_followup_after_overflow(
                                    input,
                                    EmptyInputMode::AllowEmpty,
                                    ask_injection,
                                    prior_batches,
                                    provider,
                                    planning_request,
                                    active_provider_meta,
                                    context_observation,
                                    tools,
                                    accumulated_messages,
                                    error.clone(),
                                )
                                .is_ok()
                        {
                            *followup_overflow_rescued = true;
                            continue;
                        }
                        if *followup_overflow_rescued {
                            return Err((error, true));
                        }
                    }
                    return Ok((response, window));
                }
                Err(error) => {
                    if !*followup_overflow_rescued
                        && is_context_overflow_error(&error)
                        && self
                            .rescue_followup_after_overflow(
                                input,
                                EmptyInputMode::AllowEmpty,
                                ask_injection,
                                prior_batches,
                                provider,
                                planning_request,
                                active_provider_meta,
                                context_observation,
                                tools,
                                accumulated_messages,
                                error.clone(),
                            )
                            .is_ok()
                    {
                        *followup_overflow_rescued = true;
                        continue;
                    }
                    return Err((error, false));
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments, reason = "stream followup loop wraps streaming dispatch and single overflow rescue")]
    pub(super) fn run_stream_tool_followup_with_rescue<F>(
        &self,
        control: &super::ExecutionControlRegistry,
        turn_id: &str,
        input: &TurnInput,
        ask_injection: Option<&GraphAskResumeInjection>,
        prior_batches: &[(Option<Value>, Vec<ToolCall>, Vec<ToolResult>)],
        provider: &ProviderManager,
        planning_request: &mut ProviderRequest,
        active_provider_meta: &mut ProviderEventMeta,
        context_observation: &mut BuildContextObservation,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        followup_overflow_rescued: &mut bool,
        mut run_once: F,
    ) -> Result<crate::agent::provider::ProviderResponse, (String, bool)>
    where
        F: FnMut(
            &ProviderRequest,
            &mut Vec<Value>,
        ) -> (Result<crate::agent::provider::ProviderResponse, String>, bool),
    {
        loop {
            let (followup_result, emitted_visible) = run_once(planning_request, accumulated_messages);
            match followup_result {
                Ok(response) => {
                    if let Some(error) = followup_overflow_error(&response) {
                        if !*followup_overflow_rescued
                            && !self.should_cancel_turn(control, turn_id)
                            && !emitted_visible
                            && self
                                .rescue_followup_after_overflow(
                                    input,
                                    EmptyInputMode::RejectEmpty,
                                    ask_injection,
                                    prior_batches,
                                    provider,
                                    planning_request,
                                    active_provider_meta,
                                    context_observation,
                                    tools,
                                    accumulated_messages,
                                    error.clone(),
                                )
                                .is_ok()
                        {
                            *followup_overflow_rescued = true;
                            continue;
                        }
                        if *followup_overflow_rescued {
                            return Err((error, true));
                        }
                    }
                    return Ok(response);
                }
                Err(error) => {
                    if !*followup_overflow_rescued
                        && !self.should_cancel_turn(control, turn_id)
                        && !emitted_visible
                        && is_context_overflow_error(&error)
                        && self
                            .rescue_followup_after_overflow(
                                input,
                                EmptyInputMode::RejectEmpty,
                                ask_injection,
                                prior_batches,
                                provider,
                                planning_request,
                                active_provider_meta,
                                context_observation,
                                tools,
                                accumulated_messages,
                                error.clone(),
                            )
                            .is_ok()
                    {
                        *followup_overflow_rescued = true;
                        continue;
                    }
                    return Err((error, false));
                }
            }
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
        let fallback_summary_message = wrap_summary_to_message(FALLBACK_TRIM_SUMMARY_XML);
        let fallback_summary_for_log = fallback_summary_message.content.clone();
        let fallback_summary_native = json!({
            "role": "user",
            "content": fallback_summary_for_log,
        });
        let had_multi_user_turns = base
            .provider_native_transcript
            .iter()
            .filter(|msg| msg.get("role").and_then(Value::as_str) == Some("user"))
            .count()
            > 1;

        let mut trimmed =
            trim_native_transcript_to_token_budget(&base.provider_native_transcript, target_tokens);
        if had_multi_user_turns
            && !trimmed.first().is_some_and(is_session_summary_message)
        {
            let summary_tokens = estimate_native_message_tokens(&fallback_summary_native);
            let tail_budget = target_tokens.saturating_sub(summary_tokens).max(512);
            let tail =
                trim_native_transcript_to_token_budget(&base.provider_native_transcript, tail_budget);
            let mut with_summary = Vec::with_capacity(tail.len() + 1);
            with_summary.push(fallback_summary_native);
            with_summary.extend(tail);
            if estimate_native_transcript_tokens(&with_summary) < before_tokens {
                trimmed = with_summary;
            }
        }
        let target_history = if had_multi_user_turns {
            let compressed = apply_compression(base.history.clone(), fallback_summary_message, 0);
            if compressed.compressed_count > 0 {
                compressed.history
            } else {
                base.history.clone()
            }
        } else {
            base.history.clone()
        };

        let after_tokens = estimate_native_transcript_tokens(&trimmed);
        if trimmed.len() < base.provider_native_transcript.len() || after_tokens < before_tokens {
            commit_fallback_trim(
                sessions,
                session_id,
                &base.history,
                &base.provider_native_transcript,
                target_history,
                trimmed,
                fallback_summary_for_log,
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
        let rescued_snapshot = read_session_snapshot(&sessions, sid);
        assert!(
            rescued_snapshot
                .provider_native_transcript
                .iter()
                .any(is_session_summary_message),
            "compacted/rescued native transcript must retain <session_summary> so native_tool_flow does not lose earlier context"
        );
    }

    fn seed_overflow_session_history(
        sessions: &Arc<RwLock<SessionStore>>,
        sid: &str,
    ) {
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

    fn test_turn_input(sid: &str, message: &str) -> TurnInput {
        TurnInput {
            session_id: Some(sid.to_string()),
            message: message.to_string(),
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
        }
    }

    fn tool_call_choice_response() -> String {
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "need tool",
                    "tool_calls": [{
                        "id": "call_followup_1",
                        "type": "function",
                        "function": {
                            "name": "workspace_list_files",
                            "arguments": "{\"path\":\".\"}"
                        }
                    }]
                }
            }]
        })
        .to_string()
    }

    struct SinkRecorder(Mutex<Vec<(String, super::super::TurnStreamEvent)>>);
    impl super::super::TurnEventSink for SinkRecorder {
        fn emit(&self, name: &str, payload: super::super::TurnStreamEvent) {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((name.to_string(), payload));
        }
    }

    #[test]
    fn sync_tool_followup_rescues_413_overflow_once_and_retries_once() {
        let _guard = TestRuntimeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let seen_bodies: Arc<Mutex<Vec<(usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_srv = Arc::clone(&seen_bodies);

        let handle = thread::spawn(move || {
            for step in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let body = read_http_request_body(&mut stream);
                let is_summary = body.contains("context compression task");
                seen_srv
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((body.len(), is_summary));

                let (status, resp_json) = if step == 0 {
                    ("200 OK", tool_call_choice_response())
                } else if step == 1 {
                    (
                        "413 Payload Too Large",
                        json!({ "error": { "message": "413 Payload Too Large" } }).to_string(),
                    )
                } else if is_summary {
                    (
                        "200 OK",
                        json!({
                            "choices": [{
                                "message": {
                                    "role": "assistant",
                                    "content": "<session_summary>\ngoal: recover followup\n</session_summary>"
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
                                    "content": "followup 自救成功"
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
        let sid = "followup-rescue-sync-413";
        seed_overflow_session_history(&sessions, sid);

        let result = runtime.run_turn(test_turn_input(sid, "请检查目录并汇报"));
        handle.join().expect("mock server finished");

        assert_eq!(result.assistant_message, "followup 自救成功");
        let bodies = seen_bodies.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            bodies.len(),
            4,
            "expected initial + overflowed followup + 1 rescue summary + 1 retried followup"
        );
        assert!(bodies[2].1, "step 2 must be rescue compaction summary");
        assert!(!bodies[3].1, "step 3 must be retried followup");
        assert!(
            bodies[3].0 < bodies[1].0,
            "retried followup payload ({}) must be strictly smaller than overflowed followup ({})",
            bodies[3].0,
            bodies[1].0
        );
    }

    #[test]
    fn sync_tool_followup_400_overflow_rescues_once_and_fails_on_second_overflow() {
        let _guard = TestRuntimeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let summary_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let summary_calls_srv = Arc::clone(&summary_calls);
        let total_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_calls_srv = Arc::clone(&total_calls);

        let handle = thread::spawn(move || {
            for step in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let body = read_http_request_body(&mut stream);
                total_calls_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let is_summary = body.contains("context compression task");
                if is_summary {
                    summary_calls_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }

                let (status, resp_json) = if step == 0 {
                    ("200 OK", tool_call_choice_response())
                } else if is_summary {
                    (
                        "200 OK",
                        json!({
                            "choices": [{
                                "message": {
                                    "role": "assistant",
                                    "content": "<session_summary>\ngoal: compact once\n</session_summary>"
                                }
                            }]
                        })
                        .to_string(),
                    )
                } else {
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
        let sid = "followup-rescue-sync-twice";
        seed_overflow_session_history(&sessions, sid);

        let result = runtime.run_turn(test_turn_input(sid, "继续检查"));
        handle.join().expect("mock server finished");

        assert_eq!(result.phase, "failed");
        assert_eq!(
            summary_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "rescue must run at most once"
        );
        assert_eq!(
            total_calls.load(std::sync::atomic::Ordering::SeqCst),
            4,
            "must stop after 1 rescue + 1 followup retry without looping"
        );
    }

    #[test]
    fn sync_tool_followup_non_overflow_400_does_not_trigger_rescue() {
        let _guard = TestRuntimeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let total_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_calls_srv = Arc::clone(&total_calls);

        let handle = thread::spawn(move || {
            for step in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let _body = read_http_request_body(&mut stream);
                total_calls_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let (status, resp_json) = if step == 0 {
                    ("200 OK", tool_call_choice_response())
                } else {
                    (
                        "400 Bad Request",
                        json!({
                            "error": {
                                "message": "invalid max_tokens: must be >= 1",
                                "code": "invalid_request_error"
                            }
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
        let sid = "followup-non-overflow-400";
        seed_overflow_session_history(&sessions, sid);

        let result = runtime.run_turn(test_turn_input(sid, "检查目录"));
        handle.join().expect("mock server finished");

        assert_eq!(result.provider_mode, "fallback");
        assert!(
            result
                .fallback_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("invalid max_tokens")),
            "expected non-overflow 400 fallback reason, got {:?}",
            result.fallback_reason
        );
        assert_eq!(
            total_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "non-overflow 400 must not trigger summary rescue or followup retry"
        );
    }

    fn make_streaming_selection(base_url: String) -> ResolvedProviderSelection {
        let mut selection = make_selection(base_url);
        selection.capabilities.supports_streaming = true;
        selection
    }

    fn sse_tool_call_response() -> String {
        let chunk = json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "reasoning_content": "need tool",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_followup_1",
                        "type": "function",
                        "function": {
                            "name": "workspace_list_files",
                            "arguments": "{\"path\":\".\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        format!("data: {chunk}\n\ndata: [DONE]\n\n")
    }

    fn sse_text_response(text: &str) -> String {
        let chunk = json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "content": text
                },
                "finish_reason": "stop"
            }]
        });
        format!("data: {chunk}\n\ndata: [DONE]\n\n")
    }

    #[test]
    fn stream_tool_followup_non_overflow_400_fails_without_rescue() {
        let _guard = TestRuntimeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let total_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_calls_srv = Arc::clone(&total_calls);

        let handle = thread::spawn(move || {
            for step in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let _body = read_http_request_body(&mut stream);
                total_calls_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let (status, content_type, body) = if step == 0 {
                    ("200 OK", "text/event-stream", sse_tool_call_response())
                } else {
                    (
                        "400 Bad Request",
                        "application/json",
                        json!({
                            "error": {
                                "message": "invalid max_tokens: must be >= 1",
                                "code": "invalid_request_error"
                            }
                        })
                        .to_string(),
                    )
                };

                let http = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(http.as_bytes()).expect("write response");
            }
        });

        let base_url = format!("http://{addr}/v1");
        let runtime = AgentRuntimeBuilder::new()
            .session_store(SessionStore::memory_only())
            .provider_resolver(Box::new(DummyResolver(make_streaming_selection(base_url))))
            .tool_executor(Box::new(NoopExecutor))
            .build();
        let sessions = runtime.sessions_handle();
        let sid = "followup-stream-non-overflow-400";
        seed_overflow_session_history(&sessions, sid);

        let control = super::super::ExecutionControlRegistry::new();
        let turn_id = "turn-stream-non-overflow-400".to_string();
        control.register_turn(&turn_id, Some(sid), None);
        let sink = SinkRecorder(Mutex::new(Vec::new()));
        runtime.start_turn_stream_with_control(
            &sink,
            &control,
            turn_id,
            test_turn_input(sid, "检查目录"),
        );
        handle.join().expect("mock server finished");

        let events = sink.0.lock().unwrap_or_else(|e| e.into_inner());
        assert!(events.iter().any(|(name, _)| name == "turn:failed"));
        assert_eq!(total_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn stream_tool_followup_rescues_413_once_and_completes() {
        let _guard = TestRuntimeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let seen_bodies: Arc<Mutex<Vec<(usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_srv = Arc::clone(&seen_bodies);

        let handle = thread::spawn(move || {
            for step in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let body = read_http_request_body(&mut stream);
                let is_summary = body.contains("context compression task");
                seen_srv
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((body.len(), is_summary));

                let (status, content_type, resp) = if step == 0 {
                    ("200 OK", "text/event-stream", sse_tool_call_response())
                } else if step == 1 {
                    (
                        "413 Payload Too Large",
                        "application/json",
                        json!({ "error": { "message": "413 Payload Too Large" } }).to_string(),
                    )
                } else if is_summary {
                    (
                        "200 OK",
                        "application/json",
                        json!({
                            "choices": [{
                                "message": {
                                    "role": "assistant",
                                    "content": "<session_summary>\ngoal: recover stream followup\n</session_summary>"
                                }
                            }]
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "200 OK",
                        "text/event-stream",
                        sse_text_response("流式 followup 自救成功"),
                    )
                };

                let http = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp}",
                    resp.len()
                );
                stream.write_all(http.as_bytes()).expect("write response");
            }
        });

        let base_url = format!("http://{addr}/v1");
        let runtime = AgentRuntimeBuilder::new()
            .session_store(SessionStore::memory_only())
            .provider_resolver(Box::new(DummyResolver(make_streaming_selection(base_url))))
            .tool_executor(Box::new(NoopExecutor))
            .build();
        let sessions = runtime.sessions_handle();
        let sid = "followup-rescue-stream-413";
        seed_overflow_session_history(&sessions, sid);

        let control = super::super::ExecutionControlRegistry::new();
        let turn_id = "turn-stream-followup-rescue".to_string();
        control.register_turn(&turn_id, Some(sid), None);
        let sink = SinkRecorder(Mutex::new(Vec::new()));
        runtime.start_turn_stream_with_control(
            &sink,
            &control,
            turn_id,
            test_turn_input(sid, "请流式检查并汇报"),
        );
        handle.join().expect("mock server finished");

        let events = sink.0.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            events.iter().any(|(name, payload)| {
                name == "turn:completed"
                    && payload.text.as_deref() == Some("流式 followup 自救成功")
            }),
            "expected turn:completed with rescued text, got {:?}",
            events.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
        );
        let bodies = seen_bodies.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(bodies.len(), 4);
        assert!(bodies[2].1);
        assert!(bodies[3].0 < bodies[1].0);
    }

    #[test]
    fn stream_tool_followup_does_not_rescue_when_turn_is_cancelled() {
        let _guard = TestRuntimeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let control = Arc::new(super::super::ExecutionControlRegistry::new());
        let turn_id = "turn-stream-followup-cancel".to_string();
        let sid = "followup-rescue-stream-cancel";
        control.register_turn(&turn_id, Some(sid), None);

        let total_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_calls_srv = Arc::clone(&total_calls);
        let control_srv = Arc::clone(&control);
        let turn_id_srv = turn_id.clone();

        let handle = thread::spawn(move || {
            for step in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let _body = read_http_request_body(&mut stream);
                total_calls_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let (status, content_type, resp_json) = if step == 0 {
                    ("200 OK", "text/event-stream", sse_tool_call_response())
                } else {
                    let _ = control_srv.request_stop(
                        &turn_id_srv,
                        crate::agent::stop_escalation::StopReason::User,
                    );
                    (
                        "413 Payload Too Large",
                        "application/json",
                        json!({ "error": { "message": "413 Payload Too Large" } }).to_string(),
                    )
                };

                let http = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_json}",
                    resp_json.len()
                );
                stream.write_all(http.as_bytes()).expect("write response");
            }
        });

        let base_url = format!("http://{addr}/v1");
        let runtime = AgentRuntimeBuilder::new()
            .session_store(SessionStore::memory_only())
            .provider_resolver(Box::new(DummyResolver(make_streaming_selection(base_url))))
            .tool_executor(Box::new(NoopExecutor))
            .build();
        let sessions = runtime.sessions_handle();
        seed_overflow_session_history(&sessions, sid);

        let sink = SinkRecorder(Mutex::new(Vec::new()));
        runtime.start_turn_stream_with_control(
            &sink,
            &control,
            turn_id,
            test_turn_input(sid, "先执行工具再取消"),
        );
        handle.join().expect("mock server finished");

        assert_eq!(
            total_calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "cancelled turn must not trigger rescue summary or followup retry"
        );
    }
}
