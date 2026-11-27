use super::{
    normalize_openai_messages, openai_followup_messages_for_tool_results,
    openai_messages_with_tool_result, preview_text, provider_log,
    provider_native_assistant_message_with_reasoning_value,
    provider_native_assistant_tool_call_message_with_reasoning_value,
    provider_native_assistant_tool_calls_message_with_reasoning_value,
    retry_provider_followup_stream_timeout_with_policy, text_if_present, ProviderManager,
    ProviderProtocol, ProviderRequest, ProviderResponse, ProviderStreamChunk, ProviderStreamGuard,
};
use pony_protocol::tool_contract::{ToolCall, ToolDefinition, ToolResult};
use serde_json::Value;

impl ProviderManager {
    /// D2/2026-11-27 修订：批量工具结果 followup（流式入口）。
    ///
    /// 分派顺序：
    /// 1) 长度不匹配 / 空集合 ⇒ 立刻 `Err`（零 HTTP）；
    /// 2) `calls.len() == 1` ⇒ 逐字委派既有 1:1 流式入口；
    /// 3) `!supports_streaming` ⇒ 走同步 `continue_with_tool_results` 并整段回放；
    /// 4) `supports_streaming && N > 1` ⇒ 走真流式 SSE 通道（不回退 45s 同步通道）。
    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    pub fn continue_with_tool_results_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        self.continue_with_tool_results_stream_bounded(
            &ProviderStreamGuard::without_cancel(),
            request,
            tools,
            accumulated_messages,
            assistant_message,
            calls,
            results,
            on_delta,
        )
    }

    /// 带取消守卫的批量工具结果流式 followup 入口。
    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    pub fn continue_with_tool_results_stream_bounded<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        mut on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if calls.len() != results.len() {
            return Err(format!(
                "tool_calls/tool_results length mismatch: calls={} results={}",
                calls.len(),
                results.len()
            ));
        }
        if calls.is_empty() {
            return Err("tool_calls/tool_results must not be empty".to_string());
        }
        if calls.len() == 1 {
            return self.continue_with_tool_result_stream_bounded(
                guard,
                request,
                tools,
                accumulated_messages,
                assistant_message,
                &calls[0],
                &results[0],
                on_delta,
            );
        }

        let request_cow = self.sanitize_request(request);
        let request = request_cow.as_ref();
        if !self.config.capabilities.supports_streaming {
            let response = self.continue_with_tool_results(
                request,
                tools,
                accumulated_messages,
                assistant_message,
                calls,
                results,
            )?;
            replay_sync_response_as_deltas(&response, &mut on_delta);
            return Ok(response);
        }

        self.dispatch_followup_stream_bounded(
            guard,
            request,
            tools,
            accumulated_messages,
            assistant_message,
            calls,
            results,
            on_delta,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "沿用 provider tool followup 既有签名形状"
    )]
    pub(super) fn dispatch_followup_stream_bounded<F>(
        &self,
        guard: &ProviderStreamGuard<'_>,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        accumulated_messages: &mut Vec<Value>,
        assistant_message: Option<&Value>,
        calls: &[ToolCall],
        results: &[ToolResult],
        mut on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if calls.len() == 1 && results.len() == 1 {
            let tool_call = &calls[0];
            let tool_result = &results[0];
            provider_log(format!(
                "followup:start mode=stream requested={} provider={} protocol={} model={} tool={} tool_status={} tool_output_preview={} ",
                self.config.requested_name,
                self.config.provider_name,
                self.protocol_label(),
                request.model,
                tool_call.name,
                tool_result.status,
                preview_text(&tool_result.output, 160)
            ));
        } else {
            provider_log(format!(
                "followup:start mode=stream requested={} provider={} protocol={} model={} calls={} ",
                self.config.requested_name,
                self.config.provider_name,
                self.protocol_label(),
                request.model,
                calls.len()
            ));
        }
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("followup:error {error}"));
            return Err(error);
        }

        match self.config.protocol {
            ProviderProtocol::OpenAiCompletions | ProviderProtocol::OpenAiResponses => {
                // 流式断流/超时等网络瞬态错误允许退避重试；仅在用户显式取消或发出可见文本后放弃。
                let on_delta_cell = std::cell::RefCell::new(on_delta);
                // PA-174 F1 的**流式对偶**：每次尝试前重置为入口时的 pristine 快照，
                // 避免重试向 `accumulated_messages` 尾部重复追加 `assistant + tool`。
                let base_messages = accumulated_messages.clone();
                // 跨 attempt 累计：一旦发出可见 Text 增量即置位，禁止自动重试导致重复上屏。
                let emitted_visible = std::cell::Cell::new(false);
                retry_provider_followup_stream_timeout_with_policy(
                    "followup_stream",
                    &self.policy,
                    || {
                        accumulated_messages.clone_from(&base_messages);
                        let mut delta_sink = on_delta_cell.borrow_mut();
                        let mut record_visible = |chunk: ProviderStreamChunk| {
                            if matches!(chunk, ProviderStreamChunk::Text(_)) {
                                emitted_visible.set(true);
                            }
                            delta_sink(chunk);
                        };
                        match self.config.protocol {
                            ProviderProtocol::OpenAiResponses => self
                                .send_responses_tool_followup_stream_request(
                                    guard,
                                    request,
                                    tools,
                                    accumulated_messages,
                                    assistant_message,
                                    calls,
                                    results,
                                    &mut record_visible,
                                ),
                            _ => self.send_openai_tool_followup_stream_request(
                                guard,
                                request,
                                tools,
                                accumulated_messages,
                                assistant_message,
                                calls,
                                results,
                                &mut record_visible,
                            ),
                        }
                    },
                    &|| guard.cancel_requested() || emitted_visible.get(),
                )
            }
            ProviderProtocol::AnthropicMessages => self
                .send_anthropic_tool_followup_stream_request(
                    guard,
                    request,
                    tools,
                    accumulated_messages,
                    assistant_message,
                    calls,
                    results,
                    &mut on_delta,
                ),
        }
    }
}

