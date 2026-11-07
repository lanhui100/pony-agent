//! TUI 显示偏好持久化：过程隐藏（hide_process）与思考折叠（thinking_collapsed）。
//!
//! 存储与 `model-preferences.json` 同目录（`<config_dir>/pony-agent/tui-preferences.json`），
//! 但**独立成文件**：这些是纯终端渲染侧开关，与 `providers.json`（provider 注册表契约）
//! 和桌面端 `settings.json`（前端契约）互不相干，混写会污染既有迁移链与前端类型定义，
//! 因此单独成文件、独立演进；文件里的字段缺失一律按产品默认（隐藏 + 折叠）处理，
//! 保证旧文件向前兼容。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn default_true() -> bool {
    true
}

/// TUI 显示偏好快照。缺失字段一律取产品默认值（`true`）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TuiPreferences {
    /// 隐藏 Agent 过程（思考 / 工具调用 / 工具结果块不渲染），默认隐藏。
    #[serde(default = "default_true")]
    pub hide_process: bool,
    /// 思考块折叠为单行摘要，默认折叠。
    #[serde(default = "default_true")]
    pub thinking_collapsed: bool,
}

impl Default for TuiPreferences {
    fn default() -> Self {
        Self {
            hide_process: true,
            thinking_collapsed: true,
        }
    }
}

/// JSON 文件存储（原子写：先写临时文件再 rename，避免半截文件）。
pub struct TuiPreferencesStore {
    path: PathBuf,
}

impl Default for TuiPreferencesStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiPreferencesStore {
    pub fn new() -> Self {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("pony-agent");
        path.push("tui-preferences.json");
        Self { path }
    }

    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 读取偏好；文件缺失/损坏/不可读一律退化为默认偏好（绝不因为偏好文件坏掉而阻塞启动）。
    pub fn load(&self) -> TuiPreferences {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return TuiPreferences::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self, prefs: &TuiPreferences) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建偏好目录失败: {error}"))?;
        }
        let payload = serde_json::to_string_pretty(prefs)
            .map_err(|error| format!("序列化 TUI 偏好失败: {error}"))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, payload).map_err(|error| format!("写入 TUI 偏好失败: {error}"))?;
        fs::rename(&tmp, &self.path).map_err(|error| format!("落盘 TUI 偏好失败: {error}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("pony-tui-prefs-{}-{}", std::process::id(), name))
            .join("tui-preferences.json")
    }

    fn cleanup(path: &PathBuf) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn default_hides_process_and_collapses_thinking() {
        let prefs = TuiPreferences::default();
        assert!(prefs.hide_process);
        assert!(prefs.thinking_collapsed);
    }

    #[test]
    fn missing_file_loads_defaults() {
        let path = temp_path("missing");
        let store = TuiPreferencesStore::with_path(&path);
        assert_eq!(store.load(), TuiPreferences::default());
        cleanup(&path);
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_path("roundtrip");
        let store = TuiPreferencesStore::with_path(&path);
        let prefs = TuiPreferences {
            hide_process: false,
            thinking_collapsed: false,
        };
        store.save(&prefs).unwrap();
        let loaded = TuiPreferencesStore::with_path(&path).load();
        assert_eq!(loaded, prefs);
        cleanup(&path);
    }

    #[test]
    fn corrupted_file_degrades_to_default() {
        let path = temp_path("corrupted");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let store = TuiPreferencesStore::with_path(&path);
        assert_eq!(store.load(), TuiPreferences::default());
        cleanup(&path);
    }

    #[test]
    fn partial_file_falls_back_per_field() {
        let path = temp_path("partial");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"hideProcess": false}"#).unwrap();
        let store = TuiPreferencesStore::with_path(&path);
        let prefs = store.load();
        assert!(!prefs.hide_process);
        assert!(prefs.thinking_collapsed);
        cleanup(&path);
    }
}
