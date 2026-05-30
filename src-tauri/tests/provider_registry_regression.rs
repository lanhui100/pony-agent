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

    #[path = "../../src/agent/secret_store.rs"]
    pub mod secret_store;

    #[path = "../../src/agent/config.rs"]
    pub mod config;
}

use agent::config::{
    ProviderCapabilityPreset, ProviderConfigView, ProviderModelCapabilities, ProviderModelConfig,
    ProviderRegistryStore, ProviderRegistryView,
};
use agent::provider::ProviderProtocol;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_config_root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("pony-agent-provider-regression-{stamp}"))
}

fn temp_provider_store() -> (PathBuf, ProviderRegistryStore) {
    let temp_root = temp_config_root();
    let store_path = temp_root.join("pony-agent").join("providers.json");
    let store = ProviderRegistryStore::with_path(&store_path);

    assert!(
        store_path.starts_with(&temp_root),
        "test provider store must stay inside temp root: {}",
        store_path.display()
    );

    (temp_root, store)
}

fn secret_store_file(temp_root: &Path) -> PathBuf {
    temp_root.join("pony-agent").join("secrets.json")
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).expect("read test directory");
    for entry in entries {
        let entry = entry.expect("read test entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

#[test]
fn save_view_normalizes_ids_env_var_and_legacy_reasoning_fields() {
    let _guard = lock_env();
    let (temp_root, store) = temp_provider_store();
    fs::create_dir_all(&temp_root).expect("create temp config root");
    std::env::remove_var("ACME_ROUTER_API_KEY");
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
    assert_eq!(provider.api_key_value, "");
    assert_eq!(provider.base_url, "https://api.openai.com/v1");
    assert_eq!(
        provider.selected_model_id.as_deref(),
        Some("reasoning-alpha")
    );
    assert_eq!(model.id, "reasoning-alpha");
    assert_eq!(model.max_output_tokens, 8192);
    assert_eq!(model.reasoning_budget_tokens, None);
    assert!(model.capabilities.supports_reasoning);

    let loaded = store.load_view();
    assert_eq!(loaded.providers.len(), 1);
    assert_eq!(loaded.providers[0].id, "acme-router");
    assert_eq!(loaded.selected_provider_id.as_deref(), Some("acme-router"));
    assert!(loaded.providers[0].api_key_present);
    assert_eq!(loaded.providers[0].api_key_value, "");

    let resolved = store.resolve_selection(Some("acme-router"), Some("reasoning-alpha"));
    assert_eq!(resolved.api_key.as_deref(), Some("secret-token"));
    let secret_file = secret_store_file(&temp_root);
    let persisted_secret = fs::read_to_string(&secret_file).expect("secret file should exist");
    assert!(persisted_secret.contains("secret-token"));

    std::env::remove_var("ACME_ROUTER_API_KEY");
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn save_and_reload_keep_custom_only_registry_without_default_reinsertion() {
    let _guard = lock_env();
    let (temp_root, store) = temp_provider_store();
    fs::create_dir_all(&temp_root).expect("create temp config root");
    std::env::remove_var("PPX_API_KEY");
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
    assert!(loaded.providers[0].api_key_present);
    assert_eq!(loaded.providers[0].api_key_value, "");
    let resolved = store.resolve_selection(Some("ppx"), Some("ppx-model"));
    assert_eq!(resolved.api_key.as_deref(), Some("ppx-secret"));

    std::env::remove_var("PPX_API_KEY");
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn resolve_selection_falls_back_to_env_when_secret_store_is_empty() {
    let _guard = lock_env();
    let (temp_root, store) = temp_provider_store();
    fs::create_dir_all(&temp_root).expect("create temp config root");
    std::env::set_var("LEGACY_API_KEY", "legacy-secret");

    store
        .save_view_without_env_sync(ProviderRegistryView {
            selected_provider_id: Some("legacy-provider".to_string()),
            providers: vec![ProviderConfigView {
                id: "legacy-provider".to_string(),
                name: "legacy".to_string(),
                protocol: ProviderProtocol::OpenAi,
                base_url: "https://legacy.example/v1".to_string(),
                api_key_env_var: "LEGACY_ONLY_API_KEY".to_string(),
                api_key_value: "".to_string(),
                api_key_present: false,
                models: vec![ProviderModelConfig {
                    id: "legacy-model".to_string(),
                    name: "Legacy".to_string(),
                    model: "gpt-4.1-mini".to_string(),
                    temperature: 0.2,
                    max_output_tokens: 8192,
                    capability_preset: ProviderCapabilityPreset::OpenAiChat,
                    reasoning_effort: None,
                    reasoning_budget_tokens: None,
                    capabilities: ProviderModelCapabilities::default(),
                }],
                selected_model_id: Some("legacy-model".to_string()),
            }],
        })
        .expect("save provider registry without secret sync");

    let resolved = store.resolve_selection(Some("legacy-provider"), Some("legacy-model"));
    assert_eq!(resolved.api_key.as_deref(), Some("legacy-secret"));

    std::env::remove_var("LEGACY_API_KEY");
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn resolve_selection_falls_back_to_selected_provider_and_model() {
    let _guard = lock_env();
    let (temp_root, store) = temp_provider_store();
    fs::create_dir_all(&temp_root).expect("create temp config root");
    std::env::remove_var("ALPHA_API_KEY");
    std::env::remove_var("BETA_API_KEY");
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

    std::env::remove_var("ALPHA_API_KEY");
    std::env::remove_var("BETA_API_KEY");
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn save_view_without_env_sync_keeps_key_out_of_runtime_resolution() {
    let _guard = lock_env();
    let (temp_root, store) = temp_provider_store();
    fs::create_dir_all(&temp_root).expect("create temp config root");
    std::env::remove_var("NO_SYNC_API_KEY");

    store
        .save_view_without_env_sync(ProviderRegistryView {
            selected_provider_id: Some("provider-no-sync".to_string()),
            providers: vec![ProviderConfigView {
                id: "provider-no-sync".to_string(),
                name: "no sync".to_string(),
                protocol: ProviderProtocol::OpenAi,
                base_url: "https://nosync.example/v1".to_string(),
                api_key_env_var: "NO_SYNC_API_KEY".to_string(),
                api_key_value: "no-sync-secret".to_string(),
                api_key_present: true,
                models: vec![ProviderModelConfig {
                    id: "model-no-sync".to_string(),
                    name: "No Sync".to_string(),
                    model: "gpt-4.1-mini".to_string(),
                    temperature: 0.2,
                    max_output_tokens: 8192,
                    capability_preset: ProviderCapabilityPreset::OpenAiChat,
                    reasoning_effort: None,
                    reasoning_budget_tokens: None,
                    capabilities: ProviderModelCapabilities::default(),
                }],
                selected_model_id: Some("model-no-sync".to_string()),
            }],
        })
        .expect("save provider registry without env sync");

    let loaded = store.load_view();
    assert!(!loaded.providers[0].api_key_present);
    assert_eq!(loaded.providers[0].api_key_value, "");

    let resolved = store.resolve_selection(Some("provider-no-sync"), Some("model-no-sync"));
    assert!(resolved.api_key.is_none());
    assert!(std::env::var("NO_SYNC_API_KEY").is_err());

    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn removing_provider_clears_persisted_secret() {
    let _guard = lock_env();
    let (temp_root, store) = temp_provider_store();
    fs::create_dir_all(&temp_root).expect("create temp config root");

    store
        .save_view(ProviderRegistryView {
            selected_provider_id: Some("provider-to-remove".to_string()),
            providers: vec![ProviderConfigView {
                id: "provider-to-remove".to_string(),
                name: "remove-me".to_string(),
                protocol: ProviderProtocol::OpenAi,
                base_url: "https://remove.example/v1".to_string(),
                api_key_env_var: "REMOVE_ME_API_KEY".to_string(),
                api_key_value: "remove-secret".to_string(),
                api_key_present: true,
                models: vec![ProviderModelConfig {
                    id: "remove-model".to_string(),
                    name: "Remove".to_string(),
                    model: "gpt-4.1-mini".to_string(),
                    temperature: 0.2,
                    max_output_tokens: 8192,
                    capability_preset: ProviderCapabilityPreset::OpenAiChat,
                    reasoning_effort: None,
                    reasoning_budget_tokens: None,
                    capabilities: ProviderModelCapabilities::default(),
                }],
                selected_model_id: Some("remove-model".to_string()),
            }],
        })
        .expect("seed provider");

    store
        .save_view(ProviderRegistryView {
            selected_provider_id: None,
            providers: vec![],
        })
        .expect("replace registry");

    let secret_file =
        fs::read_to_string(secret_store_file(&temp_root)).expect("secret file exists");
    assert!(!secret_file.contains("remove-secret"));

    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn tests_must_not_use_default_provider_registry_store() {
    let tests_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let forbidden = format!("{}{}", "ProviderRegistryStore::", "new()");
    let mut files = Vec::new();
    let mut offenders = Vec::new();

    collect_rust_files(&tests_dir, &mut files);

    for path in files {
        let content = fs::read_to_string(&path).expect("read test source");
        if content.contains(&forbidden) {
            offenders.push(path.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "tests must use ProviderRegistryStore::with_path(...) instead of the default constructor: {}",
        offenders.join(", ")
    );
}
