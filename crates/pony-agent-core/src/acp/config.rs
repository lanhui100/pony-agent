//! ACP 外部 agent 清单（`AcpAgentSpec`）与存储契约。
//!
//! 存储位置：`~/.config/pony-agent/acp-agents.json` —— 与 `providers.json` **同级但独立**，
//! 绝不改动 provider 配置 schema（真源 `docs/architecture/acp-client-integration.md` §4 P0 表）。
//!
//! 失败语义（禁吞异常）：读/写/解析/校验失败一律映射到 [`AcpConfigError`] 的 4 个稳定错误码，
//! 不静默回落、不 panic。**唯一的回落**是「清单文件不存在」这一显式语义 ⇒ 用内置两条默认
//! （`opencode acp` / `agy acp`）；文件存在但损坏则报错而不是悄悄用默认覆盖用户的清单。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 配置目录名（与 `providers.json` 同属 `pony-agent` 目录）。
pub const ACP_CONFIG_DIR_NAME: &str = "pony-agent";

/// 清单文件名；完整路径 = `<用户配置目录>/pony-agent/acp-agents.json`。
pub const ACP_AGENTS_FILE_NAME: &str = "acp-agents.json";

/// 一条外部 ACP agent 的启动规格（用户可增删的清单条目）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentSpec {
    /// 稳定标识（`acp_delegate` 的 `agent` 参数按此解析），如 `opencode`。
    pub id: String,
    /// UI 展示名。
    pub label: String,
    /// 可执行文件名或绝对路径（按 `PATH` 解析）。
    pub command: String,
    /// 传给命令的参数（opencode/antigravity 均为 `["acp"]`）。
    #[serde(default)]
    pub args: Vec<String>,
    /// 额外环境变量；实际注入仍受宿主 env 白名单约束。
    #[serde(default)]
    pub env: Vec<AcpEnvVariable>,
    /// 默认工作目录（必须是绝对路径；`session/new` 的 `cwd` 由此派生）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_cwd: Option<String>,
    /// 宿主能力提示：决定 `initialize` 时声明哪些 `clientCapabilities`（fail-closed 的输入）。
    #[serde(default)]
    pub capabilities_hint: AcpCapabilitiesHint,
}

/// 清单里的环境变量条目（独立于协议 `EnvVariable`，避免与线上 wire 类型耦合）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpEnvVariable {
    pub name: String,
    pub value: String,
}

/// 宿主能力提示：`false` 表示整块能力不下放（外部 agent 只能在自己的子进程里做）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcpCapabilitiesHint {
    /// 是否声明 `clientCapabilities.fs.{readTextFile,writeTextFile}`。
    #[serde(default)]
    pub fs: bool,
    /// 是否声明 `clientCapabilities.terminal`。
    #[serde(default)]
    pub terminal: bool,
}

/// 内置 agent 的静态描述（避免在常量里构造 `String`/`Vec`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinAcpAgent {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

/// 内置两条默认 agent（真源 §1/§4：`opencode acp`、`agy acp`）。
pub const BUILTIN_AGENTS: &[BuiltinAcpAgent] = &[
    BuiltinAcpAgent {
        id: "opencode",
        label: "opencode",
        command: "opencode",
        args: &["acp"],
    },
    BuiltinAcpAgent {
        id: "antigravity",
        label: "Antigravity",
        command: "agy",
        args: &["acp"],
    },
];

/// 清单读写错误；`code()` 是错误码唯一真源（与 §3.2.9 的 `acp_*` 前缀对齐）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpConfigError {
    /// 文件读取 / 写入失败。
    Io(String),
    /// JSON 语法或结构不合法。
    Parse(String),
    /// 规格不合法（缺 id / command 为空 / `default_cwd` 非绝对路径等）。
    Invalid(String),
    /// 指定 id 的 agent 不存在。
    NotFound(String),
}

impl AcpConfigError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "acp_config_io",
            Self::Parse(_) => "acp_config_parse",
            Self::Invalid(_) => "acp_config_invalid",
            Self::NotFound(_) => "acp_config_agent_not_found",
        }
    }
}

impl std::fmt::Display for AcpConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Io(message) | Self::Parse(message) | Self::Invalid(message) => message,
            Self::NotFound(message) => message,
        };
        write!(formatter, "{}: {message}", self.code())
    }
}

impl std::error::Error for AcpConfigError {}

/// 清单文件完整路径（`<config_dir>/pony-agent/acp-agents.json`）。
///
/// 必须是**绝对路径**；目录不存在时由 [`save_agents`] 创建。
pub fn agents_file_path() -> PathBuf {
    // `dirs::config_dir()` 在 Linux = `$XDG_CONFIG_HOME`（缺省 `$HOME/.config`），
    // 与 `providers.json` / `model-preferences.json` 同一解析口径（单一真值，不另立名单）。
    let mut path = dirs::config_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(ACP_CONFIG_DIR_NAME);
    path.push(ACP_AGENTS_FILE_NAME);
    path
}

