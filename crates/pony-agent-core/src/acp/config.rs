//! ACP 外部 agent 清单（`AcpAgentSpec`）与存储契约。
//!
//! 存储位置：`~/.config/pony-agent/acp-agents.json` —— 与 `providers.json` **同级但独立**，
//! 绝不改动 provider 配置 schema（真源 `docs/architecture/acp-client-integration.md` §4 P0 表）。
//!
//! 失败语义（禁吞异常）：读/写/解析/校验失败一律映射到 [`AcpConfigError`] 的 4 个稳定错误码，
//! 不静默回落、不 panic。**唯一的回落**是「清单文件不存在」这一显式语义 ⇒ 用内置两条默认
//! （`opencode acp` / `agy acp`）；文件存在但损坏则报错而不是悄悄用默认覆盖用户的清单。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::acp::protocol::{
    EnvVariable, HttpHeader, McpHttpType, McpServer, McpServerHttp, McpServerSse, McpServerStdio,
    McpSseType,
};
use crate::agent::mcp_client::is_denied_spec_env_key;
use crate::agent::mcp_config::load_mcp_config;

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
    /// `session/new` 的 `mcpServers` 三态（B174，Lead 裁决：用 `Option<Vec<..>>` 表达）。
    ///
    /// - **字段缺省 ⇒ `None`**：inherit —— 复用 Pony 既有 MCP 配置（`mcp-config-v1`，
    ///   见 [`resolve_mcp_servers`] 文档；仅能产出 stdio 条目）；
    /// - **显式 `[]` ⇒ `Some(vec![])`**：显式禁用 —— 不下发任何 server；
    /// - **显式条目 ⇒ `Some(list)`**：仅用条目（stdio/http/sse 三形态均可透传给外部 agent），
    ///   **不**叠加 Pony 既有配置。
    ///
    /// `#[serde(default)]` 让缺省字段落到 `None`，而显式 `[]` 反序列化成 `Some(vec![])`，
    /// 两者因此可区分；`None` 时 `skip_serializing_if` 保持旧清单文件逐字节兼容。
    #[serde(
        default,
        rename = "mcpServers",
        skip_serializing_if = "Option::is_none"
    )]
    pub mcp_servers: Option<Vec<AcpMcpServerSpec>>,
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
        AcpConfigError::Io(format!(
            "读取 ACP agent 清单 {} 失败：{error}。",
            path.display()
        ))
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
    let payload = serde_json::to_string_pretty(agents)
        .map_err(|error| AcpConfigError::Parse(format!("序列化 ACP agent 清单失败：{error}。")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, payload).map_err(|error| {
        AcpConfigError::Io(format!(
            "写入 ACP agent 清单临时文件 {} 失败：{error}。",
            tmp.display()
        ))
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
            mcp_servers: None,
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
    if let Some(spec) = builtin_agent_specs().into_iter().find(|spec| spec.id == id) {
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

// ───────────────────────────── mcpServers 三态（B174） ─────────────────────────────

/// 单条用户声明的 MCP server（字段语义对齐 ACP v1 wire `McpServer`，camelCase）。
///
/// 显式条目允许 stdio / http / sse 三形态**透传**给外部 agent：http/sse 只做 wire 透传，
/// Pony 不启动本地进程（Pony 自己的 MCP 配置 `mcp-config-v1` 只支持 stdio，那是 inherit 的事）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpMcpServerSpec {
    /// server 名（作为 wire `name`）。
    pub name: String,
    /// 传输形态（wire 侧展开为 `type` 判别 + 各传输字段）。
    pub transport: AcpMcpTransportSpec,
    /// 宿主是否自动批准该 server 的工具调用；缺省 = 走常规审批（fail-closed）。
    /// 注意：ACP v1 wire 无 `autoApprove` 字段，本字段是 Pony 侧策略提示，**不进 wire**。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_approve: Option<bool>,
}

/// 传输形态：与 wire [`McpServer`] 的 stdio / http / sse 三变体一一对应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpMcpTransportSpec {
    #[serde(rename_all = "camelCase")]
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        /// 环境变量：只允许 `${env:VAR}` 间接引用（Pony mcp-config 纪律）；转换时逐键走
        /// [`crate::agent::mcp_client::is_denied_spec_env_key`]：**命中即 fail-closed 报错**
        /// （不是静默过滤；诊断只报键名，绝不回显值）。provider key 不透传。
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    #[serde(rename_all = "camelCase")]
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headers: Vec<HttpHeader>,
    },
    #[serde(rename_all = "camelCase")]
    Sse {
        url: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headers: Vec<HttpHeader>,
    },
}

/// 把 spec 的 `mcpServers` 解析为 ACP v1 wire 的 `Vec<McpServer>`（B174 桥接唯一入口）。
///
/// # 三态语义（Lead 裁决）
/// - `None`（缺省）：**inherit** —— 复用 Pony 既有 MCP 配置；
/// - `Some(vec![])`：**显式禁用** —— 返回空 `Vec`，不下发任何 server；
/// - `Some(list)`：**仅用条目** —— 逐条转换，不叠加 inherit。
///
/// # inherit 真源与丢弃字段
/// 走 [`load_mcp_config`]（`mcp-config-v1`）：项目层 `<workspace_root>/.mcp.json` + 全局层
/// `$PONY_AGENT_HOME/mcp.json`，loader 自身 **fail-soft**（单条坏配置只禁用该条 + 产
/// `McpConfigDiagnostic`，不拖垮启动）。转换只取 `McpServerEntry` 的
/// `id` / `spec.{command,args,env}`：
/// - `id` → wire `name`；`command` / `args` / `env` 原样映射（env 已完成 `${env:VAR}` 解析）；
/// - **丢弃** Pony 专属字段：`source`（Global/Workspace）、`requires_approval`、
///   `trustReadOnlyHint`、`disabled` 条目、`McpConfigDiagnostic` 本身；
/// - `mcp-config-v1` 只允许 stdio（schema `not:{required:[url]}`；出现 `url` 即被 loader 拒绝），
///   故 **inherit 只能产出 [`McpServer::Stdio`]**。
///
/// # diagnostics 不得吞
/// inherit 的每条 `McpConfigDiagnostic` 都以 `code` + `server` + `message` 记入 stderr
/// （`[acp][config]` 前缀）；文件级致命诊断（`invalid_json` / `invalid_root` /
/// `unsupported_schema_version` / `invalid_mcp_servers` 等，`server == None`）直接 fail-closed
/// 报错，不静默降级为空列表。
///
/// # env 纪律
/// - inherit：loader 已按 `is_denied_spec_env_key` fail-closed 禁用违规条目；本层再做一次键级
///   防御（命中 ⇒ 丢弃该条 + stderr 诊断），绝不把 provider key 交给外部 agent；
/// - 显式条目：任一 env 键命中 deny 表 ⇒ **整条报错**（`AcpConfigError::Invalid`），不静默过滤。
///
/// # 错误码（沿用 [`AcpConfigError`] 既有 4 码，不新增）
/// - `acp_config_invalid`：显式条目非法（name/command/url 为空、inherit 整份配置不可用、
///   env deny 命中 fail-closed）；
/// - `acp_config_parse`：inherit 的 JSON 层解析失败（loader `invalid_json` / `invalid_root`）；
/// - `acp_config_io`：取 `workspace_root` 失败（`std::env::current_dir` 等）；
/// - `acp_config_agent_not_found`：与本函数无关（保留既有语义）。
///
/// `workspace_root` 取 `spec.default_cwd`（缺省为进程当前目录）；会话路径请用
/// [`resolve_mcp_servers_in`] 传入真实 session `cwd`。
pub fn resolve_mcp_servers(spec: &AcpAgentSpec) -> Result<Vec<McpServer>, AcpConfigError> {
    let workspace_root = match spec.default_cwd.as_deref() {
        Some(cwd) => PathBuf::from(cwd),
        None => std::env::current_dir().map_err(|error| {
            AcpConfigError::Io(format!(
                "解析 ACP agent `{}` 的 inherit MCP 配置时取不到当前目录：{error}。",
                spec.id
            ))
        })?,
    };
    resolve_mcp_servers_in(spec, &workspace_root)
}

/// [`resolve_mcp_servers`] 的会话路径版本（additive）：inherit 以真实 `workspace_root`
/// （session 的 `cwd`）加载 Pony MCP 配置。
pub fn resolve_mcp_servers_in(
    spec: &AcpAgentSpec,
    workspace_root: &Path,
) -> Result<Vec<McpServer>, AcpConfigError> {
    match &spec.mcp_servers {
        // 缺省 ⇒ inherit：复用 Pony 既有 MCP 配置（仅 stdio）。
        None => inherit_pony_mcp_servers(spec, workspace_root),
        // 显式 `[]` ⇒ 空 Vec；显式条目 ⇒ 仅用条目。
        Some(explicit) => explicit
            .iter()
            .map(convert_explicit_mcp_server)
            .collect::<Result<Vec<_>, _>>(),
    }
}

/// inherit：`load_mcp_config` → wire `McpServer::Stdio`（diagnostics 全量记账，不吞）。
fn inherit_pony_mcp_servers(
    spec: &AcpAgentSpec,
    workspace_root: &Path,
) -> Result<Vec<McpServer>, AcpConfigError> {
    let load = load_mcp_config(workspace_root);

    // 诊断不得吞：逐条记 code/server/message（绝不回显 env 值），文件级致命项 fail-closed。
    let mut fatal: Option<&crate::agent::mcp_config::McpConfigDiagnostic> = None;
    for diagnostic in &load.diagnostics {
        eprintln!(
            "[acp][config] ACP agent `{}` inherit MCP 诊断：code={} server={:?} message={}",
            spec.id, diagnostic.code, diagnostic.server, diagnostic.message
        );
        if diagnostic.server.is_none() && fatal.is_none() {
            fatal = Some(diagnostic);
        }
    }
    if let Some(diagnostic) = fatal {
        let message = format!(
            "ACP agent `{}` 的 inherit MCP 配置不可用（code={}）：{}",
            spec.id, diagnostic.code, diagnostic.message
        );
        return Err(match diagnostic.code.as_str() {
            "invalid_json" | "invalid_root" => AcpConfigError::Parse(message),
            _ => AcpConfigError::Invalid(message),
        });
    }

    let mut servers = Vec::with_capacity(load.servers.len());
    for entry in &load.servers {
        // 键级防御：loader 已 fail-closed 禁用违规条目；这里再挡一层（绝不透传 provider key）。
        if let Some((key, _)) = entry
            .spec
            .env
            .iter()
            .find(|(key, _)| is_denied_spec_env_key(key))
        {
            eprintln!(
                "[acp][config] ACP agent `{}` inherit MCP server `{}` 的 env 键 `{key}` 命中 deny 表，已丢弃该条（fail-closed）",
                spec.id, entry.id
            );
            continue;
        }
        servers.push(McpServer::Stdio(McpServerStdio {
            name: entry.id.clone(),
            command: entry.spec.command.clone(),
            args: entry.spec.args.clone(),
            env: entry
                .spec
                .env
                .iter()
                .map(|(name, value)| EnvVariable {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        }));
    }
    Ok(servers)
}

/// 显式条目 → wire：stdio 校验 command/env（deny ⇒ fail-closed 报错）；http/sse 仅做 wire 透传。
fn convert_explicit_mcp_server(spec: &AcpMcpServerSpec) -> Result<McpServer, AcpConfigError> {
    if spec.name.trim().is_empty() {
        return Err(AcpConfigError::Invalid(
            "ACP 显式 mcpServers 条目的 name 不能为空。".to_string(),
        ));
    }
    match &spec.transport {
        AcpMcpTransportSpec::Stdio { command, args, env } => {
            if command.trim().is_empty() {
                return Err(AcpConfigError::Invalid(format!(
                    "ACP 显式 mcpServers 条目 `{}` 的 stdio.command 不能为空。",
                    spec.name
                )));
            }
            let mut wire_env = Vec::with_capacity(env.len());
            for (key, value) in env {
                // Lead 裁决 4：显式条目 env 命中 deny 表必须 fail-closed 报错（非静默过滤）。
                if is_denied_spec_env_key(key) {
                    return Err(AcpConfigError::Invalid(format!(
                        "ACP 显式 mcpServers 条目 `{}` 的 env 键 `{key}` 属于进程/加载器劫持或凭据通道向量（deny 表），已 fail-closed 拒绝该配置。",
                        spec.name
                    )));
                }
                wire_env.push(EnvVariable {
                    name: key.clone(),
                    value: value.clone(),
                });
            }
            Ok(McpServer::Stdio(McpServerStdio {
                name: spec.name.clone(),
                command: command.clone(),
                args: args.clone(),
                env: wire_env,
            }))
        }
        AcpMcpTransportSpec::Http { url, headers } => {
            if url.trim().is_empty() {
                return Err(AcpConfigError::Invalid(format!(
                    "ACP 显式 mcpServers 条目 `{}` 的 http.url 不能为空。",
                    spec.name
                )));
            }
            Ok(McpServer::Http(McpServerHttp {
                name: spec.name.clone(),
                url: url.clone(),
                headers: headers.clone(),
                transport: McpHttpType::Http,
            }))
        }
        AcpMcpTransportSpec::Sse { url, headers } => {
            if url.trim().is_empty() {
                return Err(AcpConfigError::Invalid(format!(
                    "ACP 显式 mcpServers 条目 `{}` 的 sse.url 不能为空。",
                    spec.name
                )));
            }
            Ok(McpServer::Sse(McpServerSse {
                name: spec.name.clone(),
                url: url.clone(),
                headers: headers.clone(),
                transport: McpSseType::Sse,
            }))
        }
    }
}
