use crate::agent::provider::ProviderProtocol;
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8192;
const LEGACY_DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1200;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelCapabilities {
    pub context_window_tokens: Option<u32>,
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_image_input: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
}

impl Default for ProviderModelCapabilities {
    fn default() -> Self {
        Self {
            context_window_tokens: None,
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelConfig {
    pub id: String,
    pub name: String,
    pub model: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    #[serde(default)]
    pub reasoning_budget_tokens: Option<u32>,
    #[serde(default)]
    pub capabilities: ProviderModelCapabilities,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigView {
    pub id: String,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub api_key_env_var: String,
    pub api_key_value: String,
    pub api_key_present: bool,
    pub models: Vec<ProviderModelConfig>,
    pub selected_model_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegistryView {
    pub providers: Vec<ProviderConfigView>,
    pub selected_provider_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ProviderConfigStorage {
    id: String,
    name: String,
    protocol: ProviderProtocol,
    base_url: String,
    api_key_env_var: String,
    #[serde(default)]
    api_key_value: String,
    models: Vec<ProviderModelConfig>,
    selected_model_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ProviderRegistryStorage {
    providers: Vec<ProviderConfigStorage>,
    selected_provider_id: Option<String>,
}

#[derive(Clone)]
pub struct ResolvedProviderSelection {
    pub requested_name: String,
    pub provider_name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub api_key_env_var: String,
    pub api_key: Option<String>,
    pub model: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    pub reasoning_budget_tokens: Option<u32>,
    pub capabilities: ProviderModelCapabilities,
}

pub trait ProviderSelectionResolver: Send {
    fn resolve_provider_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection;
}

pub struct ProviderRegistryStore {
    path: PathBuf,
}

impl ProviderRegistryStore {
    pub fn new() -> Self {
        let mut path = config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("pony-agent");
        path.push("providers.json");

        Self { path }
    }

    pub fn load_view(&self) -> ProviderRegistryView {
        build_view_from_storage(self.load_storage())
    }

    pub fn save_view(&self, view: ProviderRegistryView) -> Result<ProviderRegistryView, String> {
        let storage = normalize_storage(ProviderRegistryStorage {
            providers: view
                .providers
                .into_iter()
                .map(|provider| {
                    let ProviderConfigView {
                        id,
                        name,
                        protocol,
                        base_url,
                        api_key_env_var: _,
                        api_key_value,
                        api_key_present: _,
                        models,
                        selected_model_id,
                    } = provider;

                    ProviderConfigStorage {
                        id,
                        api_key_env_var: derive_env_var_name(&name),
                        name,
                        protocol,
                        base_url,
                        api_key_value,
                        models,
                        selected_model_id,
                    }
                })
                .collect(),
            selected_provider_id: view.selected_provider_id,
        });

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建 provider 配置目录失败: {}", error))?;
        }

        let json = serde_json::to_string_pretty(&storage)
            .map_err(|error| format!("序列化 provider 配置失败: {}", error))?;

        fs::write(&self.path, json)
            .map_err(|error| format!("写入 provider 配置失败: {}", error))?;

        Ok(build_view_from_storage(storage))
    }

    pub fn save_view_without_env_sync(
        &self,
        view: ProviderRegistryView,
    ) -> Result<ProviderRegistryView, String> {
        self.save_view(view)
    }

    pub fn resolve_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        let storage = self.load_storage();
        let provider = storage
            .providers
            .iter()
            .find(|item| Some(item.id.as_str()) == provider_id)
            .or_else(|| {
                storage
                    .selected_provider_id
                    .as_deref()
                    .and_then(|selected| storage.providers.iter().find(|item| item.id == selected))
            })
            .or_else(|| storage.providers.first())
            .expect("default registry should always contain at least one provider");

        let model = provider
            .models
            .iter()
            .find(|item| Some(item.id.as_str()) == model_id)
            .or_else(|| {
                provider
                    .selected_model_id
                    .as_deref()
                    .and_then(|selected| provider.models.iter().find(|item| item.id == selected))
            })
            .or_else(|| provider.models.first())
            .expect("provider should always contain at least one model");

        ResolvedProviderSelection {
            requested_name: provider.name.clone(),
            provider_name: provider.name.clone(),
            protocol: provider.protocol.clone(),
            base_url: provider.base_url.clone(),
            api_key_env_var: provider.api_key_env_var.clone(),
            api_key: if provider.api_key_value.trim().is_empty() {
                None
            } else {
                Some(provider.api_key_value.clone())
            },
            model: model.model.clone(),
            temperature: model.temperature,
            max_output_tokens: model.max_output_tokens,
            reasoning_effort: model.reasoning_effort.clone(),
            reasoning_budget_tokens: model.reasoning_budget_tokens,
            capabilities: model.capabilities.clone(),
        }
    }

    fn load_storage(&self) -> ProviderRegistryStorage {
        if let Ok(content) = fs::read_to_string(&self.path) {
            if let Ok(storage) = serde_json::from_str::<ProviderRegistryStorage>(&content) {
                return normalize_storage(storage);
            }
        }

        normalize_storage(default_registry())
    }
}

impl ProviderSelectionResolver for ProviderRegistryStore {
    fn resolve_provider_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.resolve_selection(provider_id, model_id)
    }
}

fn build_view_from_storage(storage: ProviderRegistryStorage) -> ProviderRegistryView {
    ProviderRegistryView {
        selected_provider_id: storage.selected_provider_id.clone(),
        providers: storage
            .providers
            .into_iter()
            .map(|provider| {
                let api_key_value = provider.api_key_value.clone();

                ProviderConfigView {
                    id: provider.id,
                    name: provider.name,
                    protocol: provider.protocol,
                    base_url: provider.base_url,
                    api_key_env_var: provider.api_key_env_var,
                    api_key_present: !api_key_value.trim().is_empty(),
                    api_key_value,
                    models: provider.models,
                    selected_model_id: provider.selected_model_id,
                }
            })
            .collect(),
    }
}

fn default_registry() -> ProviderRegistryStorage {
    ProviderRegistryStorage {
        selected_provider_id: Some("provider-openai".to_string()),
        providers: default_provider_templates(),
    }
}

fn normalize_storage(mut storage: ProviderRegistryStorage) -> ProviderRegistryStorage {
    if storage.providers.is_empty() {
        return default_registry();
    }

    merge_missing_default_providers(&mut storage);

    for provider in &mut storage.providers {
        if provider.id.trim().is_empty() {
            provider.id = slugify(&provider.name, "provider");
        }
        if provider.name.trim().is_empty() {
            provider.name = provider.id.clone();
        }
        if provider.base_url.trim().is_empty() {
            provider.base_url = default_base_url(&provider.protocol).to_string();
        }
        provider.api_key_env_var = derive_env_var_name(&provider.name);
        provider.api_key_value = provider.api_key_value.trim().to_string();
        if provider.models.is_empty() {
            provider.models.push(ProviderModelConfig {
                id: format!("{}-model-default", provider.id),
                name: "默认模型".to_string(),
                model: default_model(&provider.protocol).to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: default_model_capabilities(
                    &provider.protocol,
                    default_model(&provider.protocol),
                ),
            });
        }
        for model in &mut provider.models {
            if model.id.trim().is_empty() {
                model.id = slugify(&model.name, "model");
            }
            if model.name.trim().is_empty() {
                model.name = model.model.clone();
            }
            if model.model.trim().is_empty() {
                model.model = default_model(&provider.protocol).to_string();
            }
            if model.max_output_tokens == 0
                || model.max_output_tokens == LEGACY_DEFAULT_MAX_OUTPUT_TOKENS
            {
                model.max_output_tokens = DEFAULT_MAX_OUTPUT_TOKENS;
            }
            if model.reasoning_budget_tokens == Some(0) {
                model.reasoning_budget_tokens = None;
            }
            model.capabilities = normalize_model_capabilities(
                &provider.protocol,
                &model.model,
                model.capabilities.clone(),
            );
            if !model.capabilities.supports_reasoning {
                model.reasoning_effort = None;
                model.reasoning_budget_tokens = None;
            }
        }
        if provider.selected_model_id.is_none()
            || provider
                .models
                .iter()
                .all(|model| Some(model.id.as_str()) != provider.selected_model_id.as_deref())
        {
            provider.selected_model_id = provider.models.first().map(|model| model.id.clone());
        }
    }

    if storage.selected_provider_id.is_none()
        || storage
            .providers
            .iter()
            .all(|provider| Some(provider.id.as_str()) != storage.selected_provider_id.as_deref())
    {
        storage.selected_provider_id = storage
            .providers
            .first()
            .map(|provider| provider.id.clone());
    }

    storage
}

fn merge_missing_default_providers(storage: &mut ProviderRegistryStorage) {
    let default_providers = default_provider_templates();

    for default_provider in default_providers {
        let has_provider = storage.providers.iter().any(|provider| {
            provider.id == default_provider.id
                || provider.name.eq_ignore_ascii_case(&default_provider.name)
        });

        if !has_provider {
            storage.providers.push(default_provider);
        }
    }
}

fn default_provider_templates() -> Vec<ProviderConfigStorage> {
    vec![
        ProviderConfigStorage {
            id: "provider-openai".to_string(),
            name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            api_key_value: String::new(),
            selected_model_id: Some("model-openai-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-openai-default".to_string(),
                name: "GPT 4.1 Mini".to_string(),
                model: "gpt-4.1-mini".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: default_model_capabilities(&ProviderProtocol::OpenAi, "gpt-4.1-mini"),
            }],
        },
        ProviderConfigStorage {
            id: "provider-openrouter".to_string(),
            name: "openrouter".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            api_key_env_var: "OPENROUTER_API_KEY".to_string(),
            api_key_value: String::new(),
            selected_model_id: Some("model-openrouter-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-openrouter-default".to_string(),
                name: "OpenAI GPT-4.1 Mini".to_string(),
                model: "openai/gpt-4.1-mini".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: default_model_capabilities(
                    &ProviderProtocol::OpenAi,
                    "openai/gpt-4.1-mini",
                ),
            }],
        },
        ProviderConfigStorage {
            id: "provider-deepseek".to_string(),
            name: "deepseek".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key_value: String::new(),
            selected_model_id: Some("model-deepseek-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-deepseek-default".to_string(),
                name: "DeepSeek Chat".to_string(),
                model: "deepseek-chat".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: default_model_capabilities(
                    &ProviderProtocol::OpenAi,
                    "deepseek-chat",
                ),
            }],
        },
        ProviderConfigStorage {
            id: "provider-anthropic".to_string(),
            name: "anthropic".to_string(),
            protocol: ProviderProtocol::Anthropic,
            base_url: "https://api.anthropic.com/v1".to_string(),
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            api_key_value: String::new(),
            selected_model_id: Some("model-anthropic-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-anthropic-default".to_string(),
                name: "Claude Sonnet".to_string(),
                model: "claude-3-7-sonnet-latest".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: default_model_capabilities(
                    &ProviderProtocol::Anthropic,
                    "claude-3-7-sonnet-latest",
                ),
            }],
        },
    ]
}

