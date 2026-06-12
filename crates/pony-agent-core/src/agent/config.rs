use crate::agent::provider::ProviderProtocol;
use crate::agent::secret_store::{default_secret_store, SecretStore};
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8192;
const LEGACY_DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1200;

struct CapabilityCatalogEntry {
    protocol: Option<&'static str>,
    patterns: &'static [&'static str],
    preset: ProviderCapabilityPreset,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderCapabilityPreset {
    Auto,
    OpenAiChat,
    OpenAiReasoning,
    AnthropicThinking,
    DeepseekChat,
    DeepseekReasoner,
    Custom,
}

impl Default for ProviderCapabilityPreset {
    fn default() -> Self {
        Self::Auto
    }
}

const CAPABILITY_CATALOG: &[CapabilityCatalogEntry] = &[
    CapabilityCatalogEntry {
        protocol: Some("anthropic"),
        patterns: &["claude-3-7", "claude-sonnet-4", "claude-opus-4"],
        preset: ProviderCapabilityPreset::AnthropicThinking,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["gpt-5", "gpt-5.4", "gpt-5.5", "o1", "o3", "reason"],
        preset: ProviderCapabilityPreset::OpenAiReasoning,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["gpt-4.1", "vision"],
        preset: ProviderCapabilityPreset::OpenAiChat,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["deepseek-reasoner", "deepseek-r1", "deepseek-v4-pro"],
        preset: ProviderCapabilityPreset::DeepseekReasoner,
    },
    CapabilityCatalogEntry {
        protocol: Some("openai"),
        patterns: &["deepseek-chat", "deepseek-v4-flash"],
        preset: ProviderCapabilityPreset::DeepseekChat,
    },
];

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
    pub capability_preset: ProviderCapabilityPreset,
    #[serde(default)]
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    #[serde(default)]
    pub reasoning_budget_tokens: Option<u32>,
    #[serde(default)]
    pub capabilities: ProviderModelCapabilities,
}

#[derive(Clone)]
struct ProviderModelCapabilityDeclaration {
    capability_preset: ProviderCapabilityPreset,
    capabilities: ProviderModelCapabilities,
}

#[derive(Clone)]
struct ProviderModelUserPolicy {
    temperature: f32,
    max_output_tokens: u32,
    reasoning_effort: Option<ProviderReasoningEffort>,
    reasoning_budget_tokens: Option<u32>,
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
    secret_ref: String,
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
    secret_store: Arc<dyn SecretStore>,
}

impl ProviderRegistryStore {
    pub fn new() -> Self {
        let mut path = config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("pony-agent");
        path.push("providers.json");

        Self::with_path(path)
    }

    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let secret_store = default_secret_store(secret_file_path(&path));
        Self { path, secret_store }
    }

    #[allow(dead_code)]
    pub fn with_path_and_secret_store(
        path: impl Into<PathBuf>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            path: path.into(),
            secret_store,
        }
    }

    pub fn load_view(&self) -> ProviderRegistryView {
        self.build_view_from_storage(self.load_storage())
    }

    pub fn save_view(&self, view: ProviderRegistryView) -> Result<ProviderRegistryView, String> {
        let previous_storage = self.load_storage();
        let input_storage = normalize_storage(storage_from_view(view));
        sync_registry_secrets(
            &previous_storage,
            &input_storage,
            self.secret_store.as_ref(),
        )?;
        let storage = sanitized_storage_for_persistence(&input_storage);
        self.write_storage(&storage)?;

        Ok(self.build_view_from_storage(storage))
    }
    pub fn save_view_without_env_sync(
        &self,
        view: ProviderRegistryView,
    ) -> Result<ProviderRegistryView, String> {
        let storage =
            sanitized_storage_for_persistence(&normalize_storage(storage_from_view(view)));

        self.write_storage(&storage)?;

        Ok(self.build_view_from_storage(storage))
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
            api_key: resolve_provider_api_key(provider, self.secret_store.as_ref()),
            model: model.model.clone(),
            temperature: model.temperature,
            max_output_tokens: model.max_output_tokens,
            reasoning_effort: model.reasoning_effort.clone(),
            reasoning_budget_tokens: model.reasoning_budget_tokens,
            capabilities: model.capabilities.clone(),
        }
    }

    fn build_view_from_storage(&self, storage: ProviderRegistryStorage) -> ProviderRegistryView {
        build_view_from_storage(storage, self.secret_store.as_ref())
    }

    fn write_storage(&self, storage: &ProviderRegistryStorage) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create provider config directory failed: {}", error))?;
        }

        let json = serde_json::to_string_pretty(storage)
            .map_err(|error| format!("serialize provider config failed: {}", error))?;

        fs::write(&self.path, json)
            .map_err(|error| format!("write provider config failed: {}", error))
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

