//! 模型上下文窗口（`providers[].models[].capabilities.contextWindowTokens`）的盘点与定点编辑。
//!
//! ## 为什么不用「`load_view` → 改字段 → `save_view`」
//!
//! `ProviderRegistryStore::load_storage` 归一化时会把**缺失**的窗口按能力预设补成显式值
//! （见 ADR 2026-11-14-model-declared-context-window-precedence）。于是 view↔storage 往返会
//! 把「未声明」固化成「声明了 256000」：一次 TUI/CLI 保存之后，预设再也补不了缺，
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

/// 常用上下文窗口规格（token，十进制），与桌面端 `CONTEXT_TOKEN_PRESETS` 同口径。
///
/// 面板用 ←/→ 在这些规格间快选，手输仍接受任意 `200000` / `200K` / `1.5M`。
pub const CONTEXT_WINDOW_PRESETS: &[u32] = &[32_000, 64_000, 128_000, 256_000, 512_000, 1_000_000];

/// 模型接受的**输入模态**（图片 / 视频 / 音频）。
///
/// 与桌面端 `inputCapabilityOptions` 对齐，只覆盖输入侧三项；工具 / 流式 / 推理 /
/// 输出模态属于高级参数，不在 TUI 面板暴露（避免把 `w` 编辑器做成完整配置页）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelInputCapabilities {
    pub image: bool,
    pub video: bool,
    pub audio: bool,
}

impl ModelInputCapabilities {
    /// 人读标签（全空时写「无」，避免面板出现空白字段）。
    pub fn label(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.image {
            parts.push("图片");
        }
        if self.video {
            parts.push("视频");
        }
        if self.audio {
            parts.push("音频");
        }
        if parts.is_empty() {
            "无".to_string()
        } else {
            parts.join("/")
        }
    }
}

impl From<&crate::agent::config::ProviderModelCapabilities> for ModelInputCapabilities {
    fn from(capabilities: &crate::agent::config::ProviderModelCapabilities) -> Self {
        Self {
            image: capabilities.supports_image_input,
            video: capabilities.supports_video_input,
            audio: capabilities.supports_audio_input,
        }
    }
}

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
    /// 生效的输入模态（图片 / 视频 / 音频），供能力编辑器回显。
    pub inputs: ModelInputCapabilities,
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
    // 256000」与「预设补的 256000」。
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
                inputs: ModelInputCapabilities::from(&model.capabilities),
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
    set_model_capabilities(store, provider_id, model_id, Some(window), None)
}