fn default_model_capabilities(
    protocol: &ProviderProtocol,
    model_name: &str,
) -> ProviderModelCapabilities {
    let lower = model_name.to_ascii_lowercase();
    let supports_reasoning = lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("reason")
        || lower.contains("claude-3-7")
        || lower.contains("deepseek-r1")
        || lower.contains("deepseek-reasoner")
        || lower.contains("deepseek-v4-pro");
    let supports_image_input =
        lower.contains("gpt-4.1") || lower.contains("claude") || lower.contains("vision");
    let context_window_tokens = match protocol {
        ProviderProtocol::Anthropic => Some(200_000),
        ProviderProtocol::OpenAi => Some(128_000),
    };

    ProviderModelCapabilities {
        context_window_tokens,
        supports_tools: true,
        supports_streaming: true,
        supports_image_input,
        supports_reasoning,
    }
}

fn default_true() -> bool {
    true
}

fn normalize_model_capabilities(
    protocol: &ProviderProtocol,
    model_name: &str,
    capabilities: ProviderModelCapabilities,
) -> ProviderModelCapabilities {
    let defaults = default_model_capabilities(protocol, model_name);
    let looks_like_legacy_empty = capabilities.context_window_tokens.is_none()
        && !capabilities.supports_tools
        && !capabilities.supports_streaming
        && !capabilities.supports_image_input
        && !capabilities.supports_reasoning;
    let looks_like_implicit_defaults = capabilities.context_window_tokens.is_none()
        && capabilities.supports_tools
        && capabilities.supports_streaming
        && !capabilities.supports_image_input
        && !capabilities.supports_reasoning;

    if looks_like_legacy_empty || looks_like_implicit_defaults {
        return defaults;
    }

    ProviderModelCapabilities {
        context_window_tokens: capabilities
            .context_window_tokens
            .or(defaults.context_window_tokens),
        supports_tools: capabilities.supports_tools,
        supports_streaming: capabilities.supports_streaming,
        supports_image_input: capabilities.supports_image_input,
        supports_reasoning: capabilities.supports_reasoning,
    }
}

