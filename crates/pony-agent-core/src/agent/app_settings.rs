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

        // Wave-4.1 L3-B #3：文件含凭据（otelToken）时收紧为仅属主可读写 0600。
        // 语义：**best-effort** —— 设置权限失败只降级为 warn 日志，绝不让保存失败。
        // Windows：NTFS ACL 由用户目录继承策略决定，本函数不做额外收紧（见上契约说明）。
        if storage
            .otel_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .is_some()
        {
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
