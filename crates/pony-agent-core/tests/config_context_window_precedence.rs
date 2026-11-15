//! 回归：`providers.json` 里手填的 `capabilities.contextWindowTokens` 必须活过真实注册表的
//! 加载 / 保存往返。
//!
//! 缺陷现场（ADR 2026-11-14-model-declared-context-window-precedence）：非 `custom` 预设下
//! `normalize_model_capabilities` 一律返回预设默认值 ⇒ 用户手填的 200K / 1M 被静默改写成
//! 256K，TUI / 桌面端的上下文占用百分比因此按错误分母渲染。
//!
//! 本用例故意**不测私有归一化函数**，而是走 `ProviderRegistryStore::with_path` 的真实读写链路
//! （= 用户手改 `~/.config/pony-agent/providers.json` 后的实际路径），并用临时目录隔离，
//! 不触达真实用户注册表。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::config::ProviderRegistryStore;

/// 进程内唯一临时目录守卫：用例结束即递归删除。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pony-agent-it-context-window-{label}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create isolated temp dir");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

const REGISTRY_FIXTURE: &str = r#"{
  "providers": [
    {
      "id": "fakep",
      "name": "FakeProvider",
      "protocol": "openai-completions",
      "base_url": "https://example.invalid/v1",
      "api_key_env_var": "MOCK_API_KEY",
      "api_key_value": "mock-key",
      "supported_protocols": ["openai-completions"],
      "endpoints": [],
      "models": [
        {
          "id": "m-200k",
          "name": "200K chat",
          "model": "fake-chat-200k",
          "temperature": 0.2,
          "maxOutputTokens": 64000,
          "capabilityPreset": "open-ai-chat",
          "capabilities": { "contextWindowTokens": 200000 }
        },
        {
          "id": "m-1m",
          "name": "1M chat",
          "model": "fake-chat-1m",
          "temperature": 0.2,
          "maxOutputTokens": 64000,
          "capabilityPreset": "open-ai-chat",
          "capabilities": { "contextWindowTokens": 1000000 }
        }
      ],
      "selected_model_id": "m-200k"
    }
  ],
  "selected_provider_id": "fakep"
}"#;

fn window_of(view: &pony_agent_core::agent::config::ProviderRegistryView, model_id: &str) -> Option<u32> {
    view.providers
        .iter()
        .flat_map(|provider| provider.models.iter())
        .find(|model| model.id == model_id)
        .and_then(|model| model.capabilities.context_window_tokens)
}

#[test]
fn declared_context_window_survives_real_registry_load_and_save() {
    let dir = TempDir::new("roundtrip");
    let registry_path = dir.join("providers.json");
    std::fs::write(&registry_path, REGISTRY_FIXTURE).expect("seed providers.json");

    // 1) 加载：手填窗口原样读出（修复前这里会变成 262144）。
    let store = ProviderRegistryStore::with_path(&registry_path);
    let view = store.load_view();
    assert_eq!(
        window_of(&view, "m-200k"),
        Some(200_000),
        "加载必须保留显式声明的 200K 窗口"
    );
    assert_eq!(
        window_of(&view, "m-1m"),
        Some(1_000_000),
        "加载必须保留显式声明的 1M 窗口"
    );

    // 2) 保存：不得把声明值改写成预设默认值。
    store.save_view(view).expect("save_view should succeed");

    // 3) 落盘内容与再次加载一致（防止「内存对、磁盘错」的假通过）。
    let raw: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&registry_path).expect("read persisted registry"),
    )
    .expect("persisted registry should be valid JSON");
    let persisted = raw["providers"][0]["models"]
        .as_array()
        .expect("models array")
        .iter()
        .map(|model| {
            (
                model["id"].as_str().unwrap_or_default().to_string(),
                model["capabilities"]["contextWindowTokens"].as_u64(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(persisted.get("m-200k"), Some(&Some(200_000)));
    assert_eq!(persisted.get("m-1m"), Some(&Some(1_000_000)));

    let reloaded = ProviderRegistryStore::with_path(&registry_path).load_view();
    assert_eq!(window_of(&reloaded, "m-200k"), Some(200_000));
    assert_eq!(window_of(&reloaded, "m-1m"), Some(1_000_000));
}
