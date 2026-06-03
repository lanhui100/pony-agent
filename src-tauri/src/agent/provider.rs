use crate::agent::config::{ProviderReasoningEffort, ResolvedProviderSelection};
use crate::agent::input::TurnInputImage;
use crate::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::io::{BufRead, BufReader};
use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderProtocol {
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderRole {
    System,
    Developer,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMessage {
    pub role: ProviderRole,
    pub content: String,
}

impl ProviderMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ProviderRole::System,
            content: content.into(),
        }
    }

    pub fn developer(content: impl Into<String>) -> Self {
        Self {
            role: ProviderRole::Developer,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ProviderRole::User,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub model: String,
    pub input: Vec<ProviderMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<TurnInputImage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_messages: Vec<Value>,
    #[serde(default)]
    pub observation: ProviderRequestObservation,
    pub temperature: f32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRequestObservation {
    #[serde(default)]
    pub stable_prefix_text: String,
    #[serde(default)]
    pub semi_stable_context_text: String,
    #[serde(default)]
    pub volatile_input_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_mutation_reasons: Vec<PrefixMutationReason>,
}

impl ProviderRequestObservation {
    fn is_empty(&self) -> bool {
        self.stable_prefix_text.is_empty()
            && self.semi_stable_context_text.is_empty()
            && self.volatile_input_text.is_empty()
            && self.prefix_mutation_reasons.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildContextObservation {
    pub request_format: String,
    pub message_count: usize,
    pub image_count: usize,
    pub tool_count: usize,
    pub temperature: f32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub stable_prefix_text: String,
    #[serde(default)]
    pub semi_stable_context_text: String,
    #[serde(default)]
    pub volatile_input_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_mutation_reasons: Vec<PrefixMutationReason>,
    pub request_messages_text: String,
    pub tool_definitions_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefixMutationReason {
    SessionSummaryChanged,
    RunGoalChanged,
    LongTermMemoryChanged,
    ImageNoteChanged,
    TruncationNoteChanged,
    HistoryBoundaryShifted,
    NativeTranscriptBoundaryShifted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub output_text: String,
    pub tool_call: Option<ToolCall>,
    pub reasoning_content: Option<String>,
    pub reasoning_content_value: Option<Value>,
    pub assistant_message: Option<Value>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

pub fn build_context_observation(
    request: &ProviderRequest,
    tools: &[ToolDefinition],
) -> BuildContextObservation {
    let observation = if request.observation.is_empty() {
        derive_request_observation(request)
    } else {
        request.observation.clone()
    };
    let (request_format, request_messages_text, message_count) =
        if !request.native_messages.is_empty() {
            (
                "provider-native".to_string(),
                render_native_messages(&request.native_messages),
                request.native_messages.len(),
            )
        } else {
            (
                "normalized-input".to_string(),
                render_input_messages(&request.input),
                request.input.len(),
            )
        };

    BuildContextObservation {
        request_format,
        message_count,
        image_count: request.images.len(),
        tool_count: tools.len(),
        temperature: request.temperature,
        max_output_tokens: request.max_output_tokens,
        stable_prefix_text: observation.stable_prefix_text,
        semi_stable_context_text: observation.semi_stable_context_text,
        volatile_input_text: observation.volatile_input_text,
        prefix_mutation_reasons: observation.prefix_mutation_reasons,
        request_messages_text,
        tool_definitions_text: render_tool_definitions(tools),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDecision {
    pub output_text: String,
    pub tool_call: Option<ToolCall>,
    pub reasoning_content: Option<String>,
    pub reasoning_content_value: Option<Value>,
    pub assistant_message: Option<Value>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone)]
struct OpenAiStreamMessage {
    output_text: String,
    tool_call: Option<ToolCall>,
    reasoning_content: Option<String>,
    reasoning_content_value: Option<Value>,
    token_usage: Option<TokenUsage>,
}

#[derive(Debug, Default)]
struct OpenAiSseAccumulator {
    output_text: String,
    reasoning_content: String,
    reasoning_content_value: Option<Value>,
    tool_calls: BTreeMap<usize, PartialOpenAiToolCall>,
    token_usage: Option<TokenUsage>,
    saw_data: bool,
}

#[derive(Debug, Default)]
struct PartialOpenAiToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStreamChunk {
    Text(String),
    Reasoning(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[allow(dead_code)]
pub trait ProviderClient {
    fn requested_name(&self) -> &str;
    fn name(&self) -> &str;
    fn model(&self) -> &str;
    fn protocol_label(&self) -> &'static str;
    fn temperature(&self) -> f32;
    fn max_output_tokens(&self) -> u32;
    fn decide_with_tools(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String>;
    fn decide_with_tools_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk);
    fn continue_with_tool_result(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String>;
    fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk);
}

pub struct ProviderManager {
    config: ResolvedProviderSelection,
}

impl ProviderManager {
    pub fn new(config: ResolvedProviderSelection) -> Self {
        Self { config }
    }

    pub fn requested_name(&self) -> &str {
        &self.config.requested_name
    }

    pub fn name(&self) -> &str {
        &self.config.provider_name
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn protocol_label(&self) -> &'static str {
        match self.config.protocol {
            ProviderProtocol::OpenAi => "openai",
            ProviderProtocol::Anthropic => "anthropic",
        }
    }

    pub fn temperature(&self) -> f32 {
        self.config.temperature
    }

    pub fn max_output_tokens(&self) -> u32 {
        self.config.max_output_tokens
    }

    pub fn supports_true_streaming_followup(&self) -> bool {
        if !self.config.capabilities.supports_streaming {
            return false;
        }

        matches!(self.config.protocol, ProviderProtocol::OpenAi)
    }

    pub fn supports_true_streaming_decision(&self) -> bool {
        self.config.capabilities.supports_streaming
            && matches!(self.config.protocol, ProviderProtocol::OpenAi)
    }

    pub fn context_window_tokens(&self) -> Option<u32> {
        self.config.capabilities.context_window_tokens
    }

    pub fn supports_image_input(&self) -> bool {
        self.config.capabilities.supports_image_input
    }

    pub fn requires_provider_native_tool_flow(&self) -> bool {
        matches!(self.config.protocol, ProviderProtocol::OpenAi)
            && self.config.capabilities.supports_reasoning
    }

    pub fn decide_with_tools(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        provider_log(format!(
            "decision:start requested={} provider={} protocol={} model={} tools={} api_key_present={} ",
            self.config.requested_name,
            self.config.provider_name,
            self.protocol_label(),
            request.model,
            tools.len(),
            self.config.api_key.is_some()
        ));
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("decision:error {}", error));
            return Err(error);
        }

        let result = match self.config.protocol {
            ProviderProtocol::OpenAi => self.send_openai_tool_decision_request(request, tools),
            ProviderProtocol::Anthropic => {
                self.send_anthropic_tool_decision_request(request, tools)
            }
        };

        match result {
            Ok(decision) => {
                provider_log(format!(
                    "decision:success mode={} tool_call={} output_preview={} ",
                    decision.provider_mode,
                    decision
                        .tool_call
                        .as_ref()
                        .map(|call| call.name.as_str())
                        .unwrap_or("none"),
                    preview_text(&decision.output_text, 120)
                ));
                Ok(decision)
            }
            Err(error) => {
                provider_log(format!("decision:error {}", error));
                Err(error)
            }
        }
    }

    pub fn decide_with_tools_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        mut on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        provider_log(format!(
            "decision:start mode=stream requested={} provider={} protocol={} model={} tools={} api_key_present={} ",
            self.config.requested_name,
            self.config.provider_name,
            self.protocol_label(),
            request.model,
            tools.len(),
            self.config.api_key.is_some()
        ));
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("decision:error {}", error));
            return Err(error);
        }

        let result = match self.config.protocol {
            ProviderProtocol::OpenAi => {
                self.send_openai_tool_decision_stream_request(request, tools, &mut on_delta)
            }
            ProviderProtocol::Anthropic => self.send_anthropic_tool_decision_request(request, tools),
        };

        match result {
            Ok(decision) => {
                provider_log(format!(
                    "decision:success mode={} tool_call={} output_preview={} ",
                    decision.provider_mode,
                    decision
                        .tool_call
                        .as_ref()
                        .map(|call| call.name.as_str())
                        .unwrap_or("none"),
                    preview_text(&decision.output_text, 120)
                ));
                Ok(decision)
            }
            Err(error) => {
                provider_log(format!("decision:error {}", error));
                Err(error)
            }
        }
    }

    pub fn continue_with_tool_result(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        provider_log(format!(
            "followup:start mode=sync requested={} provider={} protocol={} model={} tool={} tool_status={} tool_output_preview={} ",
            self.config.requested_name,
            self.config.provider_name,
            self.protocol_label(),
            request.model,
            tool_call.name,
            tool_result.status,
            preview_text(&tool_result.output, 160)
        ));
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("followup:error {}", error));
            return Err(error);
        }

        match self.config.protocol {
            ProviderProtocol::OpenAi => self
                .send_openai_tool_followup_request(
                    request,
                    tools,
                    assistant_message,
                    tool_call,
                    tool_result,
                )
                .or_else(|error| {
                    provider_log(format!(
                        "followup:local-fallback protocol=openai provider={} model={} reason={}",
                        self.config.provider_name, request.model, error
                    ));
                    Ok(local_tool_followup_fallback_response(
                        request,
                        tool_call,
                        tool_result,
                        error,
                    ))
                }),
            ProviderProtocol::Anthropic => self.send_anthropic_tool_followup_request(
                request,
                tools,
                assistant_message,
                tool_call,
                tool_result,
            ),
        }
    }

    pub fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        mut on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if !self.config.capabilities.supports_streaming {
            let response = self.continue_with_tool_result(
                request,
                tools,
                assistant_message,
                tool_call,
                tool_result,
            )?;
            if let Some(reasoning_content) = response.reasoning_content.clone() {
                on_delta(ProviderStreamChunk::Reasoning(reasoning_content));
            }
            on_delta(ProviderStreamChunk::Text(response.output_text.clone()));
            return Ok(response);
        }

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
        if self.config.api_key.is_none() {
            let error = format!(
                "provider {} missing API key; please save a key in the model config page (field: {}).",
                self.config.provider_name, self.config.api_key_env_var
            );
            provider_log(format!("followup:error {}", error));
            return Err(error);
        }

        match self.config.protocol {
            ProviderProtocol::OpenAi => match self.send_openai_tool_followup_stream_request(
                request,
                tools,
                assistant_message,
                tool_call,
                tool_result,
                &mut on_delta,
            ) {
                Ok(response) => Ok(response),
                Err(stream_error) => {
                    provider_log(format!(
                        "followup:stream-fallback protocol=openai provider={} model={} reason={}",
                        self.config.provider_name, request.model, stream_error
                    ));
                    let mut response = match self.send_openai_tool_followup_request(
                        request,
                        tools,
                        assistant_message,
                        tool_call,
                        tool_result,
                    ) {
                        Ok(response) => response,
                        Err(sync_error) => {
                            provider_log(format!(
                                "followup:stream-local-fallback protocol=openai provider={} model={} reason={}",
                                self.config.provider_name, request.model, sync_error
                            ));
                            local_tool_followup_fallback_response(
                                request,
                                tool_call,
                                tool_result,
                                sync_error,
                            )
                        }
                    };
                    response.provider_source = "provider_followup_stream_sync_fallback".to_string();
                    response.fallback_reason = Some(match response.fallback_reason.take() {
                        Some(existing) => format!(
                            "stream_followup_failed: {}; {}",
                            preview_text(&stream_error, 160),
                            existing
                        ),
                        None => stream_error,
                    });
                    Ok(response)
                }
            },
            ProviderProtocol::Anthropic => self.send_anthropic_tool_followup_stream_request(
                request,
                tools,
                assistant_message,
                tool_call,
                tool_result,
                &mut on_delta,
            ),
        }
    }

    fn send_openai_tool_followup_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai followup-sync endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let messages = normalize_openai_messages(openai_messages_with_tool_result(
            request,
            assistant_message,
            tool_call,
            tool_result,
        ));
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": false,
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        let payload = self.post_openai_json(&endpoint, &body)?;
        let message = first_openai_message(&payload)?;
        let output_text = extract_openai_message_text(message).unwrap_or_default();
        let tool_call = extract_openai_tool_call(message);
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("openai tool follow-up missing text or tool call".to_string());
        }

        let token_usage = extract_openai_usage(&payload)
            .unwrap_or_else(|| estimate_token_usage(request, &output_text));
        provider_log_token_usage("openai followup-sync", &token_usage);

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_call,
            reasoning_content: extract_openai_message_reasoning_content(message),
            reasoning_content_value: extract_openai_message_reasoning_value(message),
            assistant_message: Some(normalize_openai_assistant_message(message)),
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn send_openai_tool_followup_stream_request<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai followup-stream endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let messages = normalize_openai_messages(openai_messages_with_tool_result(
            request,
            assistant_message,
            tool_call,
            tool_result,
        ));
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": true,
                "stream_options": {
                    "include_usage": true
                },
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);

        self.stream_openai_request(&endpoint, &body, request, on_delta)
    }

    fn send_anthropic_tool_followup_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        _assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic followup-sync endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": anthropic_messages_with_tool_result(request, tool_call, tool_result),
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "tools": anthropic_tools_payload(tools),
                "tool_choice": {
                    "type": "auto",
                    "disable_parallel_tool_use": true
                }
            }),
            &self.config,
        );
        let payload = self.post_anthropic_json(&endpoint, &body)?;

        let output_text = extract_anthropic_output_text(&payload).unwrap_or_default();
        let content = payload
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tool_call = extract_anthropic_tool_call(&content);
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("anthropic tool follow-up missing text or tool call".to_string());
        }
        let assistant_message = match tool_call.as_ref() {
            Some(tool_call) => Some(provider_native_assistant_tool_call_message(
                text_if_present(&output_text),
                None,
                tool_call,
            )),
            None => Some(provider_native_assistant_message(&output_text)),
        };

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_call,
            reasoning_content: None,
            reasoning_content_value: None,
            assistant_message,
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_anthropic_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    fn send_anthropic_tool_followup_stream_request<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        _assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic followup-stream endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": anthropic_messages_with_tool_result(request, tool_call, tool_result),
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": true,
                "tools": anthropic_tools_payload(tools),
                "tool_choice": {
                    "type": "auto",
                    "disable_parallel_tool_use": true
                }
            }),
            &self.config,
        );

        self.stream_anthropic_request(&endpoint, &body, request, on_delta)
    }

    fn stream_openai_request<F>(
        &self,
        endpoint: &str,
        body: &Value,
        request: &ProviderRequest,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let response = self.post_openai_stream_response(endpoint, body)?;
        let message =
            collect_openai_sse_message_from_response(response, Instant::now(), endpoint, on_delta)?;
        let output_text = message.output_text.clone();
        let token_usage = message
            .token_usage
            .clone()
            .unwrap_or_else(|| estimate_token_usage(request, &output_text));
        provider_log_token_usage("openai followup-stream", &token_usage);
        let assistant_message = if let Some(tool_call) = message.tool_call.as_ref() {
            Some(provider_native_assistant_tool_call_message_with_reasoning_value(
                text_if_present(&output_text),
                message.reasoning_content_value.as_ref(),
                tool_call,
            ))
        } else {
            Some(provider_native_assistant_message_with_reasoning_value(
                &output_text,
                message.reasoning_content_value.as_ref(),
            ))
        };

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_call: message.tool_call,
            reasoning_content: message.reasoning_content.clone(),
            reasoning_content_value: message.reasoning_content_value.clone(),
            assistant_message,
            provider_source: "provider_followup_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn post_openai_stream_response(
        &self,
        endpoint: &str,
        body: &Value,
    ) -> Result<reqwest::blocking::Response, String> {
        let client = build_streaming_http_client(
            Duration::from_secs(15),
            Duration::from_secs(180),
            Duration::from_secs(180),
        )?;
        let started_at = Instant::now();

        let response = client
            .post(endpoint)
            .bearer_auth(
                self.config
                    .api_key
                    .as_deref()
                    .ok_or_else(|| "provider 缺少 API Key".to_string())?,
            )
            .json(body)
            .send()
            .map_err(|error| {
                format_request_error("调用 provider 流式接口失败", &error, started_at.elapsed())
            })?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().map_err(|error| {
                format_request_error("读取 provider 流式返回失败", &error, started_at.elapsed())
            })?;

            return Err(format!(
                "provider 返回错误状态：{}；耗时={}ms；响应正文：{}",
                status,
                started_at.elapsed().as_millis(),
                text
            ));
        }

        Ok(response)
    }

    fn stream_anthropic_request<F>(
        &self,
        endpoint: &str,
        body: &Value,
        request: &ProviderRequest,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let payload = self.post_anthropic_json(endpoint, body)?;
        let output_text = extract_anthropic_output_text(&payload).unwrap_or_default();
        let content = payload
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tool_call = extract_anthropic_tool_call(&content);
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("anthropic stream follow-up missing text or tool call".to_string());
        }
        if !output_text.is_empty() {
            on_delta(ProviderStreamChunk::Text(output_text.clone()));
        }
        let assistant_message = match tool_call.as_ref() {
            Some(tool_call) => Some(provider_native_assistant_tool_call_message(
                text_if_present(&output_text),
                None,
                tool_call,
            )),
            None => Some(provider_native_assistant_message(&output_text)),
        };

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            tool_call,
            reasoning_content: None,
            reasoning_content_value: None,
            assistant_message,
            provider_source: "provider_followup_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_anthropic_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    fn send_openai_tool_decision_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai decision endpoint={} model={} safe_tools={}",
            endpoint,
            request.model,
            tools
                .iter()
                .map(|tool| openai_safe_tool_name(tool.name))
                .collect::<Vec<_>>()
                .join(",")
        ));
        let messages = normalize_openai_messages(openai_request_messages(request));
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": false,
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:openai decision messages={} body_chars={} body_preview={}",
            request
                .input
                .iter()
                .map(|message| format!("{:?}:{}", message.role, preview_text(&message.content, 48)))
                .collect::<Vec<_>>()
                .join(" | "),
            body.to_string().chars().count(),
            preview_json(&body, 900)
        ));
        let payload = self.post_openai_json(&endpoint, &body)?;
        let message = first_openai_message(&payload)?;

        let token_usage = extract_openai_usage(&payload).unwrap_or_else(|| {
            estimate_token_usage(
                request,
                extract_openai_message_text(message)
                    .as_deref()
                    .unwrap_or_default(),
            )
        });
        provider_log_token_usage("openai decision", &token_usage);

        Ok(ProviderDecision {
            output_text: extract_openai_message_text(message).unwrap_or_default(),
            tool_call: extract_openai_tool_call(message),
            reasoning_content: extract_openai_message_reasoning_content(message),
            reasoning_content_value: extract_openai_message_reasoning_value(message),
            assistant_message: Some(normalize_openai_assistant_message(message)),
            provider_source: "provider_decision".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn send_openai_tool_decision_stream_request<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: &mut F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai decision-stream endpoint={} model={} safe_tools={}",
            endpoint,
            request.model,
            tools
                .iter()
                .map(|tool| openai_safe_tool_name(tool.name))
                .collect::<Vec<_>>()
                .join(",")
        ));
        let messages = normalize_openai_messages(openai_request_messages(request));
        let body = with_openai_request_options(
            json!({
                "model": request.model,
                "messages": messages,
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "stream": true,
                "stream_options": {
                    "include_usage": true
                },
                "tool_choice": "auto",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:openai decision-stream messages={} body_chars={} body_preview={}",
            request
                .input
                .iter()
                .map(|message| format!("{:?}:{}", message.role, preview_text(&message.content, 48)))
                .collect::<Vec<_>>()
                .join(" | "),
            body.to_string().chars().count(),
            preview_json(&body, 900)
        ));

        let response = self.post_openai_stream_response(&endpoint, &body)?;
        let message =
            collect_openai_sse_message_from_response(response, Instant::now(), &endpoint, on_delta)?;
        let output_text = message.output_text.clone();
        let tool_call = message.tool_call.clone();
        if output_text.trim().is_empty() && tool_call.is_none() {
            return Err("openai streamed decision missing text or tool call".to_string());
        }
        let token_usage = message
            .token_usage
            .clone()
            .unwrap_or_else(|| estimate_token_usage(request, &output_text));
        provider_log_token_usage("openai decision-stream", &token_usage);

        let assistant_message = if let Some(tool_call) = tool_call.as_ref() {
            Some(provider_native_assistant_tool_call_message_with_reasoning_value(
                text_if_present(&output_text),
                message.reasoning_content_value.as_ref(),
                tool_call,
            ))
        } else {
            Some(provider_native_assistant_message_with_reasoning_value(
                &output_text,
                message.reasoning_content_value.as_ref(),
            ))
        };

        Ok(ProviderDecision {
            output_text,
            tool_call,
            reasoning_content: message.reasoning_content,
            reasoning_content_value: message.reasoning_content_value,
            assistant_message,
            provider_source: "provider_decision_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(token_usage),
        })
    }

    fn send_anthropic_tool_decision_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic decision endpoint={} model={} tools={}",
            endpoint,
            request.model,
            tools
                .iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>()
                .join(",")
        ));
        let body = with_anthropic_request_options(
            json!({
                "model": request.model,
                "system": anthropic_system_text(&request.input),
                "messages": anthropic_user_messages(request),
                "temperature": request.temperature,
                "max_tokens": request.max_output_tokens,
                "tools": anthropic_tools_payload(tools),
                "tool_choice": {
                    "type": "auto",
                    "disable_parallel_tool_use": true
                }
            }),
            &self.config,
        );
        let body = apply_anthropic_tool_capability(body, tools, &self.config);
        provider_log(format!(
            "request:anthropic decision messages={} body_chars={} body_preview={}",
            request
                .input
                .iter()
                .map(|message| format!("{:?}:{}", message.role, preview_text(&message.content, 48)))
                .collect::<Vec<_>>()
                .join(" | "),
            body.to_string().chars().count(),
            preview_json(&body, 900)
        ));
        let payload = self.post_anthropic_json(&endpoint, &body)?;
        let content = payload
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| "anthropic 返回中缺少 content".to_string())?;

        let output_text = extract_anthropic_text_blocks(content);
        Ok(ProviderDecision {
            output_text: output_text.clone(),
            tool_call: extract_anthropic_tool_call(content),
            reasoning_content: None,
            reasoning_content_value: None,
            assistant_message: None,
            provider_source: "provider_decision".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_anthropic_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    fn post_openai_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let client = build_http_client(Duration::from_secs(45))?;
        let started_at = Instant::now();

        let response = client
            .post(endpoint)
            .bearer_auth(
                self.config
                    .api_key
                    .as_deref()
                    .ok_or_else(|| "provider 缺少 API Key".to_string())?,
            )
            .json(body)
            .send()
            .map_err(|error| {
                format_request_error("调用 provider 失败", &error, started_at.elapsed())
            })?;

        let status = response.status();
        let text = response.text().map_err(|error| {
            format_request_error("读取 provider 返回失败", &error, started_at.elapsed())
        })?;

        if !status.is_success() {
            return Err(format!(
                "provider 返回错误状态：{}；耗时={}ms；响应正文：{}",
                status,
                started_at.elapsed().as_millis(),
                text
            ));
        }

        serde_json::from_str::<Value>(&text).map_err(|error| {
            format!(
                "解析 provider 返回失败：{}；耗时={}ms；原始响应：{}",
                error,
                started_at.elapsed().as_millis(),
                text
            )
        })
    }

    fn post_anthropic_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let client = build_http_client(Duration::from_secs(45))?;
        let started_at = Instant::now();

        let response = client
            .post(endpoint)
            .header(
                "x-api-key",
                self.config
                    .api_key
                    .as_deref()
                    .ok_or_else(|| "provider 缺少 API Key".to_string())?,
            )
            .header("anthropic-version", "2023-06-01")
            .json(body)
            .send()
            .map_err(|error| {
                format_request_error("调用 provider 失败", &error, started_at.elapsed())
            })?;

        let status = response.status();
        let text = response.text().map_err(|error| {
            format_request_error("读取 provider 返回失败", &error, started_at.elapsed())
        })?;

        if !status.is_success() {
            return Err(format!(
                "provider 返回错误状态：{}；耗时={}ms；响应正文：{}",
                status,
                started_at.elapsed().as_millis(),
                text
            ));
        }

        serde_json::from_str::<Value>(&text).map_err(|error| {
            format!(
                "解析 provider 返回失败：{}；耗时={}ms；原始响应：{}",
                error,
                started_at.elapsed().as_millis(),
                text
            )
        })
    }
}

