use crate::agent::config::ResolvedProviderSelection;
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
    pub temperature: f32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub output_text: String,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDecision {
    pub output_text: String,
    pub tool_call: Option<ToolCall>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

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
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String>;
    fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(String);
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
                self.send_openai_tool_followup_request(request, tools, tool_call, tool_result)
            }
            ProviderProtocol::Anthropic => {
                self.send_anthropic_tool_followup_request(request, tools, tool_call, tool_result)
            }
        }
    }

    pub fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        mut on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(String),
    {
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
                tool_call,
                tool_result,
                &mut on_delta,
            ),
            ProviderProtocol::Anthropic => self.send_anthropic_tool_followup_stream_request(
                request,
                tools,
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
        let body = json!({
            "model": request.model,
            "messages": openai_messages_with_tool_result(request, tool_call, tool_result),
            "temperature": request.temperature,
            "max_tokens": request.max_output_tokens,
            "stream": false,
            "tool_choice": "none",
            "tools": openai_tools_payload(tools)
        });
        let payload = self.post_openai_json(&endpoint, &body)?;

        let output_text = extract_chat_output_text(&payload)
            .ok_or_else(|| "openai tool follow-up missing text".to_string())?;

        Ok(ProviderResponse {
            output_text: output_text.clone(),
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
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(String),
    {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        provider_log(format!(
            "request:openai followup-stream endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let body = json!({
            "model": request.model,
            "messages": openai_messages_with_tool_result(request, tool_call, tool_result),
            "temperature": request.temperature,
            "max_tokens": request.max_output_tokens,
            "stream": true,
            "tool_choice": "none",
            "tools": openai_tools_payload(tools)
        });

        self.stream_openai_request(&endpoint, &body, request, on_delta)
    }

    fn send_anthropic_tool_followup_request(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic followup-sync endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let body = json!({
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
        });
        let payload = self.post_anthropic_json(&endpoint, &body)?;

        let output_text = extract_anthropic_output_text(&payload)
            .ok_or_else(|| "anthropic tool follow-up missing text".to_string())?;

        Ok(ProviderResponse {
            output_text: output_text.clone(),
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
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(String),
    {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        provider_log(format!(
            "request:anthropic followup-stream endpoint={} model={} tool={}",
            endpoint, request.model, tool_call.name
        ));
        let body = json!({
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
        });

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
        F: FnMut(String),
    {
        let payload = self.post_openai_json(endpoint, body)?;
        let output_text = extract_chat_output_text(&payload)
            .ok_or_else(|| "openai stream follow-up missing text".to_string())?;
        on_delta(output_text.clone());

        Ok(ProviderResponse {
            output_text: output_text.clone(),
            provider_source: "provider_followup_stream".to_string(),
            provider_mode: "live".to_string(),
            fallback_reason: None,
            token_usage: Some(
                extract_openai_usage(&payload)
                    .unwrap_or_else(|| estimate_token_usage(request, &output_text)),
            ),
        })
    }

    fn stream_anthropic_request<F>(
        &self,
        endpoint: &str,
        body: &Value,
        request: &ProviderRequest,
        on_delta: &mut F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(String),
    {
        let payload = self.post_anthropic_json(endpoint, body)?;
        let output_text = extract_anthropic_output_text(&payload)
            .ok_or_else(|| "anthropic stream follow-up missing text".to_string())?;
        on_delta(output_text.clone());

        Ok(ProviderResponse {
            output_text: output_text.clone(),
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
        let messages = request
            .input
            .iter()
            .map(|message| {
                json!({
                    "role": to_chat_role(&message.role),
                    "content": message.content
                })
            })
            .collect::<Vec<_>>();
        let body = json!({
            "model": request.model,
            "messages": messages,
            "temperature": request.temperature,
            "max_tokens": request.max_output_tokens,
            "stream": false,
            "tool_choice": "auto",
            "tools": openai_tools_payload(tools)
        });
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
        let body = json!({
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
        });
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
        tool_call: &ToolCall,
        tool_result: &ToolResult,
    ) -> Result<ProviderResponse, String> {
        ProviderManager::continue_with_tool_result(self, request, tools, tool_call, tool_result)
    }

    fn continue_with_tool_result_stream<F>(
        &self,
        request: &ProviderRequest,
        tools: &[ToolDefinition],
        tool_call: &ToolCall,
        tool_result: &ToolResult,
        on_delta: F,
    ) -> Result<ProviderResponse, String>
    where
        F: FnMut(String),
    {
        ProviderManager::continue_with_tool_result_stream(
            self,
            request,
            tools,
            tool_call,
            tool_result,
            on_delta,
        )
    }
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
    tool_call: &ToolCall,
    tool_result: &ToolResult,
) -> Vec<Value> {
    let mut messages = request
        .input
        .iter()
        .map(|message| {
            json!({
                "role": to_chat_role(&message.role),
                "content": message.content
            })
        })
        .collect::<Vec<_>>();

    messages.push(json!({
        "role": "assistant",
        "content": Value::Null,
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
    }));

    messages.push(json!({
        "role": "tool",
        "tool_call_id": tool_call.call_id.clone().unwrap_or_else(|| "tool_call_local".to_string()),
        "content": tool_result.output.clone()
    }));

    messages
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
