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

/// 代理环境文件目录（B183）：缺省读 `<HOME>/.pony/`。
pub const ACP_PROXY_ENV_DIR: &str = ".pony";

/// 代理环境文件名：完整路径 = `<HOME>/.pony/proxy.env`。
///
/// 背景：`pony`（TUI）常在**没有代理变量**的 shell 里启动，而 ACP 子进程（agy/opencode）需要
/// `*_proxy` 才能出网（agy 直连会被墙、`session/new` 挂 30s）。本文件让用户只需维护一份
/// `KEY=VALUE`（可带 `export ` 前缀），宿主启动 ACP 子进程时自动注入——不再需要每次手动
/// `source ~/.pony/proxy.env`。
pub const ACP_PROXY_ENV_FILE_NAME: &str = "proxy.env";

/// `inheritProxyEnv` 的缺省值（true：该 agent 读取代理文件）。
fn default_true() -> bool {
    true
}

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
    /// B183 additive：是否继承 `<HOME>/.pony/proxy.env` 的 `*_proxy` 变量（缺省 `true`）。
    ///
    /// `false` ⇒ 该 agent 不读代理文件（只保留宿主白名单 + 显式 `env`），用于「直连/自定义
    /// 代理」场景；`true` ⇒ spawn 时把 `resolve_proxy_env()` 结果并入子进程 env。
    #[serde(default = "default_true", rename = "inheritProxyEnv")]
    pub inherit_proxy_env: bool,
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
    // B230：Windows 的 `dirs::config_dir()` = KnownFolder(RoamingAppData)，忽略 XDG；而本仓
    // ACP 清单的冻结契约就是「经 `$XDG_CONFIG_HOME/pony-agent/acp-agents.json` 发现」。
    // 故显式优先读已有的 `XDG_CONFIG_HOME`（不新增 env 变量），让契约在 Windows 上同样成立，
    // 测试的隔离根也因此跨平台一致。
    let xdg_override = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut path = xdg_override
        .or_else(dirs::config_dir)
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
            inherit_proxy_env: true,
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
/// （`[acp][config]` 前缀）。**`server == None` 的文件级诊断并不一律致命**：
/// 只有 `invalid_json` / `invalid_root` / `unsupported_schema_version` 会 fail-closed 报错，
/// 其余（如 `invalid_mcp_servers`、`workspace_mcp_config_untrusted`）一律「忽略 + 记账」，
/// 降级为空 server 列表继续执行。
///
/// 理由（实现处注释同步，见 `resolve_mcp_servers` 循环内）：`.mcp.json` 是随仓库分发的文件，
/// 只要把它升级为致命，仓库里存在一份结构不合规的 `.mcp.json` 就会让**所有** `acp_delegate`
/// 一起失败——这是"一份坏的第三方文件否决整条能力"的典型事故；而 `invalid_mcp_servers`
/// 只是 `mcpServers` 不是对象的形状问题，属于可降级诊断。
///
/// 历史注记：本条文档曾把 `invalid_mcp_servers` 误列为致命（B177 ADVISORY ①），与实现不一致；
/// 本次按实现真值改正，不改任何行为。
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

    // 诊断不得吞：逐条记 code/server/message（绝不回显 env 值）。
    //
    // B2：只有「整份配置不可解析」类诊断才致命（`invalid_json` / `invalid_root`）。
    // 其余文件级诊断——典型如 `workspace_mcp_config_untrusted`（工作区 `.mcp.json` 随仓库
    // 分发、默认不可信 ⇒ loader 按既有 trust 语义**忽略**它并产出诊断）——属于「忽略 + 记账」，
    // 不得 fail-closed：否则仓库里只要存在 `.mcp.json`，所有 acp_delegate 都会因
    // `acp_config_invalid` 全废。
    let mut fatal: Option<&crate::agent::mcp_config::McpConfigDiagnostic> = None;
    for diagnostic in &load.diagnostics {
        crate::agent::otel::log::log_json_with_fields(
            "info",
            "acp inherit MCP 诊断",
            None,
            &[
                ("agent", serde_json::json!(spec.id)),
                ("code", serde_json::json!(diagnostic.code)),
                ("server", serde_json::json!(diagnostic.server)),
                ("message", serde_json::json!(diagnostic.message)),
            ],
        );
        // 文件级诊断中，仅「不可解析 / 不受支持 schema」是致命的；`workspace_mcp_config_untrusted`
        // 是既有 trust 语义的「忽略并记账」，不得升级为错误。
        let fatal_code = matches!(
            diagnostic.code.as_str(),
            "invalid_json" | "invalid_root" | "unsupported_schema_version"
        );
        if diagnostic.server.is_none() && fatal_code && fatal.is_none() {
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
            crate::agent::otel::log::log_json_with_fields(
                "warn",
                "acp inherit MCP server 的 env 键命中 deny 表，已丢弃该条（fail-closed；只记键名）",
                None,
                &[
                    ("agent", serde_json::json!(spec.id)),
                    ("server", serde_json::json!(entry.id)),
                    ("key", serde_json::json!(key)),
                ],
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

// ───────────────────────────── B183：代理环境自动注入 ─────────────────────────────

/// 解析 `<HOME>/.pony/proxy.env` 为可注入的代理变量（B183 契约；实现由 task-26 落地）。
///
/// 语义（executor 实现时必须逐条兑现）：
/// 1. 行格式：`KEY=VALUE`；忽略空行与 `#` 注释行；可选 `export ` 前缀；值两侧单/双引号去掉；
/// 2. **只保留 `*_proxy` 键**（大小写不敏感：`http_proxy` / `HTTPS_PROXY` / `all_proxy` /
///    `no_proxy` …；其余键一律丢弃）；
/// 3. 逐键过 [`crate::agent::plugin_host::capability::is_denied_env_key`]：命中即丢弃
///    （`LD_*` / `SSH_AUTH_SOCK` / provider key 等进程/凭据通道向量，绝不注入）；
/// 4. **文件不存在 ⇒ 返回空 `Vec`，不报错**（无代理环境是合法常态）；读取/解码失败也只记
///    stderr 诊断并返回空，不阻断 ACP 启动；
/// 5. 返回值按文件出现顺序（后者覆盖前者的语义由下游合并顺序决定，见 `transport::spawn` 文档）。
pub fn resolve_proxy_env() -> Vec<(String, String)> {
    // `<HOME>` 走 `dirs` 既有口径（Unix = `$HOME`；Windows 缺 HOME 时回退用户目录）。
    // B230：`PONY_AGENT_HOME` 是产品已有的“用户级全局根”显式覆盖（技能/MCP 同源），
    // 这里同口径优先——既让隔离测试在 Windows 上可用（dirs::home_dir 忽略 HOME/USERPROFILE），
    // 也不新增任何 env 变量。
    let home_override = std::env::var_os("PONY_AGENT_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from);
    let Some(home) = home_override.or_else(dirs::home_dir) else {
        // 无 HOME 是合法环境（如最小容器）：无代理可继承，不报错。
        return Vec::new();
    };
    let path = home.join(ACP_PROXY_ENV_DIR).join(ACP_PROXY_ENV_FILE_NAME);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            // 缺文件 = 无代理（常态）；其余读失败只记诊断，绝不阻断 ACP 启动。
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::agent::otel::log::log_json_with_fields(
                    "warn",
                    "acp 读取代理文件失败，按无代理处理",
                    None,
                    &[
                        ("path", serde_json::json!(path.display().to_string())),
                        ("error", serde_json::json!(error.to_string())),
                    ],
                );
            }
            return Vec::new();
        }
    };
    parse_proxy_env(&raw)
}

/// 解析 `KEY=VALUE` 行（`resolve_proxy_env` 的纯函数部分，便于无 I/O 单测）。
///
/// 只保留 `*_proxy` 键（大小写不敏感）；同键**后者覆盖前者**；命中 `is_denied_env_key`
/// 的键丢弃并只记键名（绝不回显值）。非法行（无 `=` / 空键）跳过且不 panic。
fn parse_proxy_env(raw: &str) -> Vec<(String, String)> {
    let mut resolved: Vec<(String, String)> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim();
        let Some((key, value)) = body.split_once('=') else {
            crate::agent::otel::log::log_json_with_fields(
                "info",
                "acp 代理文件行缺少 `=`，已跳过",
                None,
                &[("key", serde_json::json!(body))],
            );
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        if !is_proxy_env_key(key) {
            // G2：只注入 `*_proxy`；其余键一律丢弃（静默，多为用户的其它 shell 变量）。
            continue;
        }
        if crate::agent::plugin_host::capability::is_denied_env_key(key) {
            // G3：即使形如 `*_proxy`，命中 deny 表也不注入；只记键名，绝不记值。
            crate::agent::otel::log::log_json_with_fields(
                "warn",
                "acp 代理变量命中 deny 表，已丢弃（不注入；只记键名）",
                None,
                &[("key", serde_json::json!(key))],
            );
            continue;
        }
        let value = unquote_env_value(value.trim());
        match resolved
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(key))
        {
            Some(existing) => existing.1 = value,
            None => resolved.push((key.to_string(), value)),
        }
    }
    resolved
}

/// 是否为可注入的代理键：大小写不敏感地以 `_proxy` 结尾
///（覆盖 `http_proxy` / `HTTPS_PROXY` / `all_proxy` / `no_proxy` 及其大写形态）。
fn is_proxy_env_key(key: &str) -> bool {
    key.to_ascii_lowercase().ends_with("_proxy")
}

/// 去掉值两侧成对的单/双引号（只剥一层；不成对时原样保留）。
fn unquote_env_value(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}
