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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub workspace_mode: WorkspaceMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsStorage {
    #[serde(default)]
    workspace_mode: WorkspaceMode,
}

impl From<AppSettingsStorage> for AppSettings {
    fn from(value: AppSettingsStorage) -> Self {
        Self {
            workspace_mode: value.workspace_mode,
        }
    }
}

impl From<AppSettings> for AppSettingsStorage {
    fn from(value: AppSettings) -> Self {
        Self {
            workspace_mode: value.workspace_mode,
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
