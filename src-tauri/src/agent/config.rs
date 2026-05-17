use crate::agent::provider::ProviderProtocol;
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
        let storage = self.load_storage();

        ProviderRegistryView {
            selected_provider_id: storage.selected_provider_id.clone(),
            providers: storage
                .providers
                .into_iter()
                .map(|provider| {
                    let api_key_value = read_env_var(&provider.api_key_env_var).unwrap_or_default();

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

    pub fn save_view(&self, view: ProviderRegistryView) -> Result<ProviderRegistryView, String> {
        for provider in &view.providers {
            let env_var_name = derive_env_var_name(&provider.name);
            if !provider.api_key_value.trim().is_empty() {
                write_env_var(&env_var_name, &provider.api_key_value)?;
            }
        }

        let storage = normalize_storage(ProviderRegistryStorage {
            providers: view
                .providers
                .into_iter()
                .map(|provider| {
                    let env_var_name = derive_env_var_name(&provider.name);

                    ProviderConfigStorage {
                        id: provider.id,
                        name: provider.name,
                        protocol: provider.protocol,
                        base_url: provider.base_url,
                        api_key_env_var: env_var_name,
                        models: provider.models,
                        selected_model_id: provider.selected_model_id,
                    }
                })
                .collect(),
            selected_provider_id: view.selected_provider_id,
        });

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建配置目录失败：{}", error))?;
        }

        let json = serde_json::to_string_pretty(&storage)
            .map_err(|error| format!("序列化 provider 配置失败：{}", error))?;

        fs::write(&self.path, json).map_err(|error| format!("写入 provider 配置失败：{}", error))?;

        Ok(self.load_view())
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
            api_key: read_env_var(&provider.api_key_env_var),
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

fn default_registry() -> ProviderRegistryStorage {
    ProviderRegistryStorage {
        selected_provider_id: Some("provider-openai".to_string()),
        providers: vec![
            ProviderConfigStorage {
                id: "provider-openai".to_string(),
                name: "openai".to_string(),
                protocol: ProviderProtocol::OpenAi,
                base_url: "https://api.openai.com/v1".to_string(),
                api_key_env_var: "OPENAI_API_KEY".to_string(),
                selected_model_id: Some("model-openai-default".to_string()),
                models: vec![ProviderModelConfig {
                    id: "model-openai-default".to_string(),
                    name: "GPT 4.1 Mini".to_string(),
                    model: "gpt-4.1-mini".to_string(),
                    temperature: 0.2,
                    max_output_tokens: 1200,
                }],
            },
            ProviderConfigStorage {
                id: "provider-anthropic".to_string(),
                name: "anthropic".to_string(),
                protocol: ProviderProtocol::Anthropic,
                base_url: "https://api.anthropic.com/v1".to_string(),
                api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
                selected_model_id: Some("model-anthropic-default".to_string()),
                models: vec![ProviderModelConfig {
                    id: "model-anthropic-default".to_string(),
                    name: "Claude Sonnet".to_string(),
                    model: "claude-3-7-sonnet-latest".to_string(),
                    temperature: 0.2,
                    max_output_tokens: 1200,
                }],
            },
        ],
    }
}

fn normalize_storage(mut storage: ProviderRegistryStorage) -> ProviderRegistryStorage {
    if storage.providers.is_empty() {
        return default_registry();
    }

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
        if provider.models.is_empty() {
            provider.models.push(ProviderModelConfig {
                id: format!("{}-model-default", provider.id),
                name: "默认模型".to_string(),
                model: default_model(&provider.protocol).to_string(),
                temperature: 0.2,
                max_output_tokens: 1200,
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
            if model.max_output_tokens == 0 {
                model.max_output_tokens = 1200;
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
        storage.selected_provider_id = storage.providers.first().map(|provider| provider.id.clone());
    }

    storage
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

fn write_env_var(name: &str, value: &str) -> Result<bool, String> {
    env::set_var(name, value);

    #[cfg(windows)]
    {
        let script = format!(
            "[Environment]::SetEnvironmentVariable('{}','{}','User')",
            name.replace('\'', "''"),
            value.replace('\'', "''")
        );

        Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map_err(|error| format!("调用 PowerShell 写入环境变量失败：{}", error))
            .and_then(|output| {
                if output.status.success() {
                    Ok(true)
                } else {
                    Err(format!(
                        "写入用户环境变量失败：{}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ))
                }
            })
    }

    #[cfg(not(windows))]
    {
        Ok(false)
    }
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
