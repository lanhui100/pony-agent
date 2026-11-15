//! 模型上下文窗口（`providers[].models[].capabilities.contextWindowTokens`）的盘点与定点编辑。
//!
//! ## 为什么不用「`load_view` → 改字段 → `save_view`」
//!
//! `ProviderRegistryStore::load_storage` 归一化时会把**缺失**的窗口按能力预设补成显式值
//! （见 ADR 2026-11-14-model-declared-context-window-precedence）。于是 view↔storage 往返会
//! 把「未声明」固化成「声明了 262144」：一次 TUI/CLI 保存之后，预设再也补不了缺，
//! 用户也分不清「我填过」与「系统补的」。
//!
//! 窗口是**逐模型的个体事实**（同一预设可挂 128K/200K/1M 的不同部署），「未声明」本身
//! 就是有效语义（跟随预设）。因此本模块直接读写 `providers.json` 的**单个键**：
//!
//! - 只增删 `capabilities.contextWindowTokens`，其余键值原样保留（不做归一化、不落默认值）；
//! - 清除窗口 = **删掉该键**，而不是写 0 或写预设默认值；
//! - 写入走同目录临时文件 + `rename`，避免半截 JSON 把用户配置改坏。
//!
//! ## 谁在用
//!
//! - TUI `/models` 面板的 `w`（`bin/tui/model_window_editor.rs`）；
//! - CLI `ponycli models list` / `ponycli models set-window`（`bin/cli.rs`）。
//!
//! 两个入口共用 [`parse_context_window_input`] / [`format_context_window`]，口径与桌面端
//! `ProviderConfigPage.vue` 的 `formatTokenUnit` 一致（十进制 1000 / 1_000_000）。

use crate::agent::config::{ProviderRegistryStore, ProviderModelConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// 可接受的最大上下文窗口（token）。上限只防手滑（多打一位数变成 2000 万），
/// 不参与任何业务判断：真值始终是上游模型自己的容量。
pub const MAX_CONTEXT_WINDOW_TOKENS: u32 = 10_000_000;

/// 一个模型的窗口视图：声明值（磁盘真值）与生效值（声明 > 预设补缺）并列。
///
/// 两者必须同时暴露：用户需要看到「这个数是我填的，还是预设补的」，
/// 否则「为什么 200K 模型显示 62%」这类问题无从诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelContextWindow {
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    pub model_name: String,
    /// 线上模型名（请求里真正发出去的那个），可作为 `model_id` 之外的别名寻址。
    pub model: String,
    /// `capabilityPreset` 的线上标签（serde 线格式，不另造第二套名字）。
    pub preset: String,
    /// `providers.json` 里显式声明的窗口；`None` = 未声明（跟随预设）。
    pub declared: Option<u32>,
    /// 生效窗口（`declared` 优先，否则预设补缺）。
    pub effective: Option<u32>,
}

impl ModelContextWindow {
    /// 生效窗口是否来自用户声明（`false` = 预设补缺）。
    pub fn is_declared(&self) -> bool {
        self.declared.is_some()
    }
}

/// 读取某个模型当前的窗口声明与生效值。找不到返回 `None`（README 级别的「未命中」不报错）。
pub fn find_context_window(
    store: &ProviderRegistryStore,
    provider_id: &str,
    model_id: &str,
) -> Option<ModelContextWindow> {
    list_context_windows(store)
        .into_iter()
        .find(|entry| entry.provider_id == provider_id && matches_model(&entry, model_id))
}

/// 盘点全部模型的窗口（顺序 = 注册表顺序，provider 主序、model 次序）。
pub fn list_context_windows(store: &ProviderRegistryStore) -> Vec<ModelContextWindow> {
    // 声明值只能从**原始 JSON**读：`load_view()` 的窗口已被预设补齐，无法区分「用户填的
    // 262144」与「预设补的 262144」。
    let declared = read_registry_value(store)
        .map(|raw| declared_windows(&raw))
        .unwrap_or_default();

    let view = store.load_view();
    let mut out = Vec::new();
    for provider in &view.providers {
        for model in &provider.models {
            let declared_window = lookup_declared(&declared, &provider.id, model);
            out.push(ModelContextWindow {
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                model_id: model.id.clone(),
                model_name: model.name.clone(),
                model: model.model.clone(),
                preset: preset_label(&model.capability_preset),
                declared: declared_window,
                effective: model
                    .capabilities
                    .context_window_tokens
                    .filter(|tokens| *tokens > 0),
            });
        }
    }
    out
}

