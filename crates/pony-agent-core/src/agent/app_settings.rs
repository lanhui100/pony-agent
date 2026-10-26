use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceMode {
    Coding,
    Work,
}

impl Default for WorkspaceMode {
    fn default() -> Self {
        Self::Coding
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuleAction {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ToolRuleMatcher {
    Exact(String),
    Prefix(String),
    Regex(String),
}

impl ToolRuleMatcher {
    pub fn matches(&self, tool_name: &str) -> bool {
        match self {
            Self::Exact(target) => !target.trim().is_empty() && tool_name == target,
            Self::Prefix(prefix) => !prefix.trim().is_empty() && tool_name.starts_with(prefix),
            Self::Regex(pattern) => {
                if pattern.trim().is_empty() || pattern.len() > 256 {
                    return false;
                }
                regex::Regex::new(pattern).map(|r| r.is_match(tool_name)).unwrap_or(false)
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolRule {
    pub matcher: ToolRuleMatcher,
    pub action: RuleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub workspace_mode: WorkspaceMode,
    #[serde(default)]
    pub tool_rules: Vec<ToolRule>,
    /// 全局 Agent Trace 上报开关（落盘键 `agentTraceReporting`，缺省 `false`）。
    /// 契约：由设置页读写；`false` 时 core 必须零 span 分配、零网络出口。
    #[serde(default)]
    pub agent_trace_reporting: bool,
    /// OTLP 端点（落盘键 `otelEndpoint`）；`None`/空串 ⇒ 回落 env ⇒ 内置公网缺省。
    /// Wave-4 C1：来源优先级 `settings > env > 内置默认`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otel_endpoint: Option<String>,
    /// OTLP 鉴权 token（落盘键 `otelToken`）；只允许流入导出 header，
    /// **严禁**进入日志、面包屑、设置页回显或任何调试输出。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otel_token: Option<String>,
    /// `service.name`（落盘键 `otelServiceName`）；`None`/空串 ⇒ 回落 env ⇒ `pony-agent`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otel_service_name: Option<String>,
    /// 采样率（落盘键 `otelSamplingRatio`，0.0 ~ 1.0）；越界值在解析期拒绝采纳。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otel_sampling_ratio: Option<f64>,
    /// PonySentry ingest 基址（落盘键 `ponysentryIngestUrl`）；`None`/空串 ⇒ 回落 env
    /// `PONYSENTRY_INGEST_URL` ⇒ 内置缺省。优先级：`settings > env > 构建期 > 缺省`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ponysentry_ingest_url: Option<String>,
    /// PonySentry 客户端 token（落盘键 `ponysentryClientToken`）；只允许流入 ingest 请求头。
    /// **严禁**进入日志、面包屑、设置页回显或任何调试输出（与 `otelToken` 同一约定）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ponysentry_client_token: Option<String>,
    /// 会话成本预算（USD，落盘键 `costBudgetUsd`）。Wave-5 K3：仅用于**软告警**，
    /// `None` / `null` / `<= 0.0` 一律视为未设置预算；任何情况下不得拦截用户请求。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_budget_usd: Option<f64>,
    /// 用量治理策略（Wave-7，落盘键 `usageGovernancePolicy`；`None` ⇒ 默认策略 = 不启用拦截）。
    ///
    /// 契约 G2：`enforcement` 默认 `false`（纯软告警）；只有用户显式 opt-in 才阻断**新** turn。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_governance_policy: Option<crate::agent::usage_governance::UsagePolicy>,
}

impl std::fmt::Debug for AppSettings {
    /// Wave-4.1 L3-A #6：`otelToken` 一律打码（沿用 `OtelConfig` 手写 Debug 的既有约定），
    /// 避免 `{:?}` / `expect("{settings:?}")` / `unwrap_err()` 之类的 Debug 打印泄露凭据。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppSettings")
            .field("workspace_mode", &self.workspace_mode)
            .field("tool_rules", &self.tool_rules)
            .field("agent_trace_reporting", &self.agent_trace_reporting)
            .field("otel_endpoint", &self.otel_endpoint)
            .field(
                "otel_token",
                &self.otel_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("otel_service_name", &self.otel_service_name)
            .field("otel_sampling_ratio", &self.otel_sampling_ratio)
            .field("ponysentry_ingest_url", &self.ponysentry_ingest_url)
            .field(
                "ponysentry_client_token",
                &self.ponysentry_client_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("cost_budget_usd", &self.cost_budget_usd)
            .field("usage_governance_policy", &self.usage_governance_policy)
            .finish()
    }
}

impl AppSettings {
    /// 转换为 OTel 配置解析所需的中性 settings 快照（Wave-4 C1）。
    ///
    /// 语义：纯字段搬运，不读写全局状态、不发网络；`token` 仅作为数据透传，
    /// 调用方（`otel::resolve_config`）只把它组装成导出 header。
    pub fn otel_settings_snapshot(&self) -> crate::agent::otel::OtelSettingsSnapshot {
        crate::agent::otel::OtelSettingsSnapshot {
            endpoint: self.otel_endpoint.clone(),
            token: self.otel_token.clone(),
            service_name: self.otel_service_name.clone(),
            sampling_ratio: self.otel_sampling_ratio,
        }
    }

    /// 转换为 PonySentry 配置解析所需的中性 settings 快照。
    ///
    /// 语义：纯字段搬运（不读写全局状态、不发网络）。调用方（`ponysentry::init_from_settings`）
    /// 按 **settings > env > 构建期注入 > 缺省** 组装生效配置；token 仅作数据透传，
    /// 全链路不落日志/Debug（两侧类型的手写 `Debug` 均打码）。
    pub fn ponysentry_settings_snapshot(
        &self,
    ) -> crate::agent::ponysentry::PonySentrySettingsSnapshot {
        crate::agent::ponysentry::PonySentrySettingsSnapshot {
            ingest_url: self.ponysentry_ingest_url.clone(),
            client_token: self.ponysentry_client_token.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
struct AppSettingsStorage {
    #[serde(default)]
    workspace_mode: WorkspaceMode,
    #[serde(default)]
    tool_rules: Vec<ToolRule>,
    #[serde(default)]
    agent_trace_reporting: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    otel_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    otel_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    otel_service_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    otel_sampling_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ponysentry_ingest_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ponysentry_client_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cost_budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage_governance_policy: Option<crate::agent::usage_governance::UsagePolicy>,
}

impl From<AppSettingsStorage> for AppSettings {
    fn from(value: AppSettingsStorage) -> Self {
        Self {
            workspace_mode: value.workspace_mode,
            tool_rules: value.tool_rules,
            agent_trace_reporting: value.agent_trace_reporting,
            otel_endpoint: value.otel_endpoint,
            otel_token: value.otel_token,
            otel_service_name: value.otel_service_name,
            otel_sampling_ratio: value.otel_sampling_ratio,
            ponysentry_ingest_url: value.ponysentry_ingest_url,
            ponysentry_client_token: value.ponysentry_client_token,
            cost_budget_usd: value.cost_budget_usd,
            usage_governance_policy: value.usage_governance_policy,
        }
    }
}

impl From<AppSettings> for AppSettingsStorage {
    fn from(value: AppSettings) -> Self {
        Self {
            workspace_mode: value.workspace_mode,
            tool_rules: value.tool_rules,
            agent_trace_reporting: value.agent_trace_reporting,
            otel_endpoint: value.otel_endpoint,
            otel_token: value.otel_token,
            otel_service_name: value.otel_service_name,
            otel_sampling_ratio: value.otel_sampling_ratio,
            ponysentry_ingest_url: value.ponysentry_ingest_url,
            ponysentry_client_token: value.ponysentry_client_token,
            cost_budget_usd: value.cost_budget_usd,
            usage_governance_policy: value.usage_governance_policy,
        }
    }
}

pub struct AppSettingsStore {
    path: PathBuf,
}

impl AppSettingsStore {
    pub fn new() -> Self {
        let mut path = config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("pony-agent");
        path.push("settings.json");
        Self::with_path(path)
    }

    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load_view(&self) -> AppSettings {
        self.load_storage().into()
    }

    pub fn save_view(&self, view: AppSettings) -> Result<AppSettings, String> {
        let storage: AppSettingsStorage = view.into();
        self.write_storage(&storage)?;
        Ok(storage.into())
    }

    fn load_storage(&self) -> AppSettingsStorage {
        if let Ok(content) = fs::read_to_string(&self.path) {
            if let Ok(storage) = serde_json::from_str::<AppSettingsStorage>(&content) {
                return storage;
            }
        }

        // 文件缺失/损坏 ⇒ 全字段缺省（workspaceMode=coding、toolRules 空、上报关、otel 四项未配置）。
        AppSettingsStorage::default()
    }

    fn write_storage(&self, storage: &AppSettingsStorage) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create app settings directory failed: {}", error))?;
        }

        let json = serde_json::to_string_pretty(storage)
            .map_err(|error| format!("serialize app settings failed: {}", error))?;

        fs::write(&self.path, json)
            .map_err(|error| format!("write app settings failed: {}", error))?;

        // Wave-4.1 L3-B #3：文件含凭据时收紧为仅属主可读写 0600。
        // task-14：`ponysentryClientToken` 与 `otelToken` 同属凭据，必须一并触发收紧。
        // 语义：**best-effort** —— 设置权限失败只降级为 warn 日志，绝不让保存失败。
        // Windows：NTFS ACL 由用户目录继承策略决定，本函数不做额外收紧（见上契约说明）。
        let has_credential = [
            storage.otel_token.as_deref(),
            storage.ponysentry_client_token.as_deref(),
        ]
        .into_iter()
        .any(|token| {
            token
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .is_some()
        });
        if has_credential {
            restrict_to_owner(&self.path);
        }

        Ok(())
    }
}

/// 把文件权限收紧到仅属主可读写（unix 0600）；非 unix 或失败时静默降级。
fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
            // 降级：不阻断保存、不 panic（日志面在 otel 模块，此处仅静默或 stderr 之外不新增边界）。
            let _ = error;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn _settings_file_path(base_path: &Path) -> PathBuf {
    base_path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_settings_dir(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pony-settings-{tag}-{stamp}"));
        fs::create_dir_all(&dir).expect("mkdir temp settings dir");
        dir
    }

    /// L3-A #6：`AppSettings` 的 Debug 必须对 `otelToken` 打码。
    #[test]
    fn app_settings_debug_redacts_otel_token() {
        let settings = AppSettings {
            otel_token: Some("SUPER_SECRET_SETTINGS_TOKEN".to_string()),
            otel_endpoint: Some("https://otel.example".to_string()),
            ..Default::default()
        };
        let shown = format!("{settings:?}");
        assert!(
            !shown.contains("SUPER_SECRET_SETTINGS_TOKEN"),
            "L3-A#6：AppSettings Debug 泄漏 otelToken：{shown}"
        );
        assert!(shown.contains("[REDACTED]"), "必须显式标出脱敏位：{shown}");
    }

    /// C1：四个 otel 字段按 camelCase 落盘、可双向读回。
    #[test]
    fn otel_settings_fields_roundtrip_with_camel_case_keys() {
        let dir = temp_settings_dir("camel");
        let path = dir.join("settings.json");
        let store = AppSettingsStore::with_path(path.clone());

        let saved = store
            .save_view(AppSettings {
                otel_endpoint: Some("https://user-otel.example".to_string()),
                otel_token: Some("user-token".to_string()),
                otel_service_name: Some("custom-svc".to_string()),
                otel_sampling_ratio: Some(0.25),
                ..Default::default()
            })
            .expect("save_view 必须成功");
        assert_eq!(saved.otel_service_name.as_deref(), Some("custom-svc"));
        assert_eq!(saved.otel_sampling_ratio, Some(0.25));

        let raw = fs::read_to_string(&path).expect("settings.json readable");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        assert_eq!(
            json.get("otelEndpoint").and_then(|value| value.as_str()),
            Some("https://user-otel.example")
        );
        assert_eq!(
            json.get("otelSamplingRatio").and_then(|value| value.as_f64()),
            Some(0.25)
        );
        assert!(json.get("otel_endpoint").is_none(), "不得写出 snake_case 键");

        let loaded = store.load_view();
        assert_eq!(loaded.otel_token.as_deref(), Some("user-token"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// task-14：PonySentry 两个 settings 字段按 camelCase 落盘、可双向读回，且能搬运为快照。
    #[test]
    fn ponysentry_settings_fields_roundtrip_with_camel_case_keys() {
        let dir = temp_settings_dir("ponysentry-camel");
        let path = dir.join("settings.json");
        let store = AppSettingsStore::with_path(path.clone());

        let saved = store
            .save_view(AppSettings {
                ponysentry_ingest_url: Some("https://ingest.example".to_string()),
                ponysentry_client_token: Some("ps-settings-token".to_string()),
                ..Default::default()
            })
            .expect("save_view 必须成功");
        assert_eq!(
            saved.ponysentry_ingest_url.as_deref(),
            Some("https://ingest.example")
        );

        let raw = fs::read_to_string(&path).expect("settings.json readable");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        assert_eq!(
            json.get("ponysentryIngestUrl").and_then(|value| value.as_str()),
            Some("https://ingest.example"),
            "落盘键必须是 camelCase `ponysentryIngestUrl`"
        );
        assert_eq!(
            json.get("ponysentryClientToken").and_then(|value| value.as_str()),
            Some("ps-settings-token"),
            "落盘键必须是 camelCase `ponysentryClientToken`"
        );
        assert!(
            json.get("ponysentry_ingest_url").is_none()
                && json.get("ponysentry_client_token").is_none(),
            "不得写出 snake_case 键"
        );

        let loaded = store.load_view();
        assert_eq!(
            loaded.ponysentry_client_token.as_deref(),
            Some("ps-settings-token"),
            "From(AppSettingsStorage) 必须搬运 ponysentryClientToken"
        );

        // 快照搬运：字段一一对应（token 仅作数据透传）。
        let snapshot = loaded.ponysentry_settings_snapshot();
        assert_eq!(
            snapshot.ingest_url.as_deref(),
            Some("https://ingest.example")
        );
        assert_eq!(snapshot.client_token.as_deref(), Some("ps-settings-token"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// task-14：`AppSettings` 的 Debug 必须对 `ponysentryClientToken` 打码。
    #[test]
    fn app_settings_debug_redacts_ponysentry_token() {
        let settings = AppSettings {
            ponysentry_client_token: Some("PONYSENTRY_SECRET_SETTINGS".to_string()),
            ponysentry_ingest_url: Some("https://ingest.example".to_string()),
            ..Default::default()
        };
        let shown = format!("{settings:?}");
        assert!(
            !shown.contains("PONYSENTRY_SECRET_SETTINGS"),
            "task-14：AppSettings Debug 泄漏 ponysentryClientToken：{shown}"
        );
        assert!(shown.contains("[REDACTED]"), "必须显式标出脱敏位：{shown}");
    }

    /// task-14：`ponysentryClientToken` 与 `otelToken` 同属凭据 —— 落盘必须同样收紧为 0600。
    #[cfg(unix)]
    #[test]
    fn settings_file_with_ponysentry_token_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_settings_dir("ponysentry-perm");
        let path = dir.join("settings.json");
        let store = AppSettingsStore::with_path(path.clone());

        store
            .save_view(AppSettings {
                ponysentry_client_token: Some("owner-only-ps-token".to_string()),
                ..Default::default()
            })
            .expect("save_view 必须成功");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "含 ponysentryClientToken 的 settings.json 必须是 0600，实际 {mode:o}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// L3-B #3：含 token 时 settings.json 权限收紧为 0600（best-effort，unix 断言）。
    #[cfg(unix)]
    #[test]
    fn settings_file_with_token_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_settings_dir("perm");
        let path = dir.join("settings.json");
        let store = AppSettingsStore::with_path(path.clone());

        store
            .save_view(AppSettings {
                otel_token: Some("owner-only-token".to_string()),
                ..Default::default()
            })
            .expect("save_view 必须成功");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "含 token 的 settings.json 必须是 0600，实际 {mode:o}");

        // 无 token 时不做额外收紧（沿用系统默认，不阻断保存）。
        store
            .save_view(AppSettings::default())
            .expect("无 token 保存必须成功");

        let _ = fs::remove_dir_all(&dir);
    }
}
