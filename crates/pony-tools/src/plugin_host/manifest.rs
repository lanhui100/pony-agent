//! 插件 manifest 契约（`pony-plugin-v1`）的解析与校验。
//!
//! 契约（ADR 0067）：
//! ```json
//! {
//!   "apiVersion": "pony-plugin-v1",
//!   "name": "string",
//!   "version": "semver",
//!   "capabilities": ["tools", "hooks"],
//!   "tools": [{ "name": "string", "description": "string", "inputSchema": {} }],
//!   "hookPoints": ["ToolCallStart"]
//! }
//! ```
//!
//! 失败闭合原则：
//! - **未知字段一律拒绝**（`deny_unknown_fields`）：manifest 是信任面的一部分，静默忽略
//!   未知键会让插件作者以为某项授权/声明生效而实际未生效；
//! - `apiVersion` 必须精确等于 [`PLUGIN_API_VERSION`]（不做前缀/子串兼容猜测）；
//! - 未知能力名一律拒绝（能力默认拒绝，不做"降级忽略"）；
//! - 名称字符集受限：插件名与工具名会拼进模型可见工具名 `plugin__<name>__<tool>`，
//!   因此禁止 `__`（分隔符）与空白/控制字符，避免命名歧义与注入式伪装。
//!
//! 本模块只做**纯数据校验**，不接触进程、环境或文件系统。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// 本宿主实现的插件契约版本。
pub const PLUGIN_API_VERSION: &str = "pony-plugin-v1";

/// 能力名：提供工具（v1 唯一被宿主实现的能力）。
pub const CAPABILITY_TOOLS: &str = "tools";
/// 能力名：hook 点位。v1 **不迁移** hooks（ADR 0067 Non-Goal），声明它会在协商阶段被拒绝。
pub const CAPABILITY_HOOKS: &str = "hooks";

/// v1 已知能力名集合（未知名在解析阶段即拒绝）。
pub const KNOWN_CAPABILITIES: &[&str] = &[CAPABILITY_TOOLS, CAPABILITY_HOOKS];

const MAX_NAME_LEN: usize = 64;
const MAX_TOOLS: usize = 256;
const MAX_DESCRIPTION_LEN: usize = 4_096;

/// manifest 中的一个工具声明。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginToolDecl {
    /// 工具名（插件内唯一；不得含 `__` 与空白）。
    pub name: String,
    /// 面向模型的说明（可为空）。
    #[serde(default)]
    pub description: String,
    /// 入参 JSON Schema（必须是 object；缺省为 `{"type":"object"}`）。
    #[serde(default = "default_input_schema")]
    pub input_schema: Value,
}

fn default_input_schema() -> Value {
    json!({ "type": "object" })
}

/// `plugin.json` 的内存表示（契约 v1 的六个字段，一个不多一个不少）。
///
/// 说明（与 ADR 0067 的偏差记录）：ADR 决策 3 提到的 `fsRoots` / `exec` / `network`
/// 信任声明**不在 v1 manifest 中承载**——v1 插件不获得文件系统根、命令执行与网络声明面
/// （能力默认拒绝的最小实现）；若后续引入，需按 ADR 修订流程加字段并同步 conformance kit
/// （ADR 0068），避免在本批用未经消费的字段造出"声明了但不生效"的假承诺。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    /// 契约版本，必须等于 [`PLUGIN_API_VERSION`]。
    pub api_version: String,
    /// 插件名（工具命名空间 `plugin__<name>__<tool>` 的中段）。
    pub name: String,
    /// 插件版本（`major.minor.patch[-pre][+build]` 形态）。
    pub version: String,
    /// 声明能力（未知名拒绝；默认空 = 无任何能力）。
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 工具声明。
    #[serde(default)]
    pub tools: Vec<PluginToolDecl>,
    /// hook 点位（v1 只做记录，不接线；见 ADR 0067 Non-Goal）。
    #[serde(default)]
    pub hook_points: Vec<String>,
}

