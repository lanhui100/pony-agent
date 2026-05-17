use crate::agent::config::ResolvedProviderSelection;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

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
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
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

    pub fn send(&self, request: &ProviderRequest) -> ProviderResponse {
        if self
            .config
            .requested_name
            .eq_ignore_ascii_case("mock")
            || self.config.api_key.is_none()
        {
            let reason = if self.config.requested_name.eq_ignore_ascii_case("mock") {
                "当前显式选择了 mock provider。".to_string()
            } else {
                format!(
                    "未读取到 {}（provider={}）的值，已回退到本地 mock。",
                    self.config.api_key_env_var,
                    self.config.provider_name
                )
            };
            return self.mock_response(request, Some(reason));
        }

        let result = match self.config.protocol {
            ProviderProtocol::OpenAi => self.send_openai_request(request),
            ProviderProtocol::Anthropic => self.send_anthropic_request(request),
        };

        match result {
            Ok(output_text) => ProviderResponse {
                output_text,
                provider_mode: "live".to_string(),
                fallback_reason: None,
            },
            Err(error) => self.mock_response(
                request,
                Some(format!("真实 provider 请求失败，已回退到本地 mock：{}", error)),
            ),
        }
    }

    fn send_openai_request(&self, request: &ProviderRequest) -> Result<String, String> {
        let endpoint = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
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
            "stream": false
        });
        let payload = self.post_openai_json(&endpoint, &body)?;

        extract_chat_output_text(&payload)
            .ok_or_else(|| "chat completions 接口没有返回可读文本".to_string())
    }

    fn send_anthropic_request(&self, request: &ProviderRequest) -> Result<String, String> {
        let endpoint = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        let system = request
            .input
            .iter()
            .filter(|message| {
                matches!(message.role, ProviderRole::System | ProviderRole::Developer)
            })
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        let messages = request
            .input
            .iter()
            .filter(|message| matches!(message.role, ProviderRole::User))
            .map(|message| {
                json!({
                    "role": "user",
                    "content": message.content
                })
            })
            .collect::<Vec<_>>();

        let body = json!({
            "model": request.model,
            "system": system,
            "messages": if messages.is_empty() {
                vec![json!({
                    "role": "user",
                    "content": "请解释当前运行状态。"
                })]
            } else {
                messages
            },
            "temperature": request.temperature,
            "max_tokens": request.max_output_tokens
        });
        let payload = self.post_anthropic_json(&endpoint, &body)?;

        extract_anthropic_output_text(&payload)
            .ok_or_else(|| "anthropic messages 接口没有返回可读文本".to_string())
    }

    fn post_openai_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|error| format!("创建 HTTP client 失败：{}", error))?;

        client
            .post(endpoint)
            .bearer_auth(
                self.config
                    .api_key
                    .as_deref()
                    .ok_or_else(|| "provider 缺少 API Key".to_string())?,
            )
            .json(body)
            .send()
            .map_err(|error| format!("调用 provider 失败：{}", error))?
            .error_for_status()
            .map_err(|error| format!("provider 返回错误状态：{}", error))?
            .json::<Value>()
            .map_err(|error| format!("解析 provider 返回失败：{}", error))
    }

    fn post_anthropic_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|error| format!("创建 HTTP client 失败：{}", error))?;

        client
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
            .map_err(|error| format!("调用 provider 失败：{}", error))?
            .error_for_status()
            .map_err(|error| format!("provider 返回错误状态：{}", error))?
            .json::<Value>()
            .map_err(|error| format!("解析 provider 返回失败：{}", error))
    }

    fn mock_response(
        &self,
        request: &ProviderRequest,
        fallback_reason: Option<String>,
    ) -> ProviderResponse {
        let last_user = request
            .input
            .iter()
            .rev()
            .find(|message| matches!(message.role, ProviderRole::User))
            .map(|message| message.content.as_str())
            .unwrap_or("（没有用户消息）");

        ProviderResponse {
            output_text: format!(
                "当前 provider 是 {} / {} 的本地 mock 路径。用户问题：{}。",
                self.config.requested_name,
                self.protocol_label(),
                last_user
            ),
            provider_mode: "mock".to_string(),
            fallback_reason,
        }
    }
}

fn to_chat_role(role: &ProviderRole) -> &'static str {
    match role {
        ProviderRole::System | ProviderRole::Developer => "system",
        ProviderRole::User => "user",
    }
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