/// 定点写入某个模型的窗口声明。
///
/// - `Some(tokens)`：写 `capabilities.contextWindowTokens = tokens`（`capabilities` 缺席/非对象时新建）；
/// - `None`：**删掉**该键 ⇒ 恢复「跟随预设」，不落任何默认值；
/// - 只有目标模型的这一个键被改动，其余键值（含其他模型的未声明状态）原样保留。
///
/// 返回写入后的窗口视图（直接从磁盘重读，避免调用方拿到内存里的过期值）。
pub fn set_context_window(
    store: &ProviderRegistryStore,
    provider_id: &str,
    model_id: &str,
    window: Option<u32>,
) -> Result<ModelContextWindow, String> {
    if let Some(tokens) = window {
        validate_window(tokens)?;
    }

    let path = store.path();
    let mut raw = read_registry_value(store)?;
    let provider = raw
        .get_mut("providers")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("{} 缺少 providers 数组", path.display()))?
        .iter_mut()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(provider_id))
        .ok_or_else(|| format!("未找到提供商 `{provider_id}`"))?;

    let model = provider
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("提供商 `{provider_id}` 缺少 models 数组"))?
        .iter_mut()
        .find(|entry| model_matches_value(entry, model_id))
        .ok_or_else(|| format!("提供商 `{provider_id}` 下未找到模型 `{model_id}`"))?;

    match window {
        Some(tokens) => {
            let object = model
                .as_object_mut()
                .ok_or_else(|| "模型条目不是 JSON 对象".to_string())?;
            let capabilities = object
                .entry("capabilities".to_string())
                .or_insert_with(|| Value::Object(Default::default()));
            if !capabilities.is_object() {
                *capabilities = Value::Object(Default::default());
            }
            capabilities
                .as_object_mut()
                .expect("capabilities 已保证是对象")
                .insert("contextWindowTokens".to_string(), json!(tokens));
        }
        None => {
            if let Some(capabilities) = model
                .get_mut("capabilities")
                .and_then(Value::as_object_mut)
            {
                capabilities.remove("contextWindowTokens");
            }
        }
    }

    write_registry_value(path, &raw)?;

    find_context_window(store, provider_id, model_id).ok_or_else(|| {
        // 写入后读不回来 = 归一化把这次写入吃掉了（例如模型 id 在归一化时被重写）。
        // 这种情况必须显式报错，绝不假装成功。
        format!("写入后未能读回 `{provider_id}` / `{model_id}` 的窗口")
    })
}

/// 解析用户输入的窗口：`200000` / `200_000` / `200K` / `1M` / `1.5M`。
///
/// 空串、`-`、`0`、`clear`、`default`、`预设` 一律表示**清除声明**（回到跟随预设）。
/// 十进制口径与桌面端一致：`K` = 1000，`M` = 1_000_000。
pub fn parse_context_window_input(raw: &str) -> Result<Option<u32>, String> {
    let text = raw.trim();
    if text.is_empty()
        || matches!(
            text.to_ascii_lowercase().as_str(),
            "-" | "0" | "clear" | "default" | "none" | "auto"
        )
        || text == "预设"
    {
        return Ok(None);
    }

    let cleaned: String = text
        .chars()
        .filter(|ch| !matches!(ch, '_' | ' ' | '\t' | ','))
        .collect();
    let (number, multiplier) = match cleaned.chars().last() {
        Some('k') | Some('K') => (&cleaned[..cleaned.len() - 1], 1_000f64),
        Some('m') | Some('M') => (&cleaned[..cleaned.len() - 1], 1_000_000f64),
        _ => (cleaned.as_str(), 1f64),
    };
    let value: f64 = number
        .parse()
        .map_err(|_| "请输入整数 token 数（支持 200000 / 200K / 1M）".to_string())?;
    if !value.is_finite() || value < 0.0 {
        return Err("窗口必须是非负数".to_string());
    }
    let tokens = value * multiplier;
    let tokens = tokens.round();
    if tokens <= 0.0 {
        return Ok(None);
    }
    if tokens > f64::from(MAX_CONTEXT_WINDOW_TOKENS) {
        return Err(format!(
            "窗口过大：{}（上限 {} token）",
            format_context_window(MAX_CONTEXT_WINDOW_TOKENS),
            MAX_CONTEXT_WINDOW_TOKENS
        ));
    }
    Ok(Some(tokens as u32))
}

