use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    OpenRouter,
    DeepSeek,
    Ollama,
    Mock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub protocol: ProviderProtocol,
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone)]
struct ProviderPreset {
    kind: ProviderKind,
    name: &'static str,
    protocol: ProviderProtocol,
    default_base_url: &'static str,
    default_model: &'static str,
    api_key_env: Option<&'static str>,
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
    config: ProviderConfig,
    requested_name: String,
}

impl ProviderManager {
    pub fn from_env() -> Self {
        let provider_name =
            read_env_var("PONY_PROVIDER").unwrap_or_else(infer_default_provider_name);
        let preset = lookup_preset(&provider_name).unwrap_or_else(mock_preset);

        let protocol = read_env_var("PONY_PROTOCOL")
            .and_then(|value| parse_protocol(&value))
            .unwrap_or_else(|| preset.protocol.clone());
        let base_url = read_env_var("PONY_BASE_URL")
            .unwrap_or_else(|| preset.default_base_url.to_string());
        let model =
            read_env_var("PONY_MODEL").unwrap_or_else(|| preset.default_model.to_string());
        let api_key = read_env_var("PONY_API_KEY")
            .or_else(|| preset.api_key_env.and_then(read_env_var));
        let temperature = read_env_var("PONY_TEMPERATURE")
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.2);
        let max_output_tokens = read_env_var("PONY_MAX_OUTPUT_TOKENS")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(800);

        let should_mock = matches!(preset.kind, ProviderKind::Mock)
            || (api_key.is_none() && !matches!(preset.kind, ProviderKind::Ollama));

        let config = if should_mock {
            ProviderConfig {
                kind: ProviderKind::Mock,
                protocol,
                name: "mock".to_string(),
                base_url,
                api_key: None,
                model,
                temperature,
                max_output_tokens,
            }
        } else {
            ProviderConfig {
                kind: preset.kind,
                protocol,
                name: preset.name.to_string(),
                base_url,
                api_key,
                model,
                temperature,
                max_output_tokens,
            }
        };

        Self {
            config,
            requested_name: provider_name,
        }
    }

    pub fn requested_name(&self) -> &str {
        &self.requested_name
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn protocol_label(&self) -> &'static str {
        match self.config.protocol {
            ProviderProtocol::Responses => "responses",
            ProviderProtocol::ChatCompletions => "chat_completions",
        }
    }

    pub fn temperature(&self) -> f32 {
        self.config.temperature
    }

    pub fn max_output_tokens(&self) -> u32 {
        self.config.max_output_tokens
    }

    pub fn send(&self, request: &ProviderRequest) -> ProviderResponse {
        if matches!(self.config.kind, ProviderKind::Mock) {
            let reason = if self.requested_name.eq_ignore_ascii_case("mock") {
                "当前显式选择了 mock provider。".to_string()
            } else {
                format!(
                    "未读取到 {} 对应的 API Key，已回退到本地 mock。",
                    self.requested_name
                )
            };
            return self.mock_response(request, Some(reason));
        }

        let result = match self.config.protocol {
            ProviderProtocol::Responses => self.send_responses_request(request),
            ProviderProtocol::ChatCompletions => self.send_chat_completions_request(request),
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

    fn send_responses_request(&self, request: &ProviderRequest) -> Result<String, String> {
        let endpoint = format!("{}/responses", self.config.base_url.trim_end_matches('/'));
        let body = json!({
            "model": request.model,
            "input": request.input,
            "stream": false,
            "temperature": request.temperature,
            "max_output_tokens": request.max_output_tokens
        });
        let payload = self.post_json(&endpoint, &body)?;

        payload
            .get("output_text")
            .and_then(Value::as_str)
            .map(|text| text.to_string())
            .or_else(|| extract_responses_output_text(&payload))
            .ok_or_else(|| "responses 接口没有返回可读文本".to_string())
    }

    fn send_chat_completions_request(&self, request: &ProviderRequest) -> Result<String, String> {
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
        let payload = self.post_json(&endpoint, &body)?;

        extract_chat_output_text(&payload)
            .ok_or_else(|| "chat completions 接口没有返回可读文本".to_string())
    }

    fn post_json(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|error| format!("创建 HTTP client 失败：{}", error))?;

        let mut builder = client.post(endpoint).json(body);

        if let Some(api_key) = self.config.api_key.as_deref() {
            builder = builder.bearer_auth(api_key);
        }

        builder
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
                self.requested_name,
                self.protocol_label(),
                last_user
            ),
            provider_mode: "mock".to_string(),
            fallback_reason,
        }
    }
}