fn storage_from_view(view: ProviderRegistryView) -> ProviderRegistryStorage {
    ProviderRegistryStorage {
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
                    secret_ref: String::new(),
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
    }
}

fn secret_file_path(registry_path: &Path) -> PathBuf {
    registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("secrets.json")
}

fn default_secret_ref(provider_id: &str) -> String {
    format!("provider/{}/api-key", provider_id.trim())
}

fn build_view_from_storage(
    storage: ProviderRegistryStorage,
    secret_store: &dyn SecretStore,
) -> ProviderRegistryView {
    ProviderRegistryView {
        selected_provider_id: storage.selected_provider_id.clone(),
        providers: storage
            .providers
            .into_iter()
            .map(|provider| {
                let api_key_value = provider.api_key_value.clone();
                let api_key_present = resolve_provider_api_key(&provider, secret_store).is_some();

                ProviderConfigView {
                    id: provider.id,
                    name: provider.name,
                    protocol: provider.protocol,
                    base_url: provider.base_url,
                    api_key_env_var: provider.api_key_env_var,
                    api_key_present,
                    api_key_value,
                    models: provider.models,
                    selected_model_id: provider.selected_model_id,
                }
            })
            .collect(),
    }
}

fn resolve_provider_api_key(
    provider: &ProviderConfigStorage,
    secret_store: &dyn SecretStore,
) -> Option<String> {
    let stored = provider.api_key_value.trim();
    if !stored.is_empty() {
        return Some(stored.to_string());
    }

    if let Ok(Some(secret)) = secret_store.get(&provider.secret_ref) {
        let trimmed = secret.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    read_env_var_best_effort(&provider.api_key_env_var)
}

fn sync_registry_secrets(
    previous: &ProviderRegistryStorage,
    next: &ProviderRegistryStorage,
    secret_store: &dyn SecretStore,
) -> Result<(), String> {
    for provider in &next.providers {
        let value = provider.api_key_value.trim();
        if value.is_empty() {
            continue;
        }

        secret_store.set(&provider.secret_ref, value)?;
    }

    for provider in &previous.providers {
        if next
            .providers
            .iter()
            .any(|item| item.secret_ref == provider.secret_ref)
        {
            continue;
        }

        let _ = secret_store.delete(&provider.secret_ref);
    }

    Ok(())
}

fn sanitized_storage_for_persistence(storage: &ProviderRegistryStorage) -> ProviderRegistryStorage {
    let mut sanitized = storage.clone();
    for provider in &mut sanitized.providers {
        provider.api_key_value.clear();
    }
    sanitized
}

fn read_env_var_best_effort(name: &str) -> Option<String> {
    let process_value = std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let persistent_value = read_persistent_env_var(name);

    if persistent_value.is_some() {
        return persistent_value;
    }

    process_value
}

#[cfg(not(windows))]
fn read_persistent_env_var(_name: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn read_persistent_env_var(name: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .ok()
        .and_then(|key| key.get_value::<String, _>(name).ok());
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
        .ok()
        .and_then(|key| key.get_value::<String, _>(name).ok());

    user.or(machine)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_registry() -> ProviderRegistryStorage {
    ProviderRegistryStorage {
        selected_provider_id: Some("provider-ppx".to_string()),
        providers: default_provider_templates(),
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
        if provider.secret_ref.trim().is_empty() {
            provider.secret_ref = default_secret_ref(&provider.id);
        }
        provider.api_key_value = provider.api_key_value.trim().to_string();
        if provider.models.is_empty() {
            provider.models.push(ProviderModelConfig {
                id: format!("{}-model-default", provider.id),
                name: "默认模型".to_string(),
                model: default_model(&provider.protocol).to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: infer_capability_preset(
                    &provider.protocol,
                    default_model(&provider.protocol),
                ),
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
            let declaration = resolve_model_capability_declaration(
                &provider.protocol,
                &model.model,
                &model.capability_preset,
                model.capabilities.clone(),
            );
            let user_policy = normalize_model_user_policy(model, &declaration.capabilities);
            model.capability_preset = declaration.capability_preset;
            model.capabilities = declaration.capabilities;
            model.temperature = user_policy.temperature;
            model.max_output_tokens = user_policy.max_output_tokens;
            model.reasoning_effort = user_policy.reasoning_effort;
            model.reasoning_budget_tokens = user_policy.reasoning_budget_tokens;
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

fn default_provider_templates() -> Vec<ProviderConfigStorage> {
    vec![
        ProviderConfigStorage {
            id: "provider-ppx".to_string(),
            name: "ppx".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://api.psydo.top/v1".to_string(),
            api_key_env_var: "PPX_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-ppx"),
            api_key_value: String::new(),
            selected_model_id: Some("model-ppx-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-ppx-default".to_string(),
                name: "GPT 5.4".to_string(),
                model: "gpt-5.4".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: default_model_capabilities(&ProviderProtocol::OpenAi, "gpt-5.4"),
            }],
        },
        ProviderConfigStorage {
            id: "provider-openai".to_string(),
            name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-openai"),
            api_key_value: String::new(),
            selected_model_id: Some("model-openai-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-openai-default".to_string(),
                name: "GPT 4.1 Mini".to_string(),
                model: "gpt-4.1-mini".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::OpenAiChat,
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
            secret_ref: default_secret_ref("provider-openrouter"),
            api_key_value: String::new(),
            selected_model_id: Some("model-openrouter-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-openrouter-default".to_string(),
                name: "OpenAI GPT-4.1 Mini".to_string(),
                model: "openai/gpt-4.1-mini".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::OpenAiChat,
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
            secret_ref: default_secret_ref("provider-deepseek"),
            api_key_value: String::new(),
            selected_model_id: Some("model-deepseek-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-deepseek-default".to_string(),
                name: "DeepSeek V4 Flash".to_string(),
                model: "deepseek-v4-flash".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::DeepseekChat,
                reasoning_effort: None,
                reasoning_budget_tokens: None,
                capabilities: default_model_capabilities(
                    &ProviderProtocol::OpenAi,
                    "deepseek-v4-flash",
                ),
            }],
        },
        ProviderConfigStorage {
            id: "provider-anthropic".to_string(),
            name: "anthropic".to_string(),
            protocol: ProviderProtocol::Anthropic,
            base_url: "https://api.anthropic.com/v1".to_string(),
            api_key_env_var: "ANTHROPIC_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-anthropic"),
            api_key_value: String::new(),
            selected_model_id: Some("model-anthropic-default".to_string()),
            models: vec![ProviderModelConfig {
                id: "model-anthropic-default".to_string(),
                name: "Claude Sonnet".to_string(),
                model: "claude-3-7-sonnet-latest".to_string(),
                temperature: 0.2,
                max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                capability_preset: ProviderCapabilityPreset::AnthropicThinking,
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
    preset_model_capabilities(
        &infer_capability_preset(protocol, model_name),
        protocol,
        model_name,
    )
}

fn infer_capability_preset(
    protocol: &ProviderProtocol,
    model_name: &str,
) -> ProviderCapabilityPreset {
    let lower = model_name.to_ascii_lowercase();

    if let Some(entry) = CAPABILITY_CATALOG.iter().find(|entry| {
        catalog_matches_protocol(protocol, entry.protocol)
            && entry.patterns.iter().any(|pattern| lower.contains(pattern))
    }) {
        return entry.preset.clone();
    }

    if matches!(protocol, ProviderProtocol::Anthropic) {
        return ProviderCapabilityPreset::AnthropicThinking;
    }

    ProviderCapabilityPreset::Auto
}

fn preset_model_capabilities(
    preset: &ProviderCapabilityPreset,
    protocol: &ProviderProtocol,
    model_name: &str,
) -> ProviderModelCapabilities {
    let lower = model_name.to_ascii_lowercase();
    let inferred_preset = if matches!(preset, ProviderCapabilityPreset::Auto) {
        infer_capability_preset(protocol, model_name)
    } else {
        preset.clone()
    };

    match inferred_preset {
        ProviderCapabilityPreset::OpenAiChat => ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: false,
        },
        ProviderCapabilityPreset::OpenAiReasoning => ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: true,
        },
        ProviderCapabilityPreset::AnthropicThinking => ProviderModelCapabilities {
            context_window_tokens: Some(200_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: true,
        },
        ProviderCapabilityPreset::DeepseekChat => ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
        },
        ProviderCapabilityPreset::DeepseekReasoner => ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: true,
        },
        ProviderCapabilityPreset::Auto => ProviderModelCapabilities {
            context_window_tokens: Some(match protocol {
                ProviderProtocol::Anthropic => 200_000,
                ProviderProtocol::OpenAi => 128_000,
            }),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: lower.contains("gpt-4.1")
                || lower.contains("claude")
                || lower.contains("vision"),
            supports_reasoning: lower.contains("gpt-5")
                || lower.contains("o1")
                || lower.contains("o3")
                || lower.contains("reason")
                || lower.contains("claude-3-7")
                || lower.contains("deepseek-r1")
                || lower.contains("deepseek-reasoner")
                || lower.contains("deepseek-v4-pro"),
        },
        ProviderCapabilityPreset::Custom => ProviderModelCapabilities::default(),
    }
}

fn default_true() -> bool {
    true
}

fn catalog_matches_protocol(protocol: &ProviderProtocol, expected: Option<&str>) -> bool {
    match expected {
        None => true,
        Some("openai") => matches!(protocol, ProviderProtocol::OpenAi),
        Some("anthropic") => matches!(protocol, ProviderProtocol::Anthropic),
        Some(_) => false,
    }
}

fn normalize_capability_preset(
    protocol: &ProviderProtocol,
    model_name: &str,
    capability_preset: &ProviderCapabilityPreset,
) -> ProviderCapabilityPreset {
    match capability_preset {
        ProviderCapabilityPreset::Custom => ProviderCapabilityPreset::Custom,
        ProviderCapabilityPreset::Auto => {
            if model_name.trim().is_empty() {
                infer_capability_preset(protocol, default_model(protocol))
            } else {
                ProviderCapabilityPreset::Auto
            }
        }
        preset => preset.clone(),
    }
}

fn normalize_model_capabilities(
    protocol: &ProviderProtocol,
    model_name: &str,
    capability_preset: &ProviderCapabilityPreset,
    capabilities: ProviderModelCapabilities,
) -> ProviderModelCapabilities {
    if matches!(capability_preset, ProviderCapabilityPreset::Custom) {
        let defaults = default_model_capabilities(protocol, model_name);
        return ProviderModelCapabilities {
            context_window_tokens: capabilities
                .context_window_tokens
                .or(defaults.context_window_tokens),
            supports_tools: capabilities.supports_tools,
            supports_streaming: capabilities.supports_streaming,
            supports_image_input: capabilities.supports_image_input,
            supports_reasoning: capabilities.supports_reasoning,
        };
    }

    let defaults = preset_model_capabilities(capability_preset, protocol, model_name);
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

    defaults
}

fn resolve_model_capability_declaration(
    protocol: &ProviderProtocol,
    model_name: &str,
    capability_preset: &ProviderCapabilityPreset,
    capabilities: ProviderModelCapabilities,
) -> ProviderModelCapabilityDeclaration {
    let normalized_preset = normalize_capability_preset(protocol, model_name, capability_preset);
    let normalized_capabilities =
        normalize_model_capabilities(protocol, model_name, &normalized_preset, capabilities);

    ProviderModelCapabilityDeclaration {
        capability_preset: normalized_preset,
        capabilities: normalized_capabilities,
    }
}

fn normalize_model_user_policy(
    model: &ProviderModelConfig,
    capabilities: &ProviderModelCapabilities,
) -> ProviderModelUserPolicy {
    ProviderModelUserPolicy {
        temperature: model.temperature,
        max_output_tokens: model.max_output_tokens,
        reasoning_effort: if capabilities.supports_reasoning {
            model.reasoning_effort.clone()
        } else {
            None
        },
        reasoning_budget_tokens: if capabilities.supports_reasoning {
            model.reasoning_budget_tokens
        } else {
            None
        },
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
    fn auto_preset_is_preserved_while_capabilities_are_inferred() {
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
                      "id": "model-openai-auto",
                      "name": "GPT 5 Auto",
                      "model": "gpt-5.5",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilityPreset": "auto"
                    }
                  ],
                  "selected_model_id": "model-openai-auto"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("fixture should parse");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert!(matches!(
            model.capability_preset,
            ProviderCapabilityPreset::Auto
        ));
        assert_eq!(model.capabilities.context_window_tokens, Some(128_000));
        assert!(model.capabilities.supports_reasoning);
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
        assert_eq!(model.temperature, 0.2);
        assert_eq!(model.max_output_tokens, 8192);
    }

    #[test]
    fn custom_capability_preset_preserves_manual_capabilities() {
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
                      "id": "model-openai-custom",
                      "name": "Custom Chat",
                      "model": "gpt-4.1-mini",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilityPreset": "custom",
                      "capabilities": {
                        "contextWindowTokens": 64000,
                        "supportsTools": false,
                        "supportsStreaming": false,
                        "supportsImageInput": true,
                        "supportsReasoning": false
                      }
                    }
                  ],
                  "selected_model_id": "model-openai-custom"
                }
              ],
              "selected_provider_id": "provider-openai"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);
        let model = &normalized.providers[0].models[0];

        assert!(matches!(
            model.capability_preset,
            ProviderCapabilityPreset::Custom
        ));
        assert_eq!(model.capabilities.context_window_tokens, Some(64_000));
        assert!(!model.capabilities.supports_tools);
        assert!(!model.capabilities.supports_streaming);
        assert!(model.capabilities.supports_image_input);
    }

    #[test]
    fn existing_custom_registry_does_not_reinsert_default_providers() {
        let storage = serde_json::from_str::<ProviderRegistryStorage>(
            r#"
            {
              "providers": [
                {
                  "id": "acme-router",
                  "name": "Acme Router",
                  "protocol": "openai",
                  "base_url": "https://router.example/v1",
                  "api_key_env_var": "ACME_ROUTER_API_KEY",
                  "api_key_value": "secret-token",
                  "models": [
                    {
                      "id": "reasoning-alpha",
                      "name": "Reasoning Alpha",
                      "model": "gpt-5-alpha",
                      "temperature": 0.2,
                      "maxOutputTokens": 8192,
                      "capabilityPreset": "open-ai-reasoning"
                    }
                  ],
                  "selected_model_id": "reasoning-alpha"
                }
              ],
              "selected_provider_id": "acme-router"
            }
            "#,
        )
        .expect("storage should deserialize");

        let normalized = normalize_storage(storage);

        assert_eq!(normalized.providers.len(), 1);
        assert_eq!(normalized.providers[0].id, "acme-router");
        assert_eq!(
            normalized.selected_provider_id.as_deref(),
            Some("acme-router")
        );
    }
}
