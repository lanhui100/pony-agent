use crate::agent::provider::ProviderProtocol;
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8192;
const LEGACY_DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1200;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelConfig {
    pub id: String,
    pub name: String,
    pub model: String,
    pub temperature: f32,
    pub max_output_tokens: u32,
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
            }],
        },
    ]
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