/// 将非流式回填响应按 `reasoning -> text` 顺序回放为增量事件（用于 `!supports_streaming` 分支）。
pub(super) fn replay_sync_response_as_deltas<F>(response: &ProviderResponse, on_delta: &mut F)
where
    F: FnMut(ProviderStreamChunk),
{
    if let Some(reasoning_content) = response.reasoning_content.clone() {
        on_delta(ProviderStreamChunk::Reasoning(reasoning_content));
    }
    on_delta(ProviderStreamChunk::Text(response.output_text.clone()));
}

/// 为 OpenAI 流式 followup（1:1 与批量共用）构造标准化消息转录并更新 `accumulated_messages`。
pub(super) fn build_openai_followup_stream_messages(
    request: &ProviderRequest,
    accumulated_messages: &mut Vec<Value>,
    assistant_message: Option<&Value>,
    calls: &[ToolCall],
    results: &[ToolResult],
) -> Result<Vec<Value>, String> {
    if calls.len() == 1 && results.len() == 1 {
        Ok(normalize_openai_messages(openai_messages_with_tool_result(
            request,
            accumulated_messages,
            assistant_message,
            &calls[0],
            &results[0],
        )))
    } else {
        openai_followup_messages_for_tool_results(
            request,
            accumulated_messages,
            assistant_message,
            calls,
            results,
        )
    }
}

/// 按流式响应中的完整 `tool_calls` 列表（无则回落单值）构造 OpenAI native assistant 消息。
pub(super) fn build_openai_stream_assistant_message(
    output_text: &str,
    reasoning_content_value: Option<&Value>,
    tool_call: Option<&ToolCall>,
    tool_calls: &[ToolCall],
) -> Value {
    if !tool_calls.is_empty() {
        provider_native_assistant_tool_calls_message_with_reasoning_value(
            text_if_present(output_text),
            reasoning_content_value,
            tool_calls,
        )
    } else if let Some(call) = tool_call {
        provider_native_assistant_tool_call_message_with_reasoning_value(
            text_if_present(output_text),
            reasoning_content_value,
            call,
        )
    } else {
        provider_native_assistant_message_with_reasoning_value(output_text, reasoning_content_value)
    }
}