impl ProviderClient for ProviderManager {
    fn requested_name(&self) -> &str {
        self.requested_name()
    }

    fn name(&self) -> &str {
        self.name()
    }

    fn model(&self) -> &str {
        self.model()
    }

    fn protocol_label(&self) -> &'static str {
        self.protocol_label()
    }

    fn temperature(&self) -> f32 {
        self.temperature()
    }

    fn max_output_tokens(&self) -> u32 {
        self.max_output_tokens()
    }

    fn decide_with_tools(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
    ) -> Result<ProviderDecision, String> {
        ProviderManager::decide_with_tools(self, request, tools)
    }

    fn decide_with_tools_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        on_delta: F,
    ) -> Result<ProviderDecision, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        ProviderManager::decide_with_tools_stream(self, request, tools, on_delta)
    }

    fn continue_with_tool_result(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        ProviderManager::continue_with_tool_result(
            self,
            request,
            tools,
            assistant_message,
            tool_call,
            tool_result,
        )
    }

    fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        assistant_message: Option<&Value>,
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        ProviderManager::continue_with_tool_result_stream(
            self,
            request,
            tools,
            assistant_message,
            tool_call,
            tool_result,
            on_delta,
        )
    }

    /*
     */
    #[cfg(any())]
    fn openai_stream_message_collects_content_and_reasoning() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看文件结构。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第 3 行是 \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"use crate::...\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("stream message should parse");

        assert_eq!(message.output_text, "第 3 行是 use crate::...");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看文件结构。"));
        assert_eq!(
            deltas,
            vec!["第 3 行是 ".to_string(), "use crate::...".to_string()]
        );
    }
    // */
}