/// 窗口的人类可读写法（十进制 1000 / 1_000_000，与桌面端 `formatTokenUnit` 同口径）。
pub fn format_context_window(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        let millions = f64::from(tokens) / 1_000_000.0;
        if (millions - millions.trunc()).abs() < f64::EPSILON {
            format!("{}M", millions as u64)
        } else {
            format!("{millions:.1}M")
        }
    } else if tokens >= 1_000 {
        let thousands = f64::from(tokens) / 1_000.0;
        if (thousands - thousands.trunc()).abs() < f64::EPSILON {
            format!("{}K", thousands as u64)
        } else {
            format!("{thousands:.1}K")
        }
    } else {
        tokens.to_string()
    }
}

fn validate_window(tokens: u32) -> Result<(), String> {
    if tokens == 0 {
        return Err("窗口必须大于 0（要去掉声明请用空值 / `-`）".to_string());
    }
    if tokens > MAX_CONTEXT_WINDOW_TOKENS {
        return Err(format!(
            "窗口过大：{}（上限 {} token）",
            format_context_window(MAX_CONTEXT_WINDOW_TOKENS),
            MAX_CONTEXT_WINDOW_TOKENS
        ));
    }
    Ok(())
}

/// 模型寻址：先按条目 id（稳定主键），再按线上模型名（用户更可能记得的那个）。
fn matches_model(entry: &ModelContextWindow, model_id: &str) -> bool {
    entry.model_id == model_id || entry.model == model_id
}

fn model_matches_value(entry: &Value, model_id: &str) -> bool {
    entry.get("id").and_then(Value::as_str) == Some(model_id)
        || entry.get("model").and_then(Value::as_str) == Some(model_id)
}

fn preset_label(preset: &crate::agent::config::ProviderCapabilityPreset) -> String {
    // 直接取 serde 线格式（camelCase），不另造一张名字表：预设增删时不会留下过期标签。
    serde_json::to_value(preset)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "auto".to_string())
}

/// 原始 JSON 里的声明映射：`(provider_id, 模型 id)` 与 `(provider_id, 线上模型名)` 双键，
/// 两种寻址都能命中（注册表里 id 可能被归一化重写过，name/model 仍是用户输入的原样）。
fn declared_windows(raw: &Value) -> HashMap<(String, String), Option<u32>> {
    let mut map: HashMap<(String, String), Option<u32>> = HashMap::new();
    let Some(providers) = raw.get("providers").and_then(Value::as_array) else {
        return map;
    };
    for provider in providers {
        let Some(provider_id) = provider.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(models) = provider.get("models").and_then(Value::as_array) else {
            continue;
        };
        for model in models {
            let declared = model
                .get("capabilities")
                .and_then(|capabilities| capabilities.get("contextWindowTokens"))
                .and_then(Value::as_u64)
                .and_then(|tokens| u32::try_from(tokens).ok())
                .filter(|tokens| *tokens > 0);
            if let Some(id) = model.get("id").and_then(Value::as_str) {
                map.insert((provider_id.to_string(), id.to_string()), declared);
            }
            if let Some(name) = model.get("model").and_then(Value::as_str) {
                map.entry((provider_id.to_string(), name.to_string()))
                    .or_insert(declared);
            }
        }
    }
    map
}

