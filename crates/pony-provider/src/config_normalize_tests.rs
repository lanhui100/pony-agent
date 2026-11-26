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

// ---------------------------------------------------------------------------
// `capabilities.contextWindowTokens`：显式声明优先，预设只补缺
// （ADR 2026-11-14-model-declared-context-window-precedence）
// ---------------------------------------------------------------------------

/// 造一份「一个 provider / 一个模型」的注册表并归一化，返回该模型。
///
/// `capabilities` 为 `None` ⇒ 该键整段缺席（走 `undeclared_model_capabilities` 哨兵）。
fn normalized_single_model(
    capability_preset: &str,
    capabilities: Option<serde_json::Value>,
) -> ProviderModelConfig {
    let mut model = serde_json::json!({
        "id": "model-under-test",
        "name": "Model Under Test",
        "model": "gpt-4.1-mini",
        "temperature": 0.2,
        "maxOutputTokens": 64000,
        "capabilityPreset": capability_preset,
    });
    if let Some(capabilities) = capabilities {
        model["capabilities"] = capabilities;
    }

    let storage = serde_json::from_value(serde_json::json!({
        "providers": [{
            "id": "provider-openai",
            "name": "openai",
            "protocol": "openai",
            "base_url": "https://api.openai.com/v1",
            "api_key_env_var": "OPENAI_API_KEY",
            "models": [model],
            "selected_model_id": "model-under-test",
        }],
        "selected_provider_id": "provider-openai",
    }))
    .expect("fixture should parse");

    let mut providers = normalize_storage(storage).providers;
    providers.remove(0).models.remove(0)
}

#[test]
fn preset_keeps_declared_context_window() {
    for declared in [200_000_u32, 1_000_000, 128_000] {
        let model = normalized_single_model(
            "open-ai-chat",
            Some(serde_json::json!({ "contextWindowTokens": declared })),
        );

        assert_eq!(
            model.capabilities.context_window_tokens,
            Some(declared),
            "显式声明的窗口必须原样保留（不允许被预设默认值覆盖）"
        );
        assert!(matches!(
            model.capability_preset,
            ProviderCapabilityPreset::OpenAiChat
        ));
    }
}

#[test]
fn preset_still_owns_boolean_capabilities_when_window_declared() {
    let model = normalized_single_model(
        "deepseek-chat",
        Some(serde_json::json!({
            "contextWindowTokens": 200_000,
            "supportsReasoning": true,
            "supportsImageInput": true,
        })),
    );

    // 窗口例外只限这一项：布尔能力仍由预设决定（deepseek-chat 不支持推理/图片）。
    assert_eq!(model.capabilities.context_window_tokens, Some(200_000));
    assert!(model.capabilities.supports_tools);
    assert!(model.capabilities.supports_streaming);
    assert!(!model.capabilities.supports_reasoning);
    assert!(!model.capabilities.supports_image_input);
    // 预设禁用了推理 ⇒ 推理相关用户策略一并清空（既有口径不回归）。
    assert!(model.reasoning_effort.is_none());
    assert!(model.reasoning_budget_tokens.is_none());
}

#[test]
fn preset_fills_context_window_when_undeclared() {
    // 整段缺席。
    let absent = normalized_single_model("open-ai-chat", None);
    assert_eq!(
        absent.capabilities.context_window_tokens,
        Some(DEFAULT_CONTEXT_WINDOW_TOKENS)
    );

    // 空对象 / 显式 null。
    let empty = normalized_single_model("open-ai-chat", Some(serde_json::json!({})));
    assert_eq!(
        empty.capabilities.context_window_tokens,
        Some(DEFAULT_CONTEXT_WINDOW_TOKENS)
    );
    let null_window =
        normalized_single_model("open-ai-chat", Some(serde_json::json!({ "contextWindowTokens": null })));
    assert_eq!(
        null_window.capabilities.context_window_tokens,
        Some(DEFAULT_CONTEXT_WINDOW_TOKENS)
    );
}

#[test]
fn non_positive_declared_window_is_treated_as_undeclared() {
    for preset in ["open-ai-chat", "auto", "custom"] {
        let model =
            normalized_single_model(preset, Some(serde_json::json!({ "contextWindowTokens": 0 })));

        assert_eq!(
            model.capabilities.context_window_tokens,
            Some(DEFAULT_CONTEXT_WINDOW_TOKENS),
            "0 不是可用窗口（预设 {preset}），必须按未声明补缺"
        );
    }
}

#[test]
fn declared_context_window_survives_save_round_trip() {
    let storage = serde_json::from_value(serde_json::json!({
        "providers": [{
            "id": "provider-anthropic",
            "name": "anthropic",
            "protocol": "anthropic",
            "base_url": "https://api.anthropic.com",
            "api_key_env_var": "ANTHROPIC_API_KEY",
            "models": [{
                "id": "model-sonnet",
                "name": "Claude Sonnet",
                "model": "claude-3-7-sonnet-latest",
                "temperature": 0.2,
                "maxOutputTokens": 64000,
                "capabilityPreset": "anthropic-thinking",
                "capabilities": { "contextWindowTokens": 200_000 },
            }],
            "selected_model_id": "model-sonnet",
        }],
        "selected_provider_id": "provider-anthropic",
    }))
    .expect("fixture should parse");

    let saved = serde_json::to_value(normalize_storage(storage)).expect("should serialize");
    let reloaded = normalize_storage(serde_json::from_value(saved).expect("should parse saved"));

    assert_eq!(
        reloaded.providers[0].models[0]
            .capabilities
            .context_window_tokens,
        Some(200_000),
        "落盘再读回不得把用户声明的窗口改写成预设默认值"
    );
}