fn build_http_client(timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .timeout(timeout)
        .http1_only()
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|error| format!("创建 HTTP client 失败：{}", error))
}

fn build_streaming_http_client(
    connect_timeout: Duration,
    read_timeout: Duration,
    timeout: Duration,
) -> Result<Client, String> {
    let effective_timeout = if read_timeout > timeout {
        read_timeout
    } else {
        timeout
    };
    Client::builder()
        .connect_timeout(connect_timeout)
        .timeout(effective_timeout)
        .http1_only()
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|error| format!("创建流式 HTTP client 失败: {}", error))
}

fn format_request_error(prefix: &str, error: &reqwest::Error, elapsed: Duration) -> String {
    let mut flags = Vec::new();
    if error.is_timeout() {
        flags.push("timeout");
    }
    if error.is_connect() {
        flags.push("connect");
    }
    if error.is_request() {
        flags.push("request");
    }
    if error.is_body() {
        flags.push("body");
    }
    if error.is_decode() {
        flags.push("decode");
    }
    if error.is_status() {
        flags.push("status");
    }

    let error_type = if flags.is_empty() {
        "unknown".to_string()
    } else {
        flags.join("+")
    };

    let mut details = vec![
        format!("type={}", error_type),
        format!("elapsed={}ms", elapsed.as_millis()),
    ];

    if let Some(url) = error.url() {
        details.push(format!("url={}", url));
    }

    if let Some(source) = error.source() {
        details.push(format!("source={}", source));
    }

    format!("{}：{}；{}", prefix, error, details.join("；"))
}

impl OpenAiSseAccumulator {
    fn push_payload<F>(&mut self, payload: &str, on_delta: &mut F) -> Result<bool, String>
    where
        F: FnMut(ProviderStreamChunk),
    {
        if payload == "[DONE]" {
            return Ok(true);
        }

        self.saw_data = true;
        let value = serde_json::from_str::<Value>(payload).map_err(|error| {
            format!(
                "解析 provider SSE chunk 失败: {}; 原始 chunk: {}",
                error, payload
            )
        })?;

        if let Some(token_usage) = extract_openai_usage(&value) {
            self.token_usage = Some(token_usage);
        }

        let delta = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"));

        let delta_text = delta
            .and_then(extract_openai_delta_content_text)
            .unwrap_or_default();
        let delta_reasoning_value = delta.and_then(extract_openai_delta_reasoning_value);
        let delta_reasoning = delta
            .and_then(extract_openai_delta_reasoning_content)
            .unwrap_or_default();
        if let Some(delta) = delta {
            merge_openai_stream_tool_calls(&mut self.tool_calls, delta);
        }

        if !delta_text.is_empty() {
            self.output_text.push_str(&delta_text);
            on_delta(ProviderStreamChunk::Text(delta_text));
        }

        if !delta_reasoning.is_empty() {
            self.reasoning_content.push_str(&delta_reasoning);
            on_delta(ProviderStreamChunk::Reasoning(delta_reasoning));
        }
        if self.reasoning_content_value.is_none() && delta_reasoning_value.is_some() {
            self.reasoning_content_value = delta_reasoning_value;
        }

        Ok(false)
    }