impl PluginManifest {
    /// 从 JSON 文本解析并校验。
    ///
    /// 错误消息带稳定前缀 `plugin_manifest_invalid:` / `plugin_api_version_mismatch:` /
    /// `plugin_capability_unsupported:`，调用方可按前缀分支（纯 `Result<_, String>` 契约
    /// 由 `tests/plugin_host_contract.rs` 冻结）。
    pub fn parse(json_text: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(json_text).map_err(|err| {
            format!("plugin_manifest_invalid: 插件 manifest 不是合法契约 JSON（{err}）。")
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// 契约校验（幂等；[`PluginManifest::parse`] 已调用，`spawn` 会再校验一次以防
    /// 调用方用 struct literal 绕过解析）。
    pub fn validate(&self) -> Result<(), String> {
        if self.api_version != PLUGIN_API_VERSION {
            return Err(format!(
                "plugin_api_version_mismatch: 插件契约版本 `{}` 与宿主支持的 `{PLUGIN_API_VERSION}` 不兼容。",
                self.api_version
            ));
        }

        validate_identifier("插件名", &self.name)?;
        validate_semver(&self.version)?;

        if self.capabilities.len() > KNOWN_CAPABILITIES.len() {
            return Err(format!(
                "plugin_manifest_invalid: 能力声明数量 {} 超过已知能力上限 {}（重复声明不受支持）。",
                self.capabilities.len(),
                KNOWN_CAPABILITIES.len()
            ));
        }
        for capability in &self.capabilities {
            if !KNOWN_CAPABILITIES.contains(&capability.as_str()) {
                return Err(format!(
                    "plugin_capability_unsupported: 未知能力 `{capability}`（v1 仅支持 {}）。",
                    KNOWN_CAPABILITIES.join(" / ")
                ));
            }
        }
        // 重复能力声明：拒绝而非去重——重复往往意味着作者对协商语义有误解。
        for (index, capability) in self.capabilities.iter().enumerate() {
            if self.capabilities[..index].contains(capability) {
                return Err(format!(
                    "plugin_manifest_invalid: 能力 `{capability}` 重复声明。"
                ));
            }
        }

        if self.tools.len() > MAX_TOOLS {
            return Err(format!(
                "plugin_manifest_invalid: 工具声明数量 {} 超过上限 {MAX_TOOLS}。",
                self.tools.len()
            ));
        }
        for (index, tool) in self.tools.iter().enumerate() {
            validate_identifier("工具名", &tool.name)?;
            if tool.name.starts_with("plugin__") || tool.name.contains("__") {
                return Err(format!(
                    "plugin_manifest_invalid: 工具名 `{}` 不得包含命名空间分隔符 `__`。",
                    tool.name
                ));
            }
            if tool.description.chars().count() > MAX_DESCRIPTION_LEN {
                return Err(format!(
                    "plugin_manifest_invalid: 工具 `{}` 的 description 超过 {MAX_DESCRIPTION_LEN} 字符上限。",
                    tool.name
                ));
            }
            if !tool.input_schema.is_object() {
                return Err(format!(
                    "plugin_manifest_invalid: 工具 `{}` 的 inputSchema 必须是 JSON object。",
                    tool.name
                ));
            }
            if self.tools[..index]
                .iter()
                .any(|previous| previous.name == tool.name)
            {
                return Err(format!(
                    "plugin_manifest_invalid: 工具名 `{}` 重复声明。",
                    tool.name
                ));
            }
        }

        for hook_point in &self.hook_points {
            if hook_point.trim().is_empty() {
                return Err("plugin_manifest_invalid: hookPoints 中不允许空标签。".to_string());
            }
        }

        Ok(())
    }

    /// manifest 内容指纹（`sha256:<hex>`）：ADR 0067 信任面的「内容哈希」一半。
    ///
    /// 用**结构体字段顺序**序列化而非原始文件字节：作者改格式（空格/键序）不会让已信任的
    /// 插件失效，但任何语义字段变化（名称/版本/能力/工具/点位）必然换哈希。
    pub fn fingerprint(&self) -> String {
        let canonical = serde_json::to_string(self)
            .unwrap_or_else(|_| format!("{}|{}|{}", self.api_version, self.name, self.version));
        format!("sha256:{}", hex_sha256(canonical.as_bytes()))
    }

    /// 是否声明了某能力。
    pub fn declares(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|item| item == capability)
    }

    /// 声明了 `tools` 能力时的工具声明表。
    pub fn declares_tools(&self) -> bool {
        self.declares(CAPABILITY_TOOLS)
    }
}

/// 名称字符集校验：非空、长度受限、`[A-Za-z0-9_-]`、不得含 `__` 与空白。
fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("plugin_manifest_invalid: {label}不能为空。"));
    }
    if trimmed != value {
        return Err(format!(
            "plugin_manifest_invalid: {label} `{value}` 首尾不得含空白。"
        ));
    }
    if value.chars().count() > MAX_NAME_LEN {
        return Err(format!(
            "plugin_manifest_invalid: {label} `{value}` 超过 {MAX_NAME_LEN} 字符上限。"
        ));
    }
    if value.contains("__") {
        return Err(format!(
            "plugin_manifest_invalid: {label} `{value}` 不得包含命名空间分隔符 `__`。"
        ));
    }
    for ch in value.chars() {
        let allowed = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-';
        if !allowed {
            return Err(format!(
                "plugin_manifest_invalid: {label} `{value}` 含非法字符 `{ch}`（仅允许 [A-Za-z0-9_-]）。"
            ));
        }
    }
    Ok(())
}