/// 定点写入上下文窗口 + 输入模态（图片 / 视频 / 音频）。
///
/// - `window`：`None` = 不动；`Some(None)` = 删键（恢复跟随预设）；`Some(Some(tokens))` = 声明。
/// - `inputs`：`None` = 不动；目标值与「已声明值 / 预设生效值」相同则**不落键**
///   ——否则打开编辑器点一下保存，就会把预设补缺固化成显式声明。
///
/// 除目标键外整棵 JSON 树保持原样（不整份归一化、不碰兄弟模型）。
pub fn set_model_capabilities(
    store: &ProviderRegistryStore,
    provider_id: &str,
    model_id: &str,
    window: Option<Option<u32>>,
    inputs: Option<ModelInputCapabilities>,
) -> Result<ModelContextWindow, String> {
    if let Some(Some(tokens)) = window {
        validate_window(tokens)?;
    }
    // 只有要改模态时才回读生效能力：布尔能力由 preset 决定（通用预设分支忽略声明值），
    // 因此「改模态」必须拿生效值当基准，并把整个能力族显式落盘。
    let effective_capabilities = inputs.map(|_| {
        store
            .load_view()
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.id == model_id || model.model == model_id)
            })
            .map(|model| model.capabilities.clone())
            .unwrap_or_default()
    });

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

    if let Some(window) = window {
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
    }

    if let Some(desired) = inputs {
        let effective = effective_capabilities.unwrap_or_default();
        let inherited = ModelInputCapabilities::from(&effective);
        // 目标与生效值一致 ⇒ 什么都不写：打开面板点一下保存，不得把预设固化成显式声明。
        if desired != inherited {
            let object = model
                .as_object_mut()
                .ok_or_else(|| "模型条目不是 JSON 对象".to_string())?;
            // 预设分支会忽略声明布尔值（`normalize_model_capabilities`），故必须落下
            // 「custom + 完整能力族」才能让勾选真的生效；其余能力保持当前生效值，
            // 不能因为切 custom 把推理/工具/流式静默关掉。
            object.insert("capabilityPreset".to_string(), json!("custom"));
            let capabilities = object
                .entry("capabilities".to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !capabilities.is_object() {
                *capabilities = Value::Object(serde_json::Map::new());
            }
            let capabilities = capabilities
                .as_object_mut()
                .expect("capabilities 已保证是对象");
            for (key, value) in [
                ("supportsTools", effective.supports_tools),
                ("supportsStreaming", effective.supports_streaming),
                ("supportsReasoning", effective.supports_reasoning),
                ("supportsImageInput", desired.image),
                ("supportsVideoInput", desired.video),
                ("supportsAudioInput", desired.audio),
                ("supportsImageOutput", effective.supports_image_output),
                ("supportsVideoOutput", effective.supports_video_output),
                ("supportsAudioOutput", effective.supports_audio_output),
            ] {
                capabilities.insert(key.to_string(), json!(value));
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

    #[test]
    fn presets_are_decimal_and_parse_back_exactly() {
        assert_eq!(
            CONTEXT_WINDOW_PRESETS,
            &[32_000, 64_000, 128_000, 256_000, 512_000, 1_000_000]
        );
        for preset in CONTEXT_WINDOW_PRESETS {
            let label = format_context_window(*preset);
            assert_eq!(
                parse_context_window_input(&label).expect("preset parses"),
                Some(*preset),
                "{label} 必须能原样解析回 {preset}"
            );
        }
    }

    #[test]
    fn changing_modalities_switches_to_custom_and_keeps_other_capabilities() {
        let root = temp_root("inputs");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);

        let updated = set_model_capabilities(
            &store,
            "provider-demo",
            "model-declared",
            None,
            Some(ModelInputCapabilities {
                image: false,
                video: true,
                audio: false,
            }),
        )
        .expect("set inputs");

        assert_eq!(
            updated.inputs,
            ModelInputCapabilities {
                image: false,
                video: true,
                audio: false,
            }
        );
        assert_eq!(updated.declared, Some(200_000), "窗口不得被顺带改写");

        let raw = read_registry_value(&store).expect("read");
        let model = &raw["providers"][0]["models"][0];
        assert_eq!(model["capabilityPreset"], json!("custom"));
        let capabilities = &model["capabilities"];
        assert_eq!(capabilities["supportsImageInput"], json!(false));
        assert_eq!(capabilities["supportsVideoInput"], json!(true));
        assert_eq!(capabilities["supportsAudioInput"], json!(false));
        // 未改动的能力沿用生效值，不能因切 custom 静默关掉。
        assert_eq!(capabilities["supportsTools"], json!(true));
        assert_eq!(capabilities["supportsStreaming"], json!(true));
        assert_eq!(capabilities["supportsReasoning"], json!(false));
        assert_eq!(capabilities["contextWindowTokens"], json!(200_000));
    }

    #[test]
    fn unchanged_modalities_write_nothing() {
        let root = temp_root("inputs-noop");
        let path = write_fixture(&root);
        let store = ProviderRegistryStore::with_path(&path);
        let before = fs::read_to_string(&path).expect("read before");

        // open-ai-chat 预设的生效值是 image=true / video=false / audio=false。
        set_model_capabilities(
            &store,
            "provider-demo",
            "model-declared",
            None,
            Some(ModelInputCapabilities {
                image: true,
                video: false,
                audio: false,
            }),
        )
        .expect("no-op inputs");

        assert_eq!(
            fs::read_to_string(&path).expect("read after"),
            before,
            "目标值与生效值一致时不得写盘（否则会把预设固化成声明）"
        );
    }
}