    fn finish(self, response_preview: &str) -> Result<OpenAiStreamMessage, String> {
        let OpenAiSseAccumulator {
            output_text,
            reasoning_content,
            reasoning_content_value,
            tool_calls,
            token_usage,
            saw_data,
        } = self;
        if !saw_data {
            return Err(format!(
                "provider 流式返回中未找到 SSE data 事件；响应预览: {}",
                response_preview
            ));
        }

        let tool_call = tool_calls
            .into_iter()
            .next()
            .and_then(|(_, partial)| partial_openai_tool_call_to_tool_call(partial));

        if output_text.is_empty() && tool_call.is_none() {
            return Err(format!(
                "provider 流式返回中未提取到文本内容；响应预览: {}",
                response_preview
            ));
        }

        Ok(OpenAiStreamMessage {
            output_text,
            tool_call,
            reasoning_content: if reasoning_content.trim().is_empty() {
                None
            } else {
                Some(reasoning_content.clone())
            },
            reasoning_content_value: reasoning_content_value.or_else(|| {
                (!reasoning_content.trim().is_empty())
                    .then(|| Value::String(reasoning_content))
            }),
            token_usage,
        })
    }
}

fn collect_openai_sse_message_from_reader<R, F>(
    reader: R,
    response_preview: &str,
    on_delta: &mut F,
) -> Result<OpenAiStreamMessage, String>
where
    R: BufRead,
    F: FnMut(ProviderStreamChunk),
{
    let mut accumulator = OpenAiSseAccumulator::default();

    for line in reader.lines() {
        let line = line.map_err(|error| format!("读取 provider SSE 数据失败: {}", error))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some(data) = trimmed.strip_prefix("data:") else {
            continue;
        };

        if accumulator.push_payload(data.trim(), on_delta)? {
            break;
        }
    }

    accumulator.finish(response_preview)
}

fn collect_openai_sse_message_from_response<F>(
    response: reqwest::blocking::Response,
    started_at: Instant,
    endpoint: &str,
    on_delta: &mut F,
) -> Result<OpenAiStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let reader = BufReader::new(response);
    collect_openai_sse_message_from_reader(reader, endpoint, on_delta).map_err(|error| {
        format!(
            "解析 provider SSE 流失败: {}; elapsed={}ms; endpoint={}",
            error,
            started_at.elapsed().as_millis(),
            endpoint
        )
    })
}

#[cfg(test)]
fn collect_openai_sse_message<F>(
    raw_text: &str,
    on_delta: &mut F,
) -> Result<OpenAiStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let mut combined = String::new();
    let mut reasoning = String::new();
    let mut reasoning_value = None;
    let mut saw_data = false;

    for line in raw_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some(data) = trimmed.strip_prefix("data:") else {
            continue;
        };

        let payload = data.trim();
        if payload == "[DONE]" {
            break;
        }

        saw_data = true;
        let value = serde_json::from_str::<Value>(payload).map_err(|error| {
            format!(
                "解析 provider SSE chunk 失败：{}；原始 chunk：{}",
                error, payload
            )
        })?;

        let delta = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"));

        let delta_text = delta
            .and_then(extract_openai_delta_content_text)
            .unwrap_or_default();
        let delta_reasoning_value = delta.and_then(extract_openai_delta_reasoning_value);
        let delta_reasoning = delta
            .and_then(extract_openai_delta_reasoning_content)
            .unwrap_or_default();

        if !delta_text.is_empty() {
            combined.push_str(&delta_text);
            on_delta(ProviderStreamChunk::Text(delta_text));
        }

        if !delta_reasoning.is_empty() {
            reasoning.push_str(&delta_reasoning);
            on_delta(ProviderStreamChunk::Reasoning(delta_reasoning));
        }
        if reasoning_value.is_none() && delta_reasoning_value.is_some() {
            reasoning_value = delta_reasoning_value;
        }
    }

    if !saw_data {
        return Err(format!(
            "provider 流式返回中未找到 SSE data 事件；原始响应：{}",
            raw_text
        ));
    }

    if combined.is_empty() {
        return Err(format!(
            "provider 流式返回中未提取到文本内容；原始响应：{}",
            raw_text
        ));
    }

    Ok(OpenAiStreamMessage {
        output_text: combined,
        tool_call: None,
        reasoning_content: if reasoning.trim().is_empty() {
            None
        } else {
            Some(reasoning.clone())
        },
        reasoning_content_value: reasoning_value.or_else(|| {
            (!reasoning.trim().is_empty()).then(|| Value::String(reasoning))
        }),
        token_usage: None,
    })
}

#[test]
fn openai_usage_extracts_cache_hit_and_reasoning_tokens() {
    let payload = json!({
        "usage": {
            "prompt_tokens": 120,
            "prompt_cache_hit_tokens": 48,
            "completion_tokens": 32,
            "total_tokens": 152,
            "completion_tokens_details": {
                "reasoning_tokens": 12
            }
        }
    });

    let usage = extract_openai_usage(&payload).expect("usage should be extracted");
    assert_eq!(usage.input_tokens, Some(120));
    assert_eq!(usage.cache_hit_input_tokens, Some(48));
    assert_eq!(usage.reasoning_tokens, Some(12));
    assert_eq!(usage.output_tokens, Some(32));
    assert_eq!(usage.total_tokens, Some(152));
}

fn with_openai_request_options(mut body: Value, config: &ResolvedProviderSelection) -> Value {
    if openai_requires_thinking_mode(config) {
        body["thinking"] = json!({
            "type": "enabled"
        });
    }

    if config.capabilities.supports_reasoning {
        if let Some(effort) = config.reasoning_effort.as_ref() {
            body["reasoning_effort"] = Value::String(reasoning_effort_label(effort).to_string());
        }
    }

    body
}

fn with_anthropic_request_options(mut body: Value, config: &ResolvedProviderSelection) -> Value {
    if config.capabilities.supports_reasoning {
        if let Some(budget_tokens) = config.reasoning_budget_tokens.filter(|budget| *budget > 0) {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget_tokens
            });
        }
    }

    body
}

fn apply_openai_tool_capability(
    mut body: Value,
    tools: &[ToolDefinition],
    config: &ResolvedProviderSelection,
) -> Value {
    let supports_tools = config.capabilities.supports_tools && !tools.is_empty();
    let supports_tool_choice = openai_supports_tool_choice(config);

    if supports_tools {
        if !supports_tool_choice {
            if let Some(object) = body.as_object_mut() {
                object.remove("tool_choice");
            }
        }
        return body;
    }

    if let Some(object) = body.as_object_mut() {
        object.remove("tools");
        object.remove("tool_choice");
    }
    body
}

fn apply_anthropic_tool_capability(
    mut body: Value,
    tools: &[ToolDefinition],
    config: &ResolvedProviderSelection,
) -> Value {
    if config.capabilities.supports_tools && !tools.is_empty() {
        return body;
    }

    if let Some(object) = body.as_object_mut() {
        object.remove("tools");
        object.remove("tool_choice");
    }
    body
}

fn openai_requires_thinking_mode(config: &ResolvedProviderSelection) -> bool {
    config.capabilities.supports_reasoning && is_deepseek_provider(config)
}

fn openai_supports_tool_choice(config: &ResolvedProviderSelection) -> bool {
    !openai_requires_thinking_mode(config)
}

fn is_deepseek_provider(config: &ResolvedProviderSelection) -> bool {
    let requested = config.requested_name.to_lowercase();
    let provider = config.provider_name.to_lowercase();
    let base_url = config.base_url.to_lowercase();
    let model = config.model.to_lowercase();

    requested.contains("deepseek")
        || provider.contains("deepseek")
        || base_url.contains("deepseek")
        || model.contains("deepseek")
}

fn reasoning_effort_label(effort: &ProviderReasoningEffort) -> &'static str {
    return match effort {
        ProviderReasoningEffort::Minimal => "minimal",
        ProviderReasoningEffort::Low => "low",
        ProviderReasoningEffort::Medium => "medium",
        ProviderReasoningEffort::High => "high",
    };

    #[cfg(any())]
    fn openai_stream_message_collects_content_and_reasoning() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看文件结构。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第 3 行是 \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"use crate::...\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("stream message should parse");

        assert_eq!(message.output_text, "第 3 行是 use crate::...");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看文件结构。"));
        assert_eq!(
            deltas,
            vec!["第 3 行是 ".to_string(), "use crate::...".to_string()]
        );
    }
    // */
}

fn to_chat_role(role: &ProviderRole) -> &'static str {
    match role {
        ProviderRole::System | ProviderRole::Developer => "system",
        ProviderRole::User => "user",
    }
}

fn openai_tools_payload(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": openai_safe_tool_name(tool.name),
                    "description": tool.description,
                    "parameters": tool.input_schema.clone()
                }
            })
        })
        .collect()
}

fn anthropic_tools_payload(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema.clone()
            })
        })
        .collect()
}

fn first_openai_message<'a>(payload: &'a Value) -> Result<&'a Value, String> {
    payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| "openai 返回中缺少 message".to_string())
}

fn normalize_openai_assistant_message(message: &Value) -> Value {
    let tool_calls = message.get("tool_calls").cloned();
    let content_value = match message.get("content") {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(items)) => Value::Array(items.clone()),
        Some(Value::Null) | None => {
            if tool_calls
                .as_ref()
                .and_then(Value::as_array)
                .map(|calls| !calls.is_empty())
                .unwrap_or(false)
            {
                Value::String(extract_openai_message_text(message).unwrap_or_default())
            } else {
                Value::Null
            }
        }
        Some(other) => other.clone(),
    };

    let reasoning_value = extract_openai_message_reasoning_value(message).unwrap_or(Value::Null);

    let mut normalized = json!({
        "role": "assistant",
        "content": content_value,
        "reasoning_content": reasoning_value,
    });

    if let Some(tool_calls) = tool_calls {
        normalized["tool_calls"] = tool_calls;
    }

    normalized
}

fn normalize_openai_messages(messages: Vec<Value>) -> Vec<Value> {
    messages
        .into_iter()
        .map(|message| {
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                normalize_openai_assistant_message(&message)
            } else {
                message
            }
        })
        .collect()
}

