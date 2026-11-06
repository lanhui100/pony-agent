//! 模型偏好持久化：默认模型（Ctrl+D）与优选列表（Ctrl+F）。
//!
//! 存储与 `providers.json` 同目录（`<config_dir>/pony-agent/model-preferences.json`），
//! 但与注册表**解耦**：注册表结构被前端 TypeScript 契约锁定，偏好属于纯 UI 侧派生状态，
//! 因此单独成文件，避免污染 provider 契约与迁移链。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 模型偏好快照。字段缺失一律按空处理（向前兼容旧文件）。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreferences {
    /// 默认模型 id（Ctrl+D 设定；`None` = 未设过）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// 优选模型 id 列表（Ctrl+F 切换；保持插入顺序，用于「优选」栏排序）。
    #[serde(default)]
    pub preferred: Vec<String>,
}

impl ModelPreferences {
    pub fn is_preferred(&self, model_id: &str) -> bool {
        self.preferred.iter().any(|id| id == model_id)
    }

    /// 切换优选状态，返回切换后是否处于优选。
    pub fn toggle_preferred(&mut self, model_id: &str) -> bool {
        if let Some(pos) = self.preferred.iter().position(|id| id == model_id) {
            self.preferred.remove(pos);
            false
        } else {
            self.preferred.push(model_id.to_string());
            true
        }
    }

    /// 按优选顺序排序给定的模型 id 序列（未优选者保持原顺序排在后面）。
    pub fn order_preferred<'a, I>(&self, ids: I) -> Vec<String>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let ids: Vec<&str> = ids.into_iter().collect();
        let mut out: Vec<String> = Vec::new();
        for pref in &self.preferred {
            if ids.iter().any(|id| *id == pref.as_str()) && !out.iter().any(|id| id == pref) {
                out.push(pref.clone());
            }
        }
        for id in ids {
            if !out.iter().any(|existing| existing == id) {
                out.push(id.to_string());
            }
        }
        out
    }
}

/// JSON 文件存储（原子写：先写临时文件再 rename，避免半截文件）。
pub struct ModelPreferencesStore {
    path: PathBuf,
}

impl Default for ModelPreferencesStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelPreferencesStore {
    pub fn new() -> Self {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("pony-agent");
        path.push("model-preferences.json");
        Self { path }
    }

    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// 读取偏好；文件缺失/损坏一律退化为空偏好（绝不因为偏好文件坏掉而阻塞启动）。
    pub fn load(&self) -> ModelPreferences {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return ModelPreferences::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self, prefs: &ModelPreferences) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建偏好目录失败: {error}"))?;
        }
        let payload = serde_json::to_string_pretty(prefs)
            .map_err(|error| format!("序列化模型偏好失败: {error}"))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, payload).map_err(|error| format!("写入模型偏好失败: {error}"))?;
        fs::rename(&tmp, &self.path).map_err(|error| format!("落盘模型偏好失败: {error}"))?;
        Ok(())
    }

    /// Ctrl+D：设为默认模型并立即持久化。
    pub fn set_default_model(&self, model_id: &str) -> Result<ModelPreferences, String> {
        let mut prefs = self.load();
        prefs.default_model = Some(model_id.to_string());
        self.save(&prefs)?;
        Ok(prefs)
    }

    /// Ctrl+F：切换优选并立即持久化，返回切换后的偏好快照与「现在是否优选」。
    pub fn toggle_preferred(&self, model_id: &str) -> Result<(ModelPreferences, bool), String> {
        let mut prefs = self.load();
        let now_preferred = prefs.toggle_preferred(model_id);
        self.save(&prefs)?;
        Ok((prefs, now_preferred))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("pony-model-prefs-{}-{}", std::process::id(), name))
            .join("model-preferences.json")
    }

    #[test]
    fn missing_file_loads_empty_preferences() {
        let store = ModelPreferencesStore::with_path(temp_path("missing"));
        assert_eq!(store.load(), ModelPreferences::default());
    }

    #[test]
    fn set_default_model_round_trips() {
        let path = temp_path("default");
        let store = ModelPreferencesStore::with_path(&path);
        store.set_default_model("gpt-5.4").unwrap();
        let loaded = ModelPreferencesStore::with_path(&path).load();
        assert_eq!(loaded.default_model.as_deref(), Some("gpt-5.4"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn toggle_preferred_is_idempotent_pair() {
        let path = temp_path("toggle");
        let store = ModelPreferencesStore::with_path(&path);
        let (prefs, on) = store.toggle_preferred("deepseek-v4-pro").unwrap();
        assert!(on);
        assert!(prefs.is_preferred("deepseek-v4-pro"));
        let (prefs, on) = store.toggle_preferred("deepseek-v4-pro").unwrap();
        assert!(!on);
        assert!(!prefs.is_preferred("deepseek-v4-pro"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupted_file_degrades_to_default() {
        let path = temp_path("corrupted");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let store = ModelPreferencesStore::with_path(&path);
        assert_eq!(store.load(), ModelPreferences::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn order_preferred_puts_marked_models_first() {
        let mut prefs = ModelPreferences::default();
        prefs.toggle_preferred("b");
        prefs.toggle_preferred("c");
        let ordered = prefs.order_preferred(["a", "b", "c"].into_iter());
        assert_eq!(ordered, vec!["b", "c", "a"]);
    }
}
