mod agent {
    pub mod provider {
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
        #[serde(rename_all = "lowercase")]
        pub enum ProviderProtocol {
            OpenAi,
            Anthropic,
        }
    }

    #[path = "../../src/agent/config.rs"]
    pub mod config;
}

use agent::config::{
    ProviderCapabilityPreset, ProviderConfigView, ProviderModelCapabilities, ProviderModelConfig,
    ProviderRegistryStore, ProviderRegistryView,
};
use agent::provider::ProviderProtocol;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn temp_config_root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("pony-agent-provider-regression-{stamp}"))
}

#[test]
fn save_view_normalizes_ids_env_var_and_legacy_reasoning_fields() {
    let _guard = env_lock().lock().expect("env lock poisoned");
    let temp_root = temp_config_root();
    fs::create_dir_all(&temp_root).expect("create temp config root");

    let original_appdata = std::env::var_os("APPDATA");
    let original_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("APPDATA", &temp_root);
    std::env::set_var("XDG_CONFIG_HOME", &temp_root);

    let store = ProviderRegistryStore::new();
    let saved = store
        .save_view(ProviderRegistryView {
            selected_provider_id: Some("missing-provider".to_string()),
            providers: vec![ProviderConfigView {
                id: "".to_string(),
                name: "Acme Router".to_string(),
                protocol: ProviderProtocol::OpenAi,
                base_url: "".to_string(),
                api_key_env_var: "IGNORED".to_string(),
                api_key_value: "  secret-token  ".to_string(),
                api_key_present: false,
                models: vec![ProviderModelConfig {
                    id: "".to_string(),
                    name: "Reasoning Alpha".to_string(),
                    model: "gpt-5-alpha".to_string(),
                    temperature: 0.2,
                    max_output_tokens: 1200,
                    capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                    reasoning_effort: None,
                    reasoning_budget_tokens: Some(0),
                    capabilities: ProviderModelCapabilities::default(),
                }],
                selected_model_id: Some("missing-model".to_string()),
            }],
        })
        .expect("save provider registry");

    let provider = saved
        .providers
        .iter()
        .find(|item| item.name == "Acme Router")
        .expect("custom provider should exist");
    let model = provider.models.first().expect("normalized model");

    assert_eq!(saved.selected_provider_id.as_deref(), Some("acme-router"));
    assert_eq!(provider.id, "acme-router");
    assert_eq!(provider.api_key_env_var, "ACME_ROUTER_API_KEY");
    assert!(provider.api_key_present);
    assert_eq!(provider.api_key_value, "secret-token");
    assert_eq!(provider.base_url, "https://api.openai.com/v1");
    assert_eq!(provider.selected_model_id.as_deref(), Some("reasoning-alpha"));
    assert_eq!(model.id, "reasoning-alpha");
    assert_eq!(model.max_output_tokens, 8192);
    assert_eq!(model.reasoning_budget_tokens, None);
    assert!(model.capabilities.supports_reasoning);

    let loaded = store.load_view();
    assert_eq!(loaded.providers.len(), 1);
    assert_eq!(loaded.providers[0].id, "acme-router");
    assert_eq!(loaded.selected_provider_id.as_deref(), Some("acme-router"));

    match original_appdata {
        Some(value) => std::env::set_var("APPDATA", value),
        None => std::env::remove_var("APPDATA"),
    }
    match original_xdg {
        Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn save_and_reload_keep_custom_only_registry_without_default_reinsertion() {
    let _guard = env_lock().lock().expect("env lock poisoned");
    let temp_root = temp_config_root();
    fs::create_dir_all(&temp_root).expect("create temp config root");

    let original_appdata = std::env::var_os("APPDATA");
    let original_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("APPDATA", &temp_root);
    std::env::set_var("XDG_CONFIG_HOME", &temp_root);

    let store = ProviderRegistryStore::new();
    store
        .save_view(ProviderRegistryView {
            selected_provider_id: Some("ppx".to_string()),
            providers: vec![ProviderConfigView {
                id: "ppx".to_string(),
                name: "ppx".to_string(),
                protocol: ProviderProtocol::OpenAi,
                base_url: "https://ppx.example/v1".to_string(),
                api_key_env_var: "IGNORED".to_string(),
                api_key_value: "ppx-secret".to_string(),
                api_key_present: true,
                models: vec![ProviderModelConfig {
                    id: "ppx-model".to_string(),
                    name: "PPX Model".to_string(),
                    model: "ppx-chat".to_string(),
                    temperature: 0.2,
                    max_output_tokens: 8192,
                    capability_preset: ProviderCapabilityPreset::Auto,
                    reasoning_effort: None,
                    reasoning_budget_tokens: None,
                    capabilities: ProviderModelCapabilities::default(),
                }],
                selected_model_id: Some("ppx-model".to_string()),
            }],
        })
        .expect("save custom-only registry");

    let loaded = store.load_view();

    assert_eq!(loaded.providers.len(), 1);
    assert_eq!(loaded.providers[0].id, "ppx");
    assert_eq!(loaded.providers[0].models.len(), 1);
    assert_eq!(loaded.selected_provider_id.as_deref(), Some("ppx"));

    match original_appdata {
        Some(value) => std::env::set_var("APPDATA", value),
        None => std::env::remove_var("APPDATA"),
    }
    match original_xdg {
        Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn resolve_selection_falls_back_to_selected_provider_and_model() {
    let _guard = env_lock().lock().expect("env lock poisoned");
    let temp_root = temp_config_root();
    fs::create_dir_all(&temp_root).expect("create temp config root");

    let original_appdata = std::env::var_os("APPDATA");
    let original_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("APPDATA", &temp_root);
    std::env::set_var("XDG_CONFIG_HOME", &temp_root);

    let store = ProviderRegistryStore::new();
    store
        .save_view(ProviderRegistryView {
            selected_provider_id: Some("beta-provider".to_string()),
            providers: vec![
                ProviderConfigView {
                    id: "alpha-provider".to_string(),
                    name: "alpha".to_string(),
                    protocol: ProviderProtocol::OpenAi,
                    base_url: "https://alpha.example/v1".to_string(),
                    api_key_env_var: "ALPHA_API_KEY".to_string(),
                    api_key_value: "".to_string(),
                    api_key_present: false,
                    models: vec![ProviderModelConfig {
                        id: "alpha-model".to_string(),
                        name: "Alpha".to_string(),
                        model: "gpt-4.1-mini".to_string(),
                        temperature: 0.1,
                        max_output_tokens: 2048,
                        capability_preset: ProviderCapabilityPreset::OpenAiChat,
                        reasoning_effort: None,
                        reasoning_budget_tokens: None,
                        capabilities: ProviderModelCapabilities::default(),
                    }],
                    selected_model_id: Some("alpha-model".to_string()),
                },
                ProviderConfigView {
                    id: "beta-provider".to_string(),
                    name: "beta".to_string(),
                    protocol: ProviderProtocol::Anthropic,
                    base_url: "https://beta.example/v1".to_string(),
                    api_key_env_var: "BETA_API_KEY".to_string(),
                    api_key_value: "beta-secret".to_string(),
                    api_key_present: true,
                    models: vec![
                        ProviderModelConfig {
                            id: "beta-chat".to_string(),
                            name: "Beta Chat".to_string(),
                            model: "claude-3-7-sonnet-latest".to_string(),
                            temperature: 0.2,
                            max_output_tokens: 4096,
                            capability_preset: ProviderCapabilityPreset::AnthropicThinking,
                            reasoning_effort: None,
                            reasoning_budget_tokens: Some(2048),
                            capabilities: ProviderModelCapabilities::default(),
                        },
                        ProviderModelConfig {
                            id: "beta-fallback".to_string(),
                            name: "Beta Fallback".to_string(),
                            model: "claude-3-7-sonnet-latest".to_string(),
                            temperature: 0.3,
                            max_output_tokens: 8192,
                            capability_preset: ProviderCapabilityPreset::AnthropicThinking,
                            reasoning_effort: None,
                            reasoning_budget_tokens: Some(4096),
                            capabilities: ProviderModelCapabilities::default(),
                        },
                    ],
                    selected_model_id: Some("beta-fallback".to_string()),
                },
            ],
        })
        .expect("save provider registry");

    let resolved = store.resolve_selection(Some("missing"), Some("missing"));

    assert_eq!(resolved.provider_name, "beta");
    assert_eq!(resolved.base_url, "https://beta.example/v1");
    assert_eq!(resolved.model, "claude-3-7-sonnet-latest");
    assert_eq!(resolved.max_output_tokens, 8192);
    assert_eq!(resolved.api_key.as_deref(), Some("beta-secret"));
    assert!(resolved.capabilities.supports_reasoning);
    assert!(resolved.capabilities.supports_image_input);

    match original_appdata {
        Some(value) => std::env::set_var("APPDATA", value),
        None => std::env::remove_var("APPDATA"),
    }
    match original_xdg {
        Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = fs::remove_dir_all(temp_root);
}