fn extract_openai_message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;

    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_str)
                        .map(|text| text.to_string())
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        Value::Null => None,
        _ => None,
    }
}

fn extract_openai_tool_call(message: &Value) -> Option<ToolCall> {
    let tool_call = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .and_then(|calls| calls.first())?;

    let id = tool_call
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let function = tool_call.get("function")?;
    let name = openai_original_tool_name(function.get("name").and_then(Value::as_str)?);
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));

    Some(ToolCall {
        call_id: id,
        name,
        arguments,
        plan: None,
    })
}

fn merge_openai_stream_tool_calls(
    tool_calls: &mut BTreeMap<usize, PartialOpenAiToolCall>,
    delta: &Value,
) {
    let Some(items) = delta.get("tool_calls").and_then(Value::as_array) else {
        return;
    };

    for item in items {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0);
        let partial = tool_calls.entry(index).or_default();

        if let Some(id) = item.get("id").and_then(Value::as_str) {
            partial.id = Some(id.to_string());
        }

        if let Some(function) = item.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                partial.name = Some(name.to_string());
            }

            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                partial.arguments.push_str(arguments);
            }
        }
    }
}

fn partial_openai_tool_call_to_tool_call(partial: PartialOpenAiToolCall) -> Option<ToolCall> {
    let name = openai_original_tool_name(partial.name.as_deref()?);
    let arguments = if partial.arguments.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&partial.arguments).unwrap_or_else(|_| json!({}))
    };

    Some(ToolCall {
        call_id: partial.id,
        name,
        arguments,
        plan: None,
    })
}

fn text_if_present(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn anthropic_system_text(input: &[ProviderMessage]) -> String {
    input
        .iter()
        .filter(|message| matches!(message.role, ProviderRole::System | ProviderRole::Developer))
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn anthropic_user_messages(request: &ProviderRequest) -> Vec<Value> {
    let user_indexes = request
        .input
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if matches!(message.role, ProviderRole::User) {
                Some(index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let last_user_index = user_indexes.last().copied();

    request
        .input
        .iter()
        .enumerate()
        .filter(|(_, message)| matches!(message.role, ProviderRole::User))
        .map(|(index, message)| {
            json!({
                "role": "user",
                "content": anthropic_user_content(message.content.as_str(), is_last_user_with_images(index, last_user_index, &request.images), &request.images)
            })
        })
        .collect::<Vec<_>>()
}

fn extract_anthropic_text_blocks(content: &[Value]) -> String {
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_anthropic_tool_call(content: &[Value]) -> Option<ToolCall> {
    let block = content
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))?;

    Some(ToolCall {
        call_id: block.get("id").and_then(Value::as_str).map(str::to_string),
        name: block.get("name").and_then(Value::as_str)?.to_string(),
        arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
        plan: None,
    })
}

fn openai_messages_with_tool_result(
    request: &ProviderRequest,
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Vec<Value> {
    let mut messages = openai_request_messages(request);

    messages.push(openai_assistant_message_for_tool_result(
        assistant_message,
        tool_call,
    ));

    messages.push(openai_followup_tool_result_message(tool_call, tool_result));

    messages
}

fn openai_assistant_message_for_tool_result(
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
) -> Value {
    let Some(message) = assistant_message else {
        return provider_native_assistant_tool_call_message(None, None, tool_call);
    };

    let mut normalized = normalize_openai_assistant_message(message);
    let filtered_tool_calls = normalized
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter(|call| openai_tool_call_matches_executed_call(call, tool_call))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if filtered_tool_calls.is_empty() {
        return provider_native_assistant_tool_call_message_with_reasoning_value(
            normalized.get("content").and_then(Value::as_str),
            normalized.get("reasoning_content"),
            tool_call,
        );
    }

    normalized["tool_calls"] = Value::Array(filtered_tool_calls);
    normalized
}

fn openai_tool_call_matches_executed_call(call: &Value, executed_call: &ToolCall) -> bool {
    if let Some(executed_call_id) = executed_call.call_id.as_deref() {
        return call.get("id").and_then(Value::as_str) == Some(executed_call_id);
    }

    call.get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(openai_original_tool_name)
        .as_deref()
        == Some(executed_call.name.as_str())
}

fn openai_request_messages(request: &ProviderRequest) -> Vec<Value> {
    if !request.native_messages.is_empty() {
        return request.native_messages.clone();
    }

    let user_indexes = request
        .input
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if matches!(message.role, ProviderRole::User) {
                Some(index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let last_user_index = user_indexes.last().copied();

    request
        .input
        .iter()
        .enumerate()
        .map(|(index, message)| {
            json!({
                "role": to_chat_role(&message.role),
                "content": openai_message_content(message.content.as_str(), is_last_user_with_images(index, last_user_index, &request.images), &request.images)
            })
        })
        .collect::<Vec<_>>()
}

fn is_last_user_with_images(
    index: usize,
    last_user_index: Option<usize>,
    images: &[TurnInputImage],
) -> bool {
    !images.is_empty() && Some(index) == last_user_index
}

fn openai_message_content(content: &str, include_images: bool, images: &[TurnInputImage]) -> Value {
    if !include_images {
        return Value::String(content.to_string());
    }

    Value::Array(openai_user_content_blocks(content, images))
}

fn openai_user_content_blocks(content: &str, images: &[TurnInputImage]) -> Vec<Value> {
    let mut blocks = Vec::new();
    if !content.trim().is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": content
        }));
    }

    blocks.extend(images.iter().map(|image| {
        json!({
            "type": "image_url",
            "image_url": {
                "url": image.data_url
            }
        })
    }));
    blocks
}

fn anthropic_user_content(content: &str, include_images: bool, images: &[TurnInputImage]) -> Value {
    if !include_images {
        return Value::String(content.to_string());
    }

    let mut blocks = Vec::new();
    if !content.trim().is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": content
        }));
    }

    for image in images {
        let Some(base64_data) = image.base64_data() else {
            continue;
        };

        blocks.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.mime_type,
                "data": base64_data
            }
        }));
    }

    Value::Array(blocks)
}

pub fn provider_native_user_message(content: &str) -> Value {
    json!({
        "role": "user",
        "content": content
    })
}

pub fn provider_native_assistant_message(content: &str) -> Value {
    provider_native_assistant_message_with_reasoning(content, None)
}

pub fn provider_native_assistant_message_with_reasoning(
    content: &str,
    reasoning_content: Option<&str>,
) -> Value {
    provider_native_assistant_message_with_reasoning_value(
        content,
        reasoning_content.map(|value| Value::String(value.to_string())).as_ref(),
    )
}

pub fn provider_native_assistant_message_with_reasoning_value(
    content: &str,
    reasoning_content: Option<&Value>,
) -> Value {
    json!({
        "role": "assistant",
        "content": content,
        "reasoning_content": reasoning_content
    })
}

pub fn provider_native_assistant_tool_call_message(
    content: Option<&str>,
    reasoning_content: Option<&str>,
    tool_call: &ToolCall,
) -> Value {
    provider_native_assistant_tool_call_message_with_reasoning_value(
        content,
        reasoning_content.map(|value| Value::String(value.to_string())).as_ref(),
        tool_call,
    )
}

pub fn provider_native_assistant_tool_call_message_with_reasoning_value(
    content: Option<&str>,
    reasoning_content: Option<&Value>,
    tool_call: &ToolCall,
) -> Value {
    json!({
        "role": "assistant",
        "content": content,
        "reasoning_content": reasoning_content,
        "tool_calls": [
            {
                "id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
                "type": "function",
                "function": {
                    "name": openai_safe_tool_name(&tool_call.name),
                    "arguments": tool_call.arguments.to_string()
                }
            }
        ]
    })
}

pub fn provider_native_tool_result_message(
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
        "content": tool_result.output.clone()
    })
}

const OPENAI_TOOL_RESULT_INLINE_LIMIT_CHARS: usize = 12_000;
const OPENAI_TOOL_RESULT_HEAD_CHARS: usize = 4_500;
const OPENAI_TOOL_RESULT_TAIL_CHARS: usize = 2_500;
const OPENAI_TOOL_RESULT_SUMMARY_PREVIEW_CHARS: usize = 240;

fn openai_followup_tool_result_message(tool_call: &ToolCall, tool_result: &ToolResult) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
        "content": openai_followup_tool_result_content(tool_result)
    })
}

fn openai_followup_tool_result_content(tool_result: &ToolResult) -> String {
    let output_chars = tool_result.output.chars().count();
    if output_chars <= OPENAI_TOOL_RESULT_INLINE_LIMIT_CHARS {
        return tool_result.output.clone();
    }

    let total_lines = tool_result.output.lines().count();
    let head = take_chars(
        &tool_result.output,
        OPENAI_TOOL_RESULT_HEAD_CHARS.min(output_chars),
    );
    let remaining_chars = output_chars.saturating_sub(head.chars().count());
    let tail = take_last_chars(
        &tool_result.output,
        OPENAI_TOOL_RESULT_TAIL_CHARS.min(remaining_chars),
    );
    let payload = json!({
        "tool_name": tool_result.tool_name,
        "status": tool_result.status,
        "compression": {
            "applied": true,
            "strategy": "head_tail_excerpt",
            "reason": "provider_followup_size_guard",
            "original_chars": output_chars,
            "original_lines": total_lines,
            "included_head_chars": head.chars().count(),
            "included_tail_chars": tail.chars().count(),
            "omitted_chars": output_chars.saturating_sub(head.chars().count() + tail.chars().count())
        },
        "summary": summarize_large_tool_output(&tool_result.output),
        "head": head,
        "tail": tail
    });

    serde_json::to_string(&payload).unwrap_or_else(|_| {
        format!(
            "{{\"tool_name\":\"{}\",\"status\":\"{}\",\"compression\":{{\"applied\":true,\"reason\":\"provider_followup_size_guard\",\"original_chars\":{}}}}}",
            escape_json_fragment(&tool_result.tool_name),
            escape_json_fragment(&tool_result.status),
            output_chars
        )
    })
}

