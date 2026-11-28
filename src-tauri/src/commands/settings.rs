//! 应用设置与 provider 注册表命令。

use crate::agent;
use crate::agent::app_settings::AppSettings;
use crate::agent::app_settings::AppSettingsStore;
use crate::agent::config::ProviderRegistryStore;
use crate::agent::config::ProviderRegistryView;
use tauri::AppHandle;

/// 掩码化服务密钥：明文密钥**永不**跨越 IPC 边界回传前端（F4c / T4）。
///
/// - 未配置：返回空串（前端据此判定 `configured == false`）；
/// - 已配置：返回不可逆展示掩码（如 `sk-…a1b2`）；长度 ≤ 8 的短密钥只回长度占位。
pub(crate) fn mask_service_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "•".repeat(chars.len());
    }

    let prefix: String = chars[..3].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}…{suffix}")
}

#[tauri::command]
pub(crate) fn load_app_settings(app: AppHandle) -> AppSettings {
    let settings = AppSettingsStore::new().load_view();
    // D7：启动即按持久化值生效（默认 false ⇒ 零 span 分配、零 egress）。
    pony_agent_core::agent::otel::set_agent_trace_reporting(settings.agent_trace_reporting);
    // Wave-5 K3：成本预算只做软告警（None/<=0 视为未设置），绝不参与任何拦截。
    pony_agent_core::agent::cost::set_budget_usd(settings.cost_budget_usd);
    // Wave-7：用量治理策略同步（None ⇒ 默认策略：enforcement=false，纯软告警）。
    pony_agent_core::agent::usage_governance::set_policy(settings.usage_governance_policy.clone());
    // B125：子代理并发上限（None ⇒ 默认值；落盘值已在保存期 clamp 到 1..=硬顶）。
    pony_agent_core::agent::subagent_runtime::set_subagent_max_concurrent(
        settings.subagent_max_concurrent,
    );
    // PonySentry 凭据分发（settings > env > 构建期烧录 > 缺省）：桌面端也可用 settings 覆盖
    // 构建期烧录的 token；无 token ⇒ fail-closed 零 egress，token 不进日志/Debug。
    let ponysentry_settings = settings.ponysentry_settings_snapshot();
    // wave-51 C1（审计 §4 P0-1）：PonySentry `release` 与安装包发布线对齐 —— 以宿主
    // `package_info().version` 显式覆盖 env `APP_RELEASE` / core crate 版本（空白或脏值不覆盖）。
    let app_version = app.package_info().version.to_string();
    let ponysentry_config = pony_agent_core::agent::ponysentry::PonySentryConfig::from_settings(
        Some(&ponysentry_settings),
    )
    .with_release(Some(app_version.as_str()));
    pony_agent_core::agent::ponysentry::init(ponysentry_config);
    settings
}

#[tauri::command]
pub(crate) fn save_app_settings(settings: AppSettings) -> Result<AppSettings, String> {
    let saved = AppSettingsStore::new().save_view(settings)?;
    // D7：保存即生效，无需重启；以落盘后的回显值为准（避免保存失败后门禁被误翻）。
    pony_agent_core::agent::otel::set_agent_trace_reporting(saved.agent_trace_reporting);
    // Wave-5 K3：预算保存即生效（软告警门槛，不阻断请求）。
    pony_agent_core::agent::cost::set_budget_usd(saved.cost_budget_usd);
    // Wave-7：治理策略保存即生效（enforcement 仍需用户显式开启）。
    pony_agent_core::agent::usage_governance::set_policy(saved.usage_governance_policy.clone());
    // B125：并发上限保存即生效（只影响后续 acquire，在途 turn 不被打断）。
    pony_agent_core::agent::subagent_runtime::set_subagent_max_concurrent(
        saved.subagent_max_concurrent,
    );
    Ok(saved)
}

#[tauri::command]
pub(crate) fn load_provider_registry() -> ProviderRegistryView {
    ProviderRegistryStore::new().load_view()
}

#[tauri::command]
pub(crate) fn save_provider_registry(
    registry: ProviderRegistryView,
) -> Result<ProviderRegistryView, String> {
    ProviderRegistryStore::new().save_view(registry)
}

#[tauri::command]
pub(crate) fn save_provider_registry_without_env_sync(
    registry: ProviderRegistryView,
) -> Result<ProviderRegistryView, String> {
    ProviderRegistryStore::new().save_view_without_env_sync(registry)
}

/// /models 目录拉取（design D3）：key 解析顺序为显式入参 → registry store 已存密钥
/// → 报错"缺少 API Key"；鉴权头尊重该协议 endpoint 的显式 auth_type（Auto 才按家族
/// 推导）；key 不入任何日志。
#[tauri::command]
pub(crate) fn fetch_provider_models(
    provider_id: Option<String>,
    protocol: agent::provider::ProviderProtocol,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    let explicit_key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let store = ProviderRegistryStore::new();
    let api_key = match explicit_key {
        Some(key) => key,
        None => store
            .provider_api_key(provider_id.as_deref())
            .ok_or_else(|| "缺少 API Key".to_string())?,
    };
    let auth_hint = store.provider_endpoint_auth(provider_id.as_deref(), &protocol);
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err("缺少 Base URL；请先在提供商高级设置中填写该协议的 Base URL。".to_string());
    }
    agent::provider::fetch_model_ids(&protocol, base_url, &auth_hint, &api_key)
}

#[tauri::command]
pub(crate) fn get_service_api_key(service: String) -> Result<String, String> {
    let store = pony_agent_core::agent::config::ProviderRegistryStore::new();
    // F4c：仅回传掩码，禁止明文出 IPC（保存新 key 仍走 set_service_api_key 单向写入）。
    Ok(store
        .get_service_api_key(&service)
        .map(|key| mask_service_api_key(&key))
        .unwrap_or_default())
}

#[tauri::command]
pub(crate) fn set_service_api_key(service: String, key: String) -> Result<(), String> {
    let store = pony_agent_core::agent::config::ProviderRegistryStore::new();
    store.set_service_api_key(&service, &key)
}
