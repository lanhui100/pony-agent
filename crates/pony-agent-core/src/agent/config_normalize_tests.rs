//! `normalize_storage` 的模型去重与选中回填口径（从 `config.rs` 外移，避开单文件规模棘轮）。

use super::*;

#[test]
fn normalize_storage_deduplicates_model_values_within_provider() {
    let normalized = normalize_storage(ProviderRegistryStorage {
        upstream_policy: UpstreamPolicyStorage::default(),
        failover_provider_ids: Vec::new(),
        providers: vec![ProviderConfigStorage {
            id: "provider-openai".to_string(),
            name: "openai".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: "https://api.openai.com/v1".to_string(),
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "OPENAI_API_KEY".to_string(),
            secret_ref: default_secret_ref("provider-openai"),
            api_key_value: String::new(),
            selected_model_id: Some("model-gpt-5-copy".to_string()),
            models: vec![
                ProviderModelConfig {
                    id: "model-gpt-5".to_string(),
                    name: "GPT-5".to_string(),
                    model: "gpt-5".to_string(),
                    temperature: 0.2,
                    max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                    capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                    reasoning_effort: Some(ProviderReasoningEffort::Medium),
                    reasoning_budget_tokens: None,
                    protocol: Some(ProviderProtocol::OpenAiCompletions),
                    base_url: None,
                    capabilities: ProviderModelCapabilities::default(),
                },
                ProviderModelConfig {
                    id: "model-gpt-5-copy".to_string(),
                    name: "GPT-5 Copy".to_string(),
                    model: " GPT-5 ".to_string(),
                    temperature: 0.2,
                    max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
                    capability_preset: ProviderCapabilityPreset::OpenAiReasoning,
                    reasoning_effort: Some(ProviderReasoningEffort::Medium),
                    reasoning_budget_tokens: None,
                    protocol: Some(ProviderProtocol::OpenAiCompletions),
                    base_url: None,
                    capabilities: ProviderModelCapabilities::default(),
                },
            ],
            supported_protocols: vec![ProviderProtocol::OpenAiCompletions],
            endpoints: vec![ProviderProtocolEndpoint {
                protocol: ProviderProtocol::OpenAiCompletions,
                enabled: true,
                base_url: "https://api.openai.com/v1".to_string(),
                auth_type: ProviderAuthType::Auto,
            }],
        }],
        selected_provider_id: Some("provider-openai".to_string()),
    });
    let provider = &normalized.providers[0];

    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].id, "model-gpt-5");
    assert_eq!(provider.selected_model_id.as_deref(), Some("model-gpt-5"));
}
