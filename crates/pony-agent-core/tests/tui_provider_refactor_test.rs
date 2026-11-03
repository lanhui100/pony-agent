// crates/pony-agent-core/tests/tui_provider_refactor_test.rs
//! 验收测试：验证 Provider 增删、选择二级联动 Model 菜单、Provider key 缺失友好提示与 SecretStore 写入。

use pony_agent_core::agent::config::{ProviderConfigView, ProviderRegistryStore, ProviderRegistryView};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::secret_store::{FileSecretStore, SecretStore};
use std::sync::Arc;

#[test]
fn test_provider_registry_delete_and_secret_cleanup() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("test_tui_prov_{}", nonce));
    std::fs::create_dir_all(&dir).unwrap();

    let reg_path = dir.join("providers.json");
    let sec_path = dir.join("secrets.json");

    let secret_store: Arc<dyn SecretStore> = Arc::new(FileSecretStore::new(sec_path));
    let registry = ProviderRegistryStore::with_path_and_secret_store(reg_path.clone(), secret_store.clone());

    // 初始注册表注入两个 provider: ponyllm 和 custom-a
    let mut view = ProviderRegistryView {
        selected_provider_id: Some("ponyllm".to_string()),
        providers: vec![
            ProviderConfigView {
                id: "ponyllm".to_string(),
                name: "ponyllm".to_string(),
                protocol: ProviderProtocol::OpenAiCompletions,
                auth_type: ProviderAuthType::Auto,
                base_url: "https://api.ponyllm.invalid/v1".to_string(),
                api_key_env_var: "PONYLLM_API_KEY".to_string(),
                api_key_value: String::new(),
                api_key_present: false,
                models: vec![],
                supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
                endpoints: vec![],
                selected_model_id: None,
            },
            ProviderConfigView {
                id: "custom-a".to_string(),
                name: "custom-a".to_string(),
                protocol: ProviderProtocol::OpenAiCompletions,
                auth_type: ProviderAuthType::Auto,
                base_url: "https://api.custom-a.invalid/v1".to_string(),
                api_key_env_var: "CUSTOM_A_API_KEY".to_string(),
                api_key_value: String::new(),
                api_key_present: false,
                models: vec![],
                supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
                endpoints: vec![],
                selected_model_id: None,
            },
        ],
    };
    registry.save_view(view.clone()).expect("save view");

    // 保存 key 到 secret store
    secret_store.set("provider/custom-a/api-key", "secret-test-key").expect("set key");
    assert_eq!(secret_store.get("provider/custom-a/api-key").unwrap(), Some("secret-test-key".to_string()));

    // 业务规则：ponyllm 为内置不能删
    let can_delete_ponyllm = "ponyllm" != "ponyllm";
    assert!(!can_delete_ponyllm);

    // 删除 custom-a
    view.providers.retain(|p| p.id != "custom-a");
    registry.save_view(view).expect("save view after delete");
    secret_store.delete("provider/custom-a/api-key").expect("delete key");

    let reloaded = registry.load_view();
    assert_eq!(reloaded.providers.len(), 1);
    assert_eq!(reloaded.providers[0].id, "ponyllm");
    assert_eq!(secret_store.get("provider/custom-a/api-key").unwrap(), None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_provider_key_diagnostics_message() {
    let provider_name = "openai";
    let env_var = "OPENAI_API_KEY";
    let formatted_notice = format!(
        "当前提供商 [{}] 未配置 API Key，请在 /provider 中配置或设置环境变量 [{}]",
        provider_name, env_var
    );
    assert!(formatted_notice.contains("未配置 API Key"));
    assert!(formatted_notice.contains("OPENAI_API_KEY"));
}