fn slugify(value: &str, fallback: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    slug = slug
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

fn derive_env_var_name(provider_name: &str) -> String {
    let mut env_name = provider_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();

    env_name = env_name
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_");

    if env_name.is_empty() {
        "CUSTOM_PROVIDER_API_KEY".to_string()
    } else {
        format!("{}_API_KEY", env_name)
    }
}

fn default_base_url(protocol: &ProviderProtocol) -> &'static str {
    match protocol {
        ProviderProtocol::OpenAi => "https://api.openai.com/v1",
        ProviderProtocol::Anthropic => "https://api.anthropic.com/v1",
    }
}

fn default_model(protocol: &ProviderProtocol) -> &'static str {
    match protocol {
        ProviderProtocol::OpenAi => "gpt-4.1-mini",
        ProviderProtocol::Anthropic => "claude-3-7-sonnet-latest",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_capabilities_fields_use_stable_defaults() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "provider-openai",
                  "name": "openai",
                  "protocol": "openai",
                  "base_url": "https://api.openai.com/v1",
                  "api_key_env_var": "OPENAI_API_KEY",
                  "models": [
                    {
                      "id": "model-openai-default",
                      "name": "GPT 5.5",
                      "model": "gpt-5.5",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilities": {
                        "supportsReasoning": true
                      },
                      "reasoningEffort": "medium",
                      "reasoningBudgetTokens": 2048
                    }
                  ],
                  "selected_model_id": "model-openai-default"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert_eq!(model.capabilities.context_window_tokens, Some(128_000));
        assert!(model.capabilities.supports_tools);
        assert!(model.capabilities.supports_streaming);
        assert!(model.capabilities.supports_reasoning);
        assert_eq!(
            model
                .reasoning_effort
                .as_ref()
                .map(|effort| matches!(effort, ProviderReasoningEffort::Medium)),
            Some(true)
        );
        assert_eq!(model.reasoning_budget_tokens, Some(2048));
    }

    #[test]
    fn disabling_reasoning_clears_reasoning_fields() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "provider-openai",
                  "name": "openai",
                  "protocol": "openai",
                  "base_url": "https://api.openai.com/v1",
                  "api_key_env_var": "OPENAI_API_KEY",
                  "models": [
                    {
                      "id": "model-openai-default",
                      "name": "GPT 4.1 Mini",
                      "model": "gpt-4.1-mini",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilities": {
                        "supportsReasoning": false
                      },
                      "reasoningEffort": "high",
                      "reasoningBudgetTokens": 4096
                    }
                  ],
                  "selected_model_id": "model-openai-default"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert!(!model.capabilities.supports_reasoning);
        assert!(model.reasoning_effort.is_none());
        assert!(model.reasoning_budget_tokens.is_none());
    }
}