fn infer_default_provider_name() -> String {
    if read_env_var("OPENAI_API_KEY").is_some() {
        return "openai".to_string();
    }
    if read_env_var("OPENROUTER_API_KEY").is_some() {
        return "openrouter".to_string();
    }
    if read_env_var("DEEPSEEK_API_KEY").is_some() {
        return "deepseek".to_string();
    }
    if read_env_var("OLLAMA_HOST").is_some() {
        return "ollama".to_string();
    }
    "mock".to_string()
}

fn lookup_preset(name: &str) -> Option<ProviderPreset> {
    match name.to_ascii_lowercase().as_str() {
        "openai" => Some(ProviderPreset {
            kind: ProviderKind::OpenAi,
            name: "openai",
            protocol: ProviderProtocol::Responses,
            default_base_url: "https://api.openai.com/v1",
            default_model: "gpt-4.1-mini",
            api_key_env: Some("OPENAI_API_KEY"),
        }),
        "openrouter" => Some(ProviderPreset {
            kind: ProviderKind::OpenRouter,
            name: "openrouter",
            protocol: ProviderProtocol::ChatCompletions,
            default_base_url: "https://openrouter.ai/api/v1",
            default_model: "openai/gpt-4.1-mini",
            api_key_env: Some("OPENROUTER_API_KEY"),
        }),
        "deepseek" => Some(ProviderPreset {
            kind: ProviderKind::DeepSeek,
            name: "deepseek",
            protocol: ProviderProtocol::ChatCompletions,
            default_base_url: "https://api.deepseek.com/v1",
            default_model: "deepseek-chat",
            api_key_env: Some("DEEPSEEK_API_KEY"),
        }),
        "ollama" => Some(ProviderPreset {
            kind: ProviderKind::Ollama,
            name: "ollama",
            protocol: ProviderProtocol::ChatCompletions,
            default_base_url: "http://127.0.0.1:11434/v1",
            default_model: "qwen2.5:7b",
            api_key_env: None,
        }),
        "mock" => Some(mock_preset()),
        _ => None,
    }
}

fn mock_preset() -> ProviderPreset {
    ProviderPreset {
        kind: ProviderKind::Mock,
        name: "mock",
        protocol: ProviderProtocol::Responses,
        default_base_url: "https://api.openai.com/v1",
        default_model: "gpt-4.1-mini",
        api_key_env: None,
    }
}

fn parse_protocol(value: &str) -> Option<ProviderProtocol> {
    match value.to_ascii_lowercase().as_str() {
        "responses" => Some(ProviderProtocol::Responses),
        "chat" | "chat_completions" | "chat-completions" => {
            Some(ProviderProtocol::ChatCompletions)
        }
        _ => None,
    }
}

fn to_chat_role(role: &ProviderRole) -> &'static str {
    match role {
        ProviderRole::System | ProviderRole::Developer => "system",
        ProviderRole::User => "user",
    }
}

fn extract_responses_output_text(payload: &Value) -> Option<String> {
    let mut parts = Vec::new();

    for item in payload.get("output")?.as_array()? {
        if let Some(content_items) = item.get("content").and_then(Value::as_array) {
            for content in content_items {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                    continue;
                }

                if let Some(text) = content
                    .get("text")
                    .and_then(|value| value.get("value"))
                    .and_then(Value::as_str)
                {
                    parts.push(text.to_string());
                }
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
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

fn read_env_var(name: &str) -> Option<String> {
    if let Ok(value) = env::var(name) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }

    #[cfg(windows)]
    {
        if let Some(value) = read_windows_env_scope(name, "User") {
            return Some(value);
        }

        if let Some(value) = read_windows_env_scope(name, "Machine") {
            return Some(value);
        }
    }

    None
}

#[cfg(windows)]
fn read_windows_env_scope(name: &str, scope: &str) -> Option<String> {
    let script = format!(
        "[Environment]::GetEnvironmentVariable('{}','{}')",
        name.replace('\'', "''"),
        scope.replace('\'', "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