/// 读取用户清单；文件不存在时返回内置默认。
pub fn load_agents() -> Result<Vec<AcpAgentSpec>, AcpConfigError> {
    let path = agents_file_path();
    if !path.exists() {
        return Ok(builtin_agent_specs());
    }
    let raw = std::fs::read_to_string(&path).map_err(|error| {
        AcpConfigError::Io(format!("读取 ACP agent 清单 {} 失败：{error}。", path.display()))
    })?;
    // 未知字段容错：不启用 `deny_unknown_fields`（清单随官方 schema 演进，未知键必须被忽略）。
    let agents: Vec<AcpAgentSpec> = serde_json::from_str(&raw).map_err(|error| {
        AcpConfigError::Parse(format!(
            "解析 ACP agent 清单 {} 失败：{error}。",
            path.display()
        ))
    })?;
    for spec in &agents {
        validate_spec(spec)?;
    }
    Ok(agents)
}

/// 原子写回用户清单（`*.tmp` + rename：拒绝半截文件）。
pub fn save_agents(agents: &[AcpAgentSpec]) -> Result<(), AcpConfigError> {
    for spec in agents {
        validate_spec(spec)?;
    }
    let path = agents_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AcpConfigError::Io(format!(
                "创建 ACP 配置目录 {} 失败：{error}。",
                parent.display()
            ))
        })?;
    }
    let payload = serde_json::to_string_pretty(agents).map_err(|error| {
        AcpConfigError::Parse(format!("序列化 ACP agent 清单失败：{error}。"))
    })?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, payload).map_err(|error| {
        AcpConfigError::Io(format!("写入 ACP agent 清单临时文件 {} 失败：{error}。", tmp.display()))
    })?;
    std::fs::rename(&tmp, &path).map_err(|error| {
        // 落盘失败必须清理临时文件，否则下次写入会撞上陈旧半成品。
        let _ = std::fs::remove_file(&tmp);
        AcpConfigError::Io(format!(
            "落盘 ACP agent 清单 {} 失败：{error}。",
            path.display()
        ))
    })
}

/// 把 [`BUILTIN_AGENTS`] 物化为 [`AcpAgentSpec`]。
pub fn builtin_agent_specs() -> Vec<AcpAgentSpec> {
    BUILTIN_AGENTS
        .iter()
        .map(|builtin| AcpAgentSpec {
            id: builtin.id.to_string(),
            label: builtin.label.to_string(),
            command: builtin.command.to_string(),
            args: builtin.args.iter().map(|arg| (*arg).to_string()).collect(),
            env: Vec::new(),
            default_cwd: None,
            capabilities_hint: AcpCapabilitiesHint::default(),
        })
        .collect()
}

/// 按 id 解析一条规格（先用户清单，后内置默认）。
pub fn resolve_agent(id: &str) -> Result<AcpAgentSpec, AcpConfigError> {
    let agents = load_agents()?;
    if let Some(spec) = agents.into_iter().find(|spec| spec.id == id) {
        return Ok(spec);
    }
    // 清单存在但未收录该 id：内置默认仍是合法的兜底（用户清单是"增补"而非"替换"）。
    if let Some(spec) = builtin_agent_specs()
        .into_iter()
        .find(|spec| spec.id == id)
    {
        return Ok(spec);
    }
    Err(AcpConfigError::NotFound(format!(
        "未找到 id 为 `{id}` 的 ACP agent（用户清单与内置默认均无）。"
    )))
}

/// 规格校验（fail-closed）：id/command 非空、`default_cwd` 必须是绝对路径（§3.2.2）。
fn validate_spec(spec: &AcpAgentSpec) -> Result<(), AcpConfigError> {
    if spec.id.trim().is_empty() {
        return Err(AcpConfigError::Invalid(
            "ACP agent 的 id 不能为空。".to_string(),
        ));
    }
    if spec.command.trim().is_empty() {
        return Err(AcpConfigError::Invalid(format!(
            "ACP agent `{}` 的 command 不能为空。",
            spec.id
        )));
    }
    if let Some(cwd) = spec.default_cwd.as_deref() {
        if !Path::new(cwd).is_absolute() {
            return Err(AcpConfigError::Invalid(format!(
                "ACP agent `{}` 的 default_cwd 必须是绝对路径（§3.2.2），实际 `{cwd}`。",
                spec.id
            )));
        }
    }
    Ok(())
}