fn local_tool_followup_fallback_response(
    request: &ProviderRequest,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
    error: String,
) -> ProviderResponse {
    let fallback_reason = format!("provider_followup_failed: {}", preview_text(&error, 240));
    let summary = local_tool_followup_summary(tool_result);
    let output_text = format!(
        "工具 `{}` 已执行完成，但 provider 在整合工具结果时失败。\n失败原因：{}\n\n本地兜底摘要：{}\n\n结果预览：\n{}",
        tool_call.name,
        preview_text(&error, 240),
        summary,
        preview_text(&tool_result.output, 2200)
    );

    ProviderResponse {
        output_text: output_text.clone(),
        tool_call: None,
        reasoning_content: None,
        reasoning_content_value: None,
        assistant_message: Some(provider_native_assistant_message(&output_text)),
        provider_source: "provider_followup_local_fallback".to_string(),
        provider_mode: "fallback".to_string(),
        fallback_reason: Some(fallback_reason),
        token_usage: Some(estimate_token_usage(request, &output_text)),
    }
}

fn local_tool_followup_summary(tool_result: &ToolResult) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(&tool_result.output) {
        if let Some(summary_text) = value
            .get("summary")
            .and_then(|summary| summary.get("text"))
            .and_then(Value::as_str)
        {
            return summary_text.to_string();
        }

        if let Some(plan_summary) = value
            .get("plan")
            .and_then(|plan| plan.get("summary"))
            .and_then(Value::as_str)
        {
            return plan_summary.to_string();
        }

        return summarize_json_value(&value).to_string();
    }

    format!(
        "status={}；tool={}；chars={}",
        tool_result.status,
        tool_result.tool_name,
        tool_result.output.chars().count()
    )
}

fn summarize_large_tool_output(output: &str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        return summarize_json_value(&value);
    }

    let first_non_empty_line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");

    json!({
        "kind": "text",
        "first_line_preview": preview_text(first_non_empty_line, OPENAI_TOOL_RESULT_SUMMARY_PREVIEW_CHARS)
    })
}

fn summarize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => json!({
            "kind": "json_object",
            "key_count": map.len(),
            "keys": map.keys().take(12).cloned().collect::<Vec<_>>()
        }),
        Value::Array(items) => json!({
            "kind": "json_array",
            "length": items.len(),
            "first_item_kind": items.first().map(json_value_kind).unwrap_or("empty")
        }),
        Value::String(_) => json!({ "kind": "json_string" }),
        Value::Number(_) => json!({ "kind": "json_number" }),
        Value::Bool(_) => json!({ "kind": "json_boolean" }),
        Value::Null => json!({ "kind": "json_null" }),
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Object(_) => "object",
        Value::Array(_) => "array",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Null => "null",
    }
}

fn take_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn take_last_chars(text: &str, count: usize) -> String {
    let total = text.chars().count();
    text.chars().skip(total.saturating_sub(count)).collect()
}

fn escape_json_fragment(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn anthropic_messages_with_tool_result(
    request: &ProviderRequest,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Vec<Value> {
    let mut messages = anthropic_user_messages(request);
    messages.push(json!({
        "role": "assistant",
        "content": [
            {
                "type": "tool_use",
                "id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
                "name": tool_call.name.clone(),
                "input": tool_call.arguments.clone()
            }
        ]
    }));

    messages.push(json!({
        "role": "user",
        "content": [
            {
                "type": "tool_result",
                "tool_use_id": tool_call.call_id.clone().unwrap_or_else(|| "toolu_local".to_string()),
                "content": tool_result.output.clone(),
                "is_error": tool_result.status != "ok"
            }
        ]
    }));

    messages
}

#[allow(dead_code)]
fn extract_chat_output_text(payload: &Value) -> Option<String> {
    let message = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))?;
    let content = message.get("content")?;

    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_str)
                        .map(|text| text.to_string())
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn extract_openai_message_reasoning_content(message: &Value) -> Option<String> {
    extract_openai_message_reasoning_value(message)
        .as_ref()
        .and_then(extract_reasoning_text_from_value)
}

fn extract_openai_message_reasoning_value(message: &Value) -> Option<Value> {
    message.get("reasoning_content").cloned()
}

fn extract_openai_delta_content_text(delta: &Value) -> Option<String> {
    match delta.get("content")? {
        Value::String(text) => Some(text.to_string()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

fn extract_openai_delta_reasoning_content(delta: &Value) -> Option<String> {
    extract_openai_delta_reasoning_value(delta)
        .as_ref()
        .and_then(extract_reasoning_text_from_value)
}

fn extract_openai_delta_reasoning_value(delta: &Value) -> Option<Value> {
    delta.get("reasoning_content").cloned()
}

fn extract_reasoning_text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.to_string()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|nested| nested.get("value"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

#[allow(dead_code)]
fn extract_chat_delta_text(payload: &Value) -> Option<String> {
    let delta = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))?;
    let content = delta.get("content")?;

    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        return Some(text.to_string());
                    }
                    item.get("text")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_str)
                        .map(|text| text.to_string())
                })
                .collect::<Vec<_>>();

            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

fn extract_openai_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload.get("usage")?;
    Some(normalize_token_usage(TokenUsage {
        input_tokens: usage.get("prompt_tokens").and_then(Value::as_u64),
        cache_hit_input_tokens: usage
            .get("prompt_cache_hit_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                usage
                    .get("input_tokens_details")
                    .and_then(|value| value.get("cached_tokens"))
                    .and_then(Value::as_u64)
            })
            .or_else(|| {
                usage
                    .get("prompt_tokens_details")
                    .and_then(|value| value.get("cached_tokens"))
                    .and_then(Value::as_u64)
            }),
        reasoning_tokens: usage
            .get("completion_tokens_details")
            .and_then(|value| value.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .or_else(|| {
                usage
                    .get("output_tokens_details")
                    .and_then(|value| value.get("reasoning_tokens"))
                    .and_then(Value::as_u64)
            }),
        output_tokens: usage.get("completion_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
    }))
}

fn openai_safe_tool_name(name: &str) -> String {
    name.replace('.', "__dot__")
}

fn openai_original_tool_name(name: &str) -> String {
    name.replace("__dot__", ".")
}

fn extract_anthropic_output_text(payload: &Value) -> Option<String> {
    let parts = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .map(|text| text.to_string())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn extract_anthropic_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload
        .get("usage")
        .or_else(|| {
            payload
                .get("message")
                .and_then(|message| message.get("usage"))
        })
        .or_else(|| payload.get("delta").and_then(|delta| delta.get("usage")))?;

    Some(normalize_token_usage(TokenUsage {
        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
    }))
}

#[allow(dead_code)]
fn extract_anthropic_delta_text(payload: &Value) -> Option<String> {
    let delta = payload.get("delta")?;
    let delta_type = delta.get("type").and_then(Value::as_str)?;

    if delta_type != "text_delta" {
        return None;
    }

    delta
        .get("text")
        .and_then(Value::as_str)
        .map(|text| text.to_string())
}

fn estimate_token_usage(request: &ProviderRequest, output_text: &str) -> TokenUsage {
    let input_chars = if !request.native_messages.is_empty() {
        request
            .native_messages
            .iter()
            .map(estimate_native_message_chars)
            .sum::<usize>()
    } else {
        request
            .input
            .iter()
            .map(|message| message.content.chars().count())
            .sum::<usize>()
    };
    let output_chars = output_text.chars().count();

    normalize_token_usage(TokenUsage {
        input_tokens: Some(estimate_tokens_from_chars(input_chars)),
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: Some(estimate_tokens_from_chars(output_chars)),
        total_tokens: None,
    })
}

fn estimate_tokens_from_chars(char_count: usize) -> u64 {
    if char_count == 0 {
        0
    } else {
        char_count.div_ceil(4) as u64
    }
}

fn estimate_native_message_chars(message: &Value) -> usize {
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        return content.chars().count();
    }

    if let Some(content) = message.get("content").and_then(Value::as_array) {
        return content.iter().map(estimate_native_content_chars).sum();
    }

    message.to_string().chars().count()
}

fn estimate_native_content_chars(block: &Value) -> usize {
    if block.get("type").and_then(Value::as_str) == Some("image")
        || block.get("type").and_then(Value::as_str) == Some("image_url")
    {
        return 1024;
    }

    if let Some(text) = block.get("text").and_then(Value::as_str) {
        return text.chars().count();
    }

    if let Some(text) = block
        .get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
    {
        return text.chars().count();
    }

    block.to_string().chars().count()
}

fn normalize_token_usage(usage: TokenUsage) -> TokenUsage {
    let total_tokens =
        usage
            .total_tokens
            .or_else(|| match (usage.input_tokens, usage.output_tokens) {
                (Some(input_tokens), Some(output_tokens)) => Some(input_tokens + output_tokens),
                _ => None,
            });

    TokenUsage {
        input_tokens: usage.input_tokens,
        cache_hit_input_tokens: usage.cache_hit_input_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        output_tokens: usage.output_tokens,
        total_tokens,
    }
}

