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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsStorage {
    #[serde(default)]
    workspace_mode: WorkspaceMode,
    #[serde(default)]
    tool_rules: Vec<ToolRule>,
    #[serde(default)]
    agent_trace_reporting: bool,
}

impl From<AppSettingsStorage> for AppSettings {
    fn from(value: AppSettingsStorage) -> Self {
        Self {
            workspace_mode: value.workspace_mode,
            tool_rules: value.tool_rules,
            agent_trace_reporting: value.agent_trace_reporting,
        }
    }
}

impl From<AppSettings> for AppSettingsStorage {
    fn from(value: AppSettings) -> Self {
        Self {
            workspace_mode: value.workspace_mode,
            tool_rules: value.tool_rules,
            agent_trace_reporting: value.agent_trace_reporting,
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

        AppSettingsStorage {
            workspace_mode: WorkspaceMode::default(),
            tool_rules: Vec::new(),
            agent_trace_reporting: false,
        }
    }

    fn write_storage(&self, storage: &AppSettingsStorage) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create app settings directory failed: {}", error))?;
        }

        let json = serde_json::to_string_pretty(storage)
            .map_err(|error| format!("serialize app settings failed: {}", error))?;

        fs::write(&self.path, json).map_err(|error| format!("write app settings failed: {}", error))
    }
}

fn _settings_file_path(base_path: &Path) -> PathBuf {
    base_path.to_path_buf()
}