fn lookup_declared(
    declared: &HashMap<(String, String), Option<u32>>,
    provider_id: &str,
    model: &ProviderModelConfig,
) -> Option<u32> {
    declared
        .get(&(provider_id.to_string(), model.id.clone()))
        .copied()
        .flatten()
        .or_else(|| {
            declared
                .get(&(provider_id.to_string(), model.model.clone()))
                .copied()
                .flatten()
        })
}

fn read_registry_value(store: &ProviderRegistryStore) -> Result<Value, String> {
    let path = store.path();
    let content = fs::read_to_string(path)
        .map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    serde_json::from_str::<Value>(&content)
        .map_err(|error| format!("解析 {} 失败：{error}", path.display()))
}

/// 同目录临时文件 + `rename`：中途失败不会留下半截 JSON（配置损坏比写失败更难恢复）。
/// 末尾不追加换行，与 `ProviderRegistryStore::write_storage` 的落盘形态保持一致。
fn write_registry_value(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败：{error}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("序列化模型窗口失败：{error}"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("providers.json");
    let temp = path.with_file_name(format!("{file_name}.tmp-{}", std::process::id()));
    fs::write(&temp, json)
        .map_err(|error| format!("写入 {} 失败：{error}", temp.display()))?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("替换 {} 失败：{error}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "pony-model-window-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    /// 两个模型：一个显式声明 200000，一个**完全没有** capabilities 段（未声明）。
    fn write_fixture(root: &Path) -> PathBuf {
        let path = root.join("providers.json");
        let raw = json!({
            "providers": [{
                "id": "provider-demo",
                "name": "Demo",
                "protocol": "openai-completions",
                "base_url": "https://example.test/v1",
                "api_key_env_var": "DEMO_API_KEY",
                "models": [
                    {
                        "id": "model-declared",
                        "name": "Declared",
                        "model": "declared-200k",
                        "temperature": 0.2,
                        "maxOutputTokens": 64000,
                        "capabilityPreset": "open-ai-chat",
                        "capabilities": { "contextWindowTokens": 200000 }
                    },
                    {
                        "id": "model-undeclared",
                        "name": "Undeclared",
                        "model": "undeclared-1m",
                        "temperature": 0.2,
                        "maxOutputTokens": 64000,
                        "capabilityPreset": "open-ai-chat"
                    }
                ],
                "selectedModelId": "model-declared"
            }],
            "selectedProviderId": "provider-demo"
        });
        fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).expect("write fixture");
        path
    }

    #[test]
    fn list_distinguishes_declared_from_preset_filled_window() {
        let root = temp_root("list");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);

        let entries = list_context_windows(&store);
        let declared = entries
            .iter()
            .find(|entry| entry.model_id == "model-declared")
            .expect("declared model");
        let undeclared = entries
            .iter()
            .find(|entry| entry.model_id == "model-undeclared")
            .expect("undeclared model");

        assert_eq!(declared.declared, Some(200_000));
        assert_eq!(declared.effective, Some(200_000));
        assert!(declared.is_declared());
        // 未声明：磁盘上没有该键，生效值来自预设补缺（默认 256K）。
        assert_eq!(undeclared.declared, None);
        assert_eq!(
            undeclared.effective,
            Some(crate::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS)
        );
        assert!(!undeclared.is_declared());
    }

    #[test]
    fn setting_one_model_window_leaves_other_models_untouched() {
        let root = temp_root("set");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);
        let before = read_registry_value(&store).expect("read before");

        let updated = set_context_window(&store, "provider-demo", "model-undeclared", Some(1_000_000))
            .expect("set window");
        assert_eq!(updated.declared, Some(1_000_000));
        assert_eq!(updated.effective, Some(1_000_000));

        let after = read_registry_value(&store).expect("read after");
        // 除目标键之外，整个注册表必须逐字等价（含另一个模型的声明值、endpoints 等）。
        let mut expected = before.clone();
        expected["providers"][0]["models"][1]["capabilities"] =
            json!({ "contextWindowTokens": 1_000_000 });
        assert_eq!(after, expected);
        assert_eq!(
            after["providers"][0]["models"][0]["capabilities"]["contextWindowTokens"],
            json!(200_000)
        );
    }

    #[test]
    fn clearing_window_removes_the_key_instead_of_writing_a_default() {
        let root = temp_root("clear");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);

        let cleared = set_context_window(&store, "provider-demo", "model-declared", None)
            .expect("clear window");
        assert_eq!(cleared.declared, None);
        assert_eq!(
            cleared.effective,
            Some(crate::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS)
        );

        let raw = read_registry_value(&store).expect("read cleared");
        assert!(raw["providers"][0]["models"][0]["capabilities"]
            .get("contextWindowTokens")
            .is_none());
        // capabilities 段本身仍在（不清空整段）。
        assert!(raw["providers"][0]["models"][0]
            .get("capabilities")
            .is_some());
    }

    #[test]
    fn setting_window_on_model_without_capabilities_creates_only_that_key() {
        let root = temp_root("create");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);

        set_context_window(&store, "provider-demo", "model-undeclared", Some(128_000))
            .expect("create capabilities");

        let raw = read_registry_value(&store).expect("read created");
        assert_eq!(
            raw["providers"][0]["models"][1]["capabilities"],
            json!({ "contextWindowTokens": 128_000 })
        );
        // 另一个模型仍是原样 200000（未被"统一"成同一个值）。
        assert_eq!(
            raw["providers"][0]["models"][0]["capabilities"]["contextWindowTokens"],
            json!(200_000)
        );
    }

    #[test]
    fn unknown_provider_or_model_fails_closed_without_writing() {
        let root = temp_root("unknown");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);
        let before = fs::read_to_string(&path).expect("read file");

        let provider_error =
            set_context_window(&store, "nope", "model-declared", Some(200_000)).unwrap_err();
        assert!(provider_error.contains("未找到提供商"), "{provider_error}");
        let model_error =
            set_context_window(&store, "provider-demo", "nope", Some(200_000)).unwrap_err();
        assert!(model_error.contains("未找到模型"), "{model_error}");

        // 失败路径绝不落盘。
        assert_eq!(fs::read_to_string(&path).expect("read file"), before);
    }

    #[test]
    fn model_can_be_addressed_by_wire_model_name() {
        let root = temp_root("alias");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);

        let updated = set_context_window(&store, "provider-demo", "declared-200k", Some(1_000_000))
            .expect("set by wire name");
        assert_eq!(updated.model_id, "model-declared");
        assert_eq!(updated.declared, Some(1_000_000));
    }

    #[test]
    fn parse_accepts_plain_k_and_m_notation() {
        assert_eq!(parse_context_window_input("200000").unwrap(), Some(200_000));
        assert_eq!(parse_context_window_input(" 200_000 ").unwrap(), Some(200_000));
        assert_eq!(parse_context_window_input("200K").unwrap(), Some(200_000));
        assert_eq!(parse_context_window_input("200k").unwrap(), Some(200_000));
        assert_eq!(parse_context_window_input("1M").unwrap(), Some(1_000_000));
        assert_eq!(parse_context_window_input("1.5M").unwrap(), Some(1_500_000));
        assert_eq!(parse_context_window_input("128,000").unwrap(), Some(128_000));
    }

    #[test]
    fn parse_treats_blank_and_dash_as_clear() {
        for raw in ["", "   ", "-", "0", "clear", "DEFAULT", "none", "预设"] {
            assert_eq!(
                parse_context_window_input(raw).unwrap(),
                None,
                "input {raw:?} should clear the declaration"
            );
        }
    }

    #[test]
    fn parse_rejects_garbage_and_out_of_range() {
        assert!(parse_context_window_input("abc").is_err());
        assert!(parse_context_window_input("200KB").is_err());
        assert!(parse_context_window_input("-5").is_err());
        assert!(parse_context_window_input("11M").is_err());
    }

    #[test]
    fn format_mirrors_desktop_decimal_units() {
        assert_eq!(format_context_window(200_000), "200K");
        assert_eq!(format_context_window(1_000_000), "1M");
        assert_eq!(format_context_window(1_500_000), "1.5M");
        assert_eq!(format_context_window(262_144), "262.1K");
        assert_eq!(format_context_window(999), "999");
    }
}