/// 极简 semver 校验：`major.minor.patch` 三段数字 + 可选 `-pre` / `+build`。
///
/// 不引入 semver crate：v1 只要求"版本形状合法"，不做区间匹配（兼容策略由 apiVersion 承担）。
fn validate_semver(value: &str) -> Result<(), String> {
    let core = value.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(format!(
            "plugin_manifest_invalid: version `{value}` 不是 `major.minor.patch` 形态。"
        ));
    }
    for part in parts {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(format!(
                "plugin_manifest_invalid: version `{value}` 的段 `{part}` 必须是数字。"
            ));
        }
    }
    Ok(())
}

/// 十六进制 sha256（小写）。
pub fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "apiVersion": "pony-plugin-v1",
        "name": "echo",
        "version": "0.1.0",
        "capabilities": ["tools"],
        "tools": [{"name": "echo", "description": "echo back", "inputSchema": {"type": "object"}}],
        "hookPoints": []
    }"#;

    #[test]
    fn parses_valid_manifest_and_reports_tools() {
        let manifest = PluginManifest::parse(VALID).expect("valid manifest must parse");
        assert_eq!(manifest.name, "echo");
        assert!(manifest.declares_tools());
        assert_eq!(manifest.tools.len(), 1);
        assert!(manifest.fingerprint().starts_with("sha256:"));
    }

    #[test]
    fn rejects_wrong_api_version_and_unknown_capability() {
        let wrong_version = VALID.replace("pony-plugin-v1", "pony-plugin-v2");
        let err = PluginManifest::parse(&wrong_version).expect_err("apiVersion mismatch must fail");
        assert!(err.starts_with("plugin_api_version_mismatch:"), "{err}");

        let unknown_capability = VALID.replace(r#"["tools"]"#, r#"["teleport"]"#);
        let err = PluginManifest::parse(&unknown_capability)
            .expect_err("unknown capability must fail");
        assert!(err.starts_with("plugin_capability_unsupported:"), "{err}");
    }

    #[test]
    fn rejects_unknown_fields_and_malformed_payloads() {
        let unknown_field = VALID.replace(r#""hookPoints": []"#, r#""hookPoints": [], "trustMe": true"#);
        let err = PluginManifest::parse(&unknown_field).expect_err("unknown field must fail");
        assert!(err.starts_with("plugin_manifest_invalid:"), "{err}");

        let err = PluginManifest::parse("{not json").expect_err("malformed json must fail");
        assert!(err.starts_with("plugin_manifest_invalid:"), "{err}");

        let bad_tool = VALID.replace(r#""name": "echo""#, r#""name": "e__cho""#);
        let err = PluginManifest::parse(&bad_tool).expect_err("separator in tool name must fail");
        assert!(err.starts_with("plugin_manifest_invalid:"), "{err}");
    }

    #[test]
    fn fingerprint_changes_only_with_semantic_fields() {
        let base = PluginManifest::parse(VALID).expect("valid");
        let reordered = VALID.replace(
            r#""apiVersion": "pony-plugin-v1",
        "name": "echo","#,
            r#""name": "echo",
        "apiVersion": "pony-plugin-v1","#,
        );
        let reformatted = PluginManifest::parse(&reordered).expect("valid reordered");
        assert_eq!(
            base.fingerprint(),
            reformatted.fingerprint(),
            "键序/格式变化不得改变信任指纹"
        );

        let bumped = VALID.replace(r#""version": "0.1.0""#, r#""version": "0.1.1""#);
        let bumped = PluginManifest::parse(&bumped).expect("valid bumped");
        assert_ne!(base.fingerprint(), bumped.fingerprint(), "版本变化必须换指纹");
    }
}