fn render_input_messages(messages: &[ProviderMessage]) -> String {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            format!(
                "[{}] {}\n{}",
                index,
                to_chat_role(&message.role),
                message.content.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_native_messages(messages: &[Value]) -> String {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let content = match message.get("content") {
                Some(Value::String(text)) => text.trim().to_string(),
                Some(other) => {
                    serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string())
                }
                None => message.to_string(),
            };
            format!("[{}] {}\n{}", index, role, content)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_tool_definitions(tools: &[ToolDefinition]) -> String {
    if tools.is_empty() {
        return "none".to_string();
    }

    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            format!(
                "[{}] {}\ndescription: {}\nschema:\n{}",
                index,
                tool.name,
                tool.description,
                serde_json::to_string_pretty(&tool.input_schema)
                    .unwrap_or_else(|_| tool.input_schema.to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn derive_request_observation(request: &ProviderRequest) -> ProviderRequestObservation {
    if !request.native_messages.is_empty() {
        derive_native_request_observation(&request.native_messages)
    } else {
        derive_input_request_observation(&request.input)
    }
}

fn derive_input_request_observation(messages: &[ProviderMessage]) -> ProviderRequestObservation {
    if messages.is_empty() {
        return ProviderRequestObservation::default();
    }

    let stable_prefix_end = messages
        .iter()
        .enumerate()
        .take_while(|(index, message)| is_stable_input_prefix_message(*index, message))
        .count()
        .min(messages.len().saturating_sub(1));
    let volatile_input_start = messages
        .iter()
        .rposition(|message| matches!(message.role, ProviderRole::User))
        .unwrap_or(messages.len().saturating_sub(1));

    ProviderRequestObservation {
        stable_prefix_text: render_input_messages(&messages[..stable_prefix_end]),
        semi_stable_context_text: render_input_messages(
            &messages[stable_prefix_end..volatile_input_start],
        ),
        volatile_input_text: render_input_messages(&messages[volatile_input_start..]),
        prefix_mutation_reasons: Vec::new(),
    }
}

fn derive_native_request_observation(messages: &[Value]) -> ProviderRequestObservation {
    if messages.is_empty() {
        return ProviderRequestObservation::default();
    }

    let stable_prefix_end = messages
        .iter()
        .enumerate()
        .take_while(|(index, message)| is_stable_native_prefix_message(*index, message))
        .count()
        .min(messages.len().saturating_sub(1));
    let volatile_input_start = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .unwrap_or(messages.len().saturating_sub(1));

    ProviderRequestObservation {
        stable_prefix_text: render_native_messages(&messages[..stable_prefix_end]),
        semi_stable_context_text: render_native_messages(
            &messages[stable_prefix_end..volatile_input_start],
        ),
        volatile_input_text: render_native_messages(&messages[volatile_input_start..]),
        prefix_mutation_reasons: Vec::new(),
    }
}

fn is_stable_input_prefix_message(index: usize, message: &ProviderMessage) -> bool {
    match message.role {
        ProviderRole::System => index == 0 || !looks_like_dynamic_prefix_content(&message.content),
        ProviderRole::Developer => !looks_like_dynamic_prefix_content(&message.content),
        ProviderRole::User => false,
    }
}

fn is_stable_native_prefix_message(index: usize, message: &Value) -> bool {
    if message.get("role").and_then(Value::as_str) != Some("system") {
        return false;
    }

    let content = match message.get("content") {
        Some(Value::String(text)) => text,
        _ => return index == 0,
    };

    index == 0 || !looks_like_dynamic_prefix_content(content)
}

fn looks_like_dynamic_prefix_content(content: &str) -> bool {
    if content.starts_with("Capability profile:") {
        return false;
    }

    [
        "Session summary:",
        "Run goal:",
        "Long-term memory status:",
        "Older context was truncated",
        "supportsImageInput=false for this model.",
        "This model is marked as image-capable.",
        "The user appears to reference image content",
        "graph=",
        "session=",
    ]
    .iter()
    .any(|marker| content.contains(marker))
}

fn provider_log(message: String) {
    eprintln!("[pony-provider] {}", message);
}

fn provider_log_token_usage(context: &str, usage: &TokenUsage) {
    provider_log(format!(
        "usage:{} input={:?} cache_hit_input={:?} reasoning={:?} output={:?} total={:?}",
        context,
        usage.input_tokens,
        usage.cache_hit_input_tokens,
        usage.reasoning_tokens,
        usage.output_tokens,
        usage.total_tokens
    ));
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let count = normalized.chars().count();
    if count <= max_chars {
        normalized
    } else {
        let preview = normalized.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

fn preview_json(value: &Value, max_chars: usize) -> String {
    match serde_json::to_string(value) {
        Ok(text) => preview_text(&text, max_chars),
        Err(error) => format!("json-serialize-error: {}", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::ProviderModelCapabilities;
    use crate::agent::config::ResolvedProviderSelection;
    use crate::agent::input::TurnInputImage;

    #[test]
    fn openai_request_messages_encode_image_blocks_for_last_user_message() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![
                ProviderMessage::developer("Keep answers short."),
                ProviderMessage::user("请描述这张图"),
            ],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,Zm9v".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("demo.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let messages = openai_request_messages(&request);
        let content = messages[1]
            .get("content")
            .and_then(Value::as_array)
            .expect("last user message should use content blocks");

        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            content[1]
                .get("image_url")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str),
            Some("data:image/png;base64,Zm9v")
        );
    }

    #[test]
    fn anthropic_user_messages_encode_base64_image_blocks_for_last_user_message() {
        let request = ProviderRequest {
            model: "claude-3-7-sonnet".to_string(),
            input: vec![ProviderMessage::user("请看图回答")],
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,Zm9v".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("demo.png".to_string()),
            }],
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let messages = anthropic_user_messages(&request);
        let content = messages[0]
            .get("content")
            .and_then(Value::as_array)
            .expect("anthropic user message should use content blocks");

        assert_eq!(content[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            content[1].get("type").and_then(Value::as_str),
            Some("image")
        );
        assert_eq!(
            content[1]
                .get("source")
                .and_then(|value| value.get("data"))
                .and_then(Value::as_str),
            Some("Zm9v")
        );
    }

    #[test]
    fn build_context_observation_exposes_layered_request_segments() {
        let request = ProviderRequest {
            model: "gpt-4.1-mini".to_string(),
            input: vec![
                ProviderMessage::system("Stable system instruction"),
                ProviderMessage::developer("Stable capability prefix"),
                ProviderMessage::user("Recent context summary"),
                ProviderMessage::user("Actual volatile request"),
            ],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(
            &request,
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
        );

        assert_eq!(observation.request_format, "normalized-input");
        assert_eq!(observation.message_count, 4);
        assert!(observation
            .stable_prefix_text
            .contains("Stable system instruction"));
        assert!(observation
            .stable_prefix_text
            .contains("Stable capability prefix"));
        assert!(!observation
            .stable_prefix_text
            .contains("Actual volatile request"));
        assert!(observation
            .semi_stable_context_text
            .contains("Recent context summary"));
        assert!(observation
            .volatile_input_text
            .contains("Actual volatile request"));
        assert_eq!(observation.tool_count, 1);
    }

    #[test]
    fn build_context_observation_prefers_explicit_layer_metadata() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("fallback text should not win")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation {
                stable_prefix_text: "stable prefix".to_string(),
                semi_stable_context_text: "semi-stable context".to_string(),
                volatile_input_text: "actual request".to_string(),
                prefix_mutation_reasons: vec![PrefixMutationReason::SessionSummaryChanged],
            },
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(&request, &[]);

        assert_eq!(observation.stable_prefix_text, "stable prefix");
        assert_eq!(observation.semi_stable_context_text, "semi-stable context");
        assert_eq!(observation.volatile_input_text, "actual request");
        assert!(!observation
            .stable_prefix_text
            .contains("fallback text should not win"));
        assert_eq!(
            observation.prefix_mutation_reasons,
            vec![PrefixMutationReason::SessionSummaryChanged]
        );
    }

    #[test]
    fn build_context_observation_fallback_keeps_dynamic_system_and_developer_text_out_of_stable_prefix(
    ) {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![
                ProviderMessage::system("Stable system instruction"),
                ProviderMessage::developer(
                    "Capability profile: contextWindowTokens=128000 / supportsImageInput=true.",
                ),
                ProviderMessage::developer(
                    "Session summary: volatile summary / graph=graph-a / session=session-1",
                ),
                ProviderMessage::user("actual request"),
            ],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };

        let observation = build_context_observation(&request, &[]);

        assert!(observation
            .stable_prefix_text
            .contains("Stable system instruction"));
        assert!(observation
            .stable_prefix_text
            .contains("Capability profile:"));
        assert!(!observation
            .stable_prefix_text
            .contains("Session summary: volatile summary"));
        assert!(observation
            .semi_stable_context_text
            .contains("Session summary: volatile summary"));
    }

    #[test]
    fn tool_capability_disabled_removes_openai_tool_fields() {
        let config = ResolvedProviderSelection {
            requested_name: "openai".to_string(),
            provider_name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "gpt-4.1-mini".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: false,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: false,
            },
        };

        let body = apply_openai_tool_capability(
            json!({
                "tool_choice": "auto",
                "tools": [{ "type": "function" }]
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tool_choice").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn deepseek_reasoning_removes_openai_tool_choice_but_keeps_tools() {
        let config = ResolvedProviderSelection {
            requested_name: "deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "deepseek-v4-pro".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: true,
            },
        };

        let body = apply_openai_tool_capability(
            json!({
                "tool_choice": "auto",
                "tools": [{ "type": "function", "function": { "name": "workspace_read_file" } }]
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tool_choice").is_none());
        assert!(body.get("tools").is_some());
    }

    #[test]
    fn tool_capability_disabled_removes_anthropic_tool_fields() {
        let config = ResolvedProviderSelection {
            requested_name: "anthropic".to_string(),
            provider_name: "anthropic".to_string(),
            protocol: ProviderProtocol::Anthropic,
            base_url: "https://api.anthropic.com/v1".to_string(),
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "claude-3-7-sonnet-latest".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(200_000),
                supports_tools: false,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: true,
            },
        };

        let body = apply_anthropic_tool_capability(
            json!({
                "tools": [{ "name": "workspace.read_file" }],
                "tool_choice": { "type": "auto" }
            }),
            &[ToolDefinition {
                name: "workspace.read_file",
                description: "read file",
                input_schema: json!({ "type": "object" }),
            }],
            &config,
        );

        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn openai_tool_followup_replays_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-pro".to_string(),
            input: vec![ProviderMessage::user(
                "检查 src-tauri/src/agent/provider.rs",
            )],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_123".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src-tauri/src/agent/provider.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "file content".to_string(),
            duration_ms: 12,
        };

        let messages = openai_messages_with_tool_result(
            &request,
            Some(&provider_native_assistant_tool_call_message(
                Some("先读取文件，再给出结论。"),
                Some("先确认 provider 协议分支，再继续调用工具。"),
                &tool_call,
            )),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].get("reasoning_content").and_then(Value::as_str),
            Some("先确认 provider 协议分支，再继续调用工具。")
        );
    }

    #[test]
    fn openai_tool_followup_preserves_structured_reasoning_content() {
        let request = ProviderRequest {
            model: "deepseek-v4-pro".to_string(),
            input: vec![ProviderMessage::user("继续处理工具结果")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_structured".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src-tauri/src/agent/provider.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "file content".to_string(),
            duration_ms: 12,
        };
        let structured_reasoning = json!([
            { "type": "reasoning", "text": "先确认工具结果。" },
            { "type": "reasoning", "text": "再决定下一步。" }
        ]);
        let assistant_message = json!({
            "role": "assistant",
            "content": "先读取文件，再给出结论。",
            "reasoning_content": structured_reasoning.clone(),
            "tool_calls": [
                {
                    "id": "call_structured",
                    "type": "function",
                    "function": {
                        "name": "workspace_read_file",
                        "arguments": "{\"path\":\"src-tauri/src/agent/provider.rs\"}"
                    }
                }
            ]
        });

        let messages = openai_messages_with_tool_result(
            &request,
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].get("reasoning_content"), Some(&structured_reasoning));
    }

    #[test]
    fn openai_tool_followup_filters_unexecuted_parallel_tool_calls() {
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("检查 capabilities 目录")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_path_info".to_string()),
            name: "workspace_path_info".to_string(),
            arguments: json!({ "path": "capabilities" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace_path_info".to_string(),
            status: "ok".to_string(),
            output: "{\"kind\":\"directory\"}".to_string(),
            duration_ms: 10,
        };
        let assistant_message = json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "需要查看目录信息。",
            "tool_calls": [
                {
                    "id": "call_path_info",
                    "type": "function",
                    "function": {
                        "name": "workspace_path_info",
                        "arguments": "{\"path\":\"capabilities\"}"
                    }
                },
                {
                    "id": "call_list_files",
                    "type": "function",
                    "function": {
                        "name": "workspace_list_files",
                        "arguments": "{\"path\":\"capabilities\"}"
                    }
                }
            ]
        });

        let messages = openai_messages_with_tool_result(
            &request,
            Some(&assistant_message),
            &tool_call,
            &tool_result,
        );

        let assistant_tool_calls = messages[1]
            .get("tool_calls")
            .and_then(Value::as_array)
            .expect("assistant tool calls");
        assert_eq!(assistant_tool_calls.len(), 1);
        assert_eq!(
            assistant_tool_calls[0].get("id").and_then(Value::as_str),
            Some("call_path_info")
        );
        assert_eq!(
            messages[2].get("tool_call_id").and_then(Value::as_str),
            Some("call_path_info")
        );
        assert!(!serde_json::to_string(&messages)
            .expect("messages serialize")
            .contains("call_list_files"));
    }

    #[test]
    fn openai_tool_followup_fallback_rebuild_keeps_structured_reasoning_content() {
        let tool_call = ToolCall {
            call_id: Some("call_repaired".to_string()),
            name: "workspace_read_file".to_string(),
            arguments: json!({ "path": "Cargo.toml" }),
            plan: None,
        };
        let structured_reasoning = json!([
            { "type": "reasoning", "text": "先修正工具调用，再继续。" }
        ]);
        let assistant_message = json!({
            "role": "assistant",
            "content": "继续读取文件",
            "reasoning_content": structured_reasoning.clone(),
            "tool_calls": [
                {
                    "id": "call_other",
                    "type": "function",
                    "function": {
                        "name": "workspace_list_files",
                        "arguments": "{\"path\":\".\"}"
                    }
                }
            ]
        });

        let rebuilt = openai_assistant_message_for_tool_result(Some(&assistant_message), &tool_call);

        assert_eq!(rebuilt.get("reasoning_content"), Some(&structured_reasoning));
        assert_eq!(
            rebuilt
                .get("tool_calls")
                .and_then(Value::as_array)
                .and_then(|calls| calls.first())
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("workspace_read_file")
        );
    }

    #[test]
    fn openai_tool_followup_keeps_small_tool_output_raw() {
        let tool_call = ToolCall {
            call_id: Some("call_small".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src/main.rs" }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.read_file".to_string(),
            status: "ok".to_string(),
            output: "small output".to_string(),
            duration_ms: 3,
        };

        let message = openai_followup_tool_result_message(&tool_call, &tool_result);

        assert_eq!(
            message.get("content").and_then(Value::as_str),
            Some("small output")
        );
    }

    #[test]
    fn openai_tool_followup_truncates_large_tool_output() {
        let tool_call = ToolCall {
            call_id: Some("call_large".to_string()),
            name: "workspace.gather_context".to_string(),
            arguments: json!({ "paths": ["src"] }),
            plan: None,
        };
        let large_output = format!(
            "{{\"files\":\"{}\",\"tail\":\"{}\"}}",
            "a".repeat(OPENAI_TOOL_RESULT_INLINE_LIMIT_CHARS),
            "z".repeat(4_000)
        );
        let tool_result = ToolResult {
            tool_name: "workspace.gather_context".to_string(),
            status: "ok".to_string(),
            output: large_output.clone(),
            duration_ms: 42,
        };

        let message = openai_followup_tool_result_message(&tool_call, &tool_result);
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .expect("content string");
        let payload: Value = serde_json::from_str(content).expect("compressed json payload");

        assert!(content.chars().count() < large_output.chars().count());
        assert_eq!(
            payload
                .get("compression")
                .and_then(|value| value.get("applied"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            payload
                .get("summary")
                .and_then(|value| value.get("kind"))
                .and_then(Value::as_str),
            Some("json_object")
        );
        assert_eq!(
            payload
                .get("summary")
                .and_then(|value| value.get("keys"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(payload.get("head").and_then(Value::as_str).is_some());
        assert!(payload.get("tail").and_then(Value::as_str).is_some());
    }

    #[test]
    fn openai_sse_reader_streams_reasoning_and_text_in_order() {
        let raw_text = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先看工具输出。\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第一段\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"第二段\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |delta| deltas.push(delta),
        )
        .expect("stream message should parse");

        assert_eq!(message.output_text, "第一段第二段");
        assert_eq!(message.reasoning_content.as_deref(), Some("先看工具输出。"));
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Reasoning("先看工具输出。".to_string()),
                ProviderStreamChunk::Text("第一段".to_string()),
                ProviderStreamChunk::Text("第二段".to_string())
            ]
        );
    }

    #[test]
    fn openai_sse_string_collector_still_parses_full_payload() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"beta\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut deltas = Vec::new();

        let message = collect_openai_sse_message(raw_text, &mut |delta| deltas.push(delta))
            .expect("string helper should parse");

        assert_eq!(message.output_text, "alphabeta");
        assert_eq!(message.reasoning_content, None);
        assert_eq!(
            deltas,
            vec![
                ProviderStreamChunk::Text("alpha".to_string()),
                ProviderStreamChunk::Text("beta".to_string())
            ]
        );
    }

    #[test]
    fn openai_sse_reader_extracts_usage_from_terminal_chunk() {
        let raw_text = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"alpha\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"prompt_cache_hit_tokens\":48,\"completion_tokens\":32,\"total_tokens\":152}}\n\n",
            "data: [DONE]\n\n"
        );

        let message = collect_openai_sse_message_from_reader(
            std::io::Cursor::new(raw_text.as_bytes()),
            "unit-test",
            &mut |_delta| {},
        )
        .expect("stream message should parse");

        let usage = message.token_usage.expect("usage should be captured");
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.cache_hit_input_tokens, Some(48));
        assert_eq!(usage.output_tokens, Some(32));
        assert_eq!(usage.total_tokens, Some(152));
    }

    #[test]
    fn openai_reasoning_tool_followup_stream_attempts_live_stream_before_fallback() {
        let config = ResolvedProviderSelection {
            requested_name: "ppx".to_string(),
            provider_name: "ppx".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "http://127.0.0.1:1/v1".to_string(),
            api_key_env_var: "PPX_API_KEY".to_string(),
            api_key: Some("test".to_string()),
            model: "gpt-5.4".to_string(),
            temperature: 0.2,
            max_output_tokens: 8192,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
            },
        };
        let manager = ProviderManager::new(config);
        let request = ProviderRequest {
            model: "gpt-5.4".to_string(),
            input: vec![ProviderMessage::user("继续总结工具结果")],
            images: Vec::new(),
            native_messages: Vec::new(),
            observation: ProviderRequestObservation::default(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_followup".to_string()),
            name: "workspace.gather_context".to_string(),
            arguments: json!({ "paths": ["src-tauri/src/agent/provider.rs"] }),
            plan: None,
        };
        let tool_result = ToolResult {
            tool_name: "workspace.gather_context".to_string(),
            status: "ok".to_string(),
            output: "{\"summary\":{\"text\":\"已读取 provider follow-up 相关实现\"}}".to_string(),
            duration_ms: 18,
        };
        let mut deltas = Vec::new();

        let response = manager
            .continue_with_tool_result_stream(
                &request,
                &[ToolDefinition {
                    name: "workspace.gather_context",
                    description: "gather context",
                    input_schema: json!({ "type": "object" }),
                }],
                None,
                &tool_call,
                &tool_result,
                |delta| deltas.push(delta),
            )
            .expect("follow-up should degrade after stream failure");

        assert_eq!(
            response.provider_source,
            "provider_followup_stream_sync_fallback"
        );
        assert_eq!(response.provider_mode, "fallback");
        assert!(response
            .fallback_reason
            .as_deref()
            .unwrap_or_default()
            .contains("stream_followup_failed"));
        assert!(response
            .fallback_reason
            .as_deref()
            .unwrap_or_default()
            .contains("provider_followup_failed"));
        assert!(deltas.is_empty());
    }
}
