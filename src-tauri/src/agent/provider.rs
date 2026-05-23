use crate::agent::config::{ProviderReasoningEffort, ResolvedProviderSelection};
use crate::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
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
    pub native_messages: Vec<Value>,
    pub temperature: f32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub output_text: String,
    pub reasoning_content: Option<String>,
    pub assistant_message: Option<Value>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDecision {
    pub output_text: String,
    pub tool_call: Option<ToolCall>,
    pub reasoning_content: Option<String>,
    pub assistant_message: Option<Value>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenAiStreamMessage {
    output_text: String,
    reasoning_content: Option<String>,
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
            ProviderProtocol::OpenAi => {
                self.send_openai_tool_followup_request(
                    request,
                    tools,
                    assistant_message,
                    tool_call,
                    tool_result,
                )
            }
            ProviderProtocol::Anthropic => {
                self.send_anthropic_tool_followup_request(
                    request,
                    tools,
                    assistant_message,
                    tool_call,
                    tool_result,
                )
            }
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
            ProviderProtocol::OpenAi => self.send_openai_tool_followup_stream_request(
                request,
                tools,
                assistant_message,
                tool_call,
                tool_result,
                &mut on_delta,
            ),
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
                "tool_choice": "none",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );
        let body = apply_openai_tool_capability(body, tools, &self.config);
        let payload = self.post_openai_json(&endpoint, &body)?;
        let message = first_openai_message(&payload)?;
        let output_text = extract_openai_message_text(message)
            .ok_or_else(|| "openai tool follow-up missing text".to_string())?;

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            reasoning_content: extract_openai_message_reasoning_content(message),
            assistant_message: Some(normalize_openai_assistant_message(message)),
            provider_source: "provider_followup_sync".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_openai_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
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
                "tool_choice": "none",
                "tools": openai_tools_payload(tools)
            }),
            &self.config,
        );

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

        let output_text = extract_anthropic_output_text(&payload)
            .ok_or_else(|| "anthropic tool follow-up missing text".to_string())?;

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            reasoning_content: None,
            assistant_message: None,
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
        let raw_text = self.post_openai_stream_text(endpoint, body)?;
        let message = collect_openai_sse_message(&raw_text, on_delta)?;
        let output_text = message.output_text.clone();

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            reasoning_content: message.reasoning_content.clone(),
            assistant_message: Some(provider_native_assistant_message_with_reasoning(
                &output_text,
                message.reasoning_content.as_deref(),
            )),
            provider_source: "provider_followup_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(estimate_token_usage(request, &output_text)),
        })
    }

    fn post_openai_stream_text(&self, endpoint: &str, body: &Value) -> Result<String, String> {
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
                format_request_error("调用 provider 流式接口失败", &error, started_at.elapsed())
            })?;

        let status = response.status();
        let text = response.text().map_err(|error| {
            format_request_error("读取 provider 流式返回失败", &error, started_at.elapsed())
        })?;

        if !status.is_success() {
            return Err(format!(
                "provider 返回错误状态：{}；耗时={}ms；响应正文：{}",
                status,
                started_at.elapsed().as_millis(),
                text
            ));
        }

        Ok(text)
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
        let output_text = extract_anthropic_output_text(&payload)
            .ok_or_else(|| "anthropic stream follow-up missing text".to_string())?;
        on_delta(ProviderStreamChunk::Text(output_text.clone()));

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            reasoning_content: None,
            assistant_message: Some(provider_native_assistant_message(&output_text)),
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

        Ok(ProviderDecision {
            output_text: extract_openai_message_text(message).unwrap_or_default(),
            tool_call: extract_openai_tool_call(message),
            reasoning_content: extract_openai_message_reasoning_content(message),
            assistant_message: Some(normalize_openai_assistant_message(message)),
            provider_source: "provider_decision".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(extract_openai_usage(&payload).unwrap_or_else(|| {
                estimate_token_usage(
                    request,
                    extract_openai_message_text(message)
                        .as_deref()
                        .unwrap_or_default(),
                )
            })),
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
                "messages": anthropic_user_messages(&request.input),
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

fn collect_openai_sse_message<F>(raw_text: &str, on_delta: &mut F) -> Result<OpenAiStreamMessage, String>
where
    F: FnMut(ProviderStreamChunk),
{
    let mut combined = String::new();
    let mut reasoning = String::new();
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
        reasoning_content: if reasoning.trim().is_empty() {
            None
        } else {
            Some(reasoning)
        },
    })
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
            if tool_calls.as_ref().and_then(Value::as_array).map(|calls| !calls.is_empty()).unwrap_or(false)
            {
                Value::String(extract_openai_message_text(message).unwrap_or_default())
            } else {
                Value::Null
            }
        }
        Some(other) => other.clone(),
    };

    let reasoning_value = message
        .get("reasoning_content")
        .cloned()
        .or_else(|| {
            extract_openai_message_reasoning_content(message).map(Value::String)
        })
        .unwrap_or(Value::Null);

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
    })
}

fn anthropic_system_text(input: &[ProviderMessage]) -> String {
    input
        .iter()
        .filter(|message| matches!(message.role, ProviderRole::System | ProviderRole::Developer))
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn anthropic_user_messages(input: &[ProviderMessage]) -> Vec<Value> {
    input
        .iter()
        .filter(|message| matches!(message.role, ProviderRole::User))
        .map(|message| {
            json!({
                "role": "user",
                "content": message.content
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
    })
}

fn openai_messages_with_tool_result(
    request: &ProviderRequest,
    assistant_message: Option<&Value>,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Vec<Value> {
    let mut messages = openai_request_messages(request);

    messages.push(
        assistant_message
            .map(normalize_openai_assistant_message)
            .unwrap_or_else(|| provider_native_assistant_tool_call_message(None, None, tool_call)),
    );

    messages.push(provider_native_tool_result_message(tool_call, tool_result));

    messages
}

fn openai_request_messages(request: &ProviderRequest) -> Vec<Value> {
    if !request.native_messages.is_empty() {
        return request.native_messages.clone();
    }

    request
        .input
        .iter()
        .map(|message| {
            json!({
                "role": to_chat_role(&message.role),
                "content": message.content
            })
        })
        .collect::<Vec<_>>()
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

pub fn provider_native_tool_result_message(tool_call: &ToolCall, tool_result: &ToolResult) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
        "content": tool_result.output.clone()
    })
}

fn anthropic_messages_with_tool_result(
    request: &ProviderRequest,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Vec<Value> {
    let mut messages = anthropic_user_messages(&request.input);
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
    message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .map(str::to_string)
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
    match delta.get("reasoning_content")? {
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
    let input_chars = request
        .input
        .iter()
        .map(|message| message.content.chars().count())
        .sum::<usize>();
    let output_chars = output_text.chars().count();

    normalize_token_usage(TokenUsage {
        input_tokens: Some(estimate_tokens_from_chars(input_chars)),
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
        output_tokens: usage.output_tokens,
        total_tokens,
    }
}

fn provider_log(message: String) {
    eprintln!("[pony-provider] {}", message);
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
            input: vec![ProviderMessage::user("检查 src-tauri/src/agent/provider.rs")],
            native_messages: Vec::new(),
            temperature: 0.2,
            max_output_tokens: 1024,
        };
        let tool_call = ToolCall {
            call_id: Some("call_123".to_string()),
            name: "workspace.read_file".to_string(),
            arguments: json!({ "path": "src-tauri/src/agent/provider.rs" }),
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
            messages[1]
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("先确认 provider 协议分支，再继续调用工具。")
        );
    }
}
