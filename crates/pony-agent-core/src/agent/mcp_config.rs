//! MCP 通道闭合（ADR 0066 / T2）：配置加载 → 工具发现 → 外部 descriptor → 治理分派。
//!
//! 本模块把 MCP stdio server 接成**一条**受治理的外部工具通道：
//!
//! ```text
//! .mcp.json / $PONY_AGENT_HOME/mcp.json
//!      │  load_mcp_config()（两层、未知字段容忍、单条坏配置 fail-soft）
//!      ▼
//! McpServerEntry ──connect──► McpToolProvider（impl ToolProvider, source_id = mcp:<server>）
//!      │                            │ tools/list → 外部 descriptor
//!      │                            │   model_name = mcp__<server>__<tool>（登记为 alias）
//!      │                            │   kind = External，权限事实 = annotations.readOnlyHint
//!      ▼                            ▼
//! McpToolChannel ──register_tool_providers──► GovernedDispatcher（授权/预算/生命周期）
//!      │
//!      └─ surface_tool_definitions()：内建面 + 外部工具（稳定排序，会话内冻结）
//! ```
//!
//! 纪律（ADR 0064 三层模型 / ADR 0066）：
//! - **只允许一条外部通道**：MCP 工具必须在 `GovernedDispatcher` 里按 `descriptor_id` 分派；
//!   禁止旁路治理管线（授权、预算预约、生命周期证据）。
//! - **失败闭合**：配置坏只禁用该 server（fail-soft），调用失败一律 fail-closed 且错误码分源
//!   （`mcp_server_unavailable` / `mcp_tool_not_found` / `mcp_timeout` / `mcp_resources_not_wired`）。
//! - **凭据不落日志**：配置只允许 `${env:VAR}` 间接引用；诊断只记 server 名与键名，绝不记值。
//! - **工具面稳定**：外部工具按 `(source_id, model_name)` 稳定排序追加在内建之后，mid-turn 不变
//!   （ADR 0065 decision 8 的 KV-cache 前缀不变量）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;

use crate::agent::mcp_client::{
    is_denied_spec_env_key, McpClient, McpClientError, McpServerSpec, McpToolSpec,
    DEFAULT_TIMEOUT, DEFAULT_CONNECT_TIMEOUT,
};
use crate::agent::tools::{
    ToolDefinition, ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy,
    ToolExposure, ToolHandlerProvenance, ToolIdentity, ToolKind, ToolPermissionDeclaration,
    ToolPermissionScope, ToolProvider, ToolProviderError,
};

/// 配置文件 schema 版本（`mcp-config-v1`）。
pub const MCP_CONFIG_SCHEMA_VERSION: &str = "mcp-config-v1";
/// 项目层配置文件名（`<workspace>/.mcp.json`）。
pub const MCP_PROJECT_CONFIG_FILE: &str = ".mcp.json";
/// 全局层配置文件名（`$PONY_AGENT_HOME/mcp.json`）。
pub const MCP_GLOBAL_CONFIG_FILE: &str = "mcp.json";
/// 全局层 home 环境变量。
pub const MCP_HOME_ENV: &str = "PONY_AGENT_HOME";
/// 工作区级 MCP 配置的信任策略（**唯一信任真值**，L3-B-A 裁决）。
///
/// `load_mcp_config_from` 是 pub，若它按「显式调用即信任」处理，就等于开了第二个绕过闸门的
/// 入口；因此信任决策必须**显式传入**，由调用方（生产入口按 env、宿主按设置、测试按用例）
/// 给出，而不是由「谁调用了哪个函数」隐式推断。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceMcpTrust {
    /// 不加载工作区级配置（默认；产出 `workspace_mcp_config_untrusted` 诊断）。
    Untrusted,
    /// 加载工作区级配置（用户显式信任：env 开关 / 设置页 / 宿主显式声明）。
    Trusted,
}

/// 信任工作区级 MCP 配置的显式开关（默认关闭）。
///
/// **供应链边界（L3-B-A）**：`<workspace>/.mcp.json` 随仓库一起分发，攻击者可控
/// （`{"command":"/bin/sh","args":["-c","id"]}` 即以宿主用户权限执行）。因此 v1 **默认不加载**
/// 工作区级配置，只有用户显式设置 `PONY_MCP_TRUST_WORKSPACE=1`（或设置页等价开关）才启用；
/// 被忽略时产生 `workspace_mcp_config_untrusted` 诊断（fail-soft，不阻断启动）。
/// `$PONY_AGENT_HOME/mcp.json` 是用户自建文件，视为可信。
pub const MCP_TRUST_WORKSPACE_ENV: &str = "PONY_MCP_TRUST_WORKSPACE";
/// v1 允许 `${env:VAR}` 引用的环境变量命名空间（L3-B-B：防止仓库配置外带宿主凭据）。
pub const MCP_ENV_NAMESPACE: &str = "PONY_MCP_";
/// 外部工具模型名前缀（`mcp__<server>__<tool>`）。
pub const MCP_TOOL_NAME_PREFIX: &str = "mcp__";

/// **信任模型（L3-B4，安全裁决）**：`annotations.readOnlyHint` 是**远端自报**，不可信。
///
/// 恶意或写错的 MCP server 只要把破坏性工具标成 `readOnlyHint: true`，就能让它在
/// `read_only` 会话里被放行。因此默认策略是 **fail-closed：不采信远端 hint，一律按
/// `WorkspaceWrite`（可写）声明**；只有当**本地配置**为该 server 显式 opt-in
/// （`trustReadOnlyHint: true` / `trust_read_only_hint: true`）时，才把 hint 当作权限事实。
/// 信任锚永远在本地配置文件里，绝不在远端响应里。
fn trust_policy() -> &'static Mutex<BTreeMap<String, bool>> {
    static POLICY: OnceLock<Mutex<BTreeMap<String, bool>>> = OnceLock::new();
    POLICY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 本地配置是否为本 server 显式 opt-in 采信远端 `readOnlyHint`（默认 `false`）。
pub fn trusts_read_only_hint(server_id: &str) -> bool {
    trust_policy()
        .lock()
        .ok()
        .and_then(|policy| policy.get(server_id).copied())
        .unwrap_or(false)
}

/// 登记本地信任决策（由配置加载器在读取 `trustReadOnlyHint` 时调用）。
pub fn set_trust_read_only_hint(server_id: &str, trust: bool) {
    if let Ok(mut policy) = trust_policy().lock() {
        policy.insert(server_id.to_string(), trust);
    }
}

/// 分源错误码（ADR 0066 decision 4）。
pub mod codes {
    /// MCP server 不可达（启动失败/握手失败/会话断开）。
    pub const SERVER_UNAVAILABLE: &str = "mcp_server_unavailable";
    /// 调用的工具不在该 server 的 `tools/list` 里。
    pub const TOOL_NOT_FOUND: &str = "mcp_tool_not_found";
    /// MCP 请求超时。
    pub const TIMEOUT: &str = "mcp_timeout";
    /// 资源面未接线（v1 结构性 fail-closed）。
    pub const RESOURCES_NOT_WIRED: &str = "mcp_resources_not_wired";
    /// server 可达但拒绝该次工具调用（JSON-RPC error）。
    pub const TOOL_ERROR: &str = "mcp_tool_error";
    /// 需要宿主审批/授权。
    pub const CAPABILITY_DENIED: &str = "capability_denied";
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 配置面
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 配置条目来自哪一层（项目层覆盖全局层）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum McpConfigSource {
    Global,
    Workspace,
}

impl McpConfigSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }
}

/// 一条可用的 MCP server 配置（已解析、已校验）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerEntry {
    /// server 名（`mcpServers` 的键），决定 `mcp__<server>__<tool>` 命名空间。
    pub id: String,
    /// 启动描述（command/args/env，env 已完成 `${env:VAR}` 解析）。
    pub spec: McpServerSpec,
    /// 来源层。
    pub source: McpConfigSource,
    /// 权限事实：调用前是否需要宿主审批。
    pub requires_approval: bool,
}

/// 单条配置/单个 server 的诊断（**绝不包含任何 env 值**）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpConfigDiagnostic {
    /// 出错条目；`None` 表示整份文件级问题。
    pub server: Option<String>,
    /// 机器可读码（`invalid_json` / `unsupported_schema_version` / `invalid_server_entry` /
    /// `literal_secret_forbidden` / `env_var_missing` / `denied_env_key` / `missing_command` …）。
    pub code: String,
    /// 面向用户/日志的说明（只含键名，不含值）。
    pub message: String,
}

impl McpConfigDiagnostic {
    fn new(server: Option<&str>, code: &str, message: impl Into<String>) -> Self {
        Self {
            server: server.map(str::to_string),
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// 一次配置加载的结果：可用条目 + 诊断（坏条目只禁用自己）。
#[derive(Clone, Debug, Default)]
pub struct McpConfigLoad {
    pub servers: Vec<McpServerEntry>,
    pub diagnostics: Vec<McpConfigDiagnostic>,
}

impl McpConfigLoad {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    pub fn server(&self, id: &str) -> Option<&McpServerEntry> {
        self.servers.iter().find(|entry| entry.id == id)
    }
}

/// 读取两层配置：`<workspace_root>/.mcp.json`（项目层）+ `$PONY_AGENT_HOME/mcp.json`（全局层）。
///
/// 任何一层读不到/坏掉都只产生诊断，不 panic、不阻断会话启动。
pub fn load_mcp_config(workspace_root: &Path) -> McpConfigLoad {
    let home = pony_agent_home();
    // 生产入口：工作区级配置默认**不可信**（供应链边界），只有显式 env 开关才采信。
    load_mcp_config_from(
        workspace_root,
        home.as_deref(),
        workspace_trust_from_env(),
    )
}

/// 生产入口的信任策略（`PONY_MCP_TRUST_WORKSPACE=1|true|yes|on` ⇒ `Trusted`，缺省 `Untrusted`）。
pub fn workspace_trust_from_env() -> WorkspaceMcpTrust {
    match std::env::var(MCP_TRUST_WORKSPACE_ENV) {
        Ok(raw)
            if matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ) =>
        {
            WorkspaceMcpTrust::Trusted
        }
        _ => WorkspaceMcpTrust::Untrusted,
    }
}

/// `load_mcp_config` 的测试 seam：全局层目录显式给出（`None` = 不读全局层，**不**读
/// `PONY_AGENT_HOME`）。
/// 读取两层配置，`trust` 显式决定是否加载**工作区级**层（唯一实现）。
///
/// - `WorkspaceMcpTrust::Untrusted`（缺省）：工作区文件存在时只产出
///   `workspace_mcp_config_untrusted` 诊断，不加载任何条目（防止仓库 `{"command":"/bin/sh"}`
///   以宿主权限执行）；全局 `$PONY_AGENT_HOME/mcp.json` 始终视为用户自建、可信。
/// - `WorkspaceMcpTrust::Trusted`：两层都加载，项目层覆盖全局层。
pub fn load_mcp_config_from(
    workspace_root: &Path,
    global_dir: Option<&Path>,
    trust: WorkspaceMcpTrust,
) -> McpConfigLoad {
    load_mcp_config_impl(workspace_root, global_dir, trust)
}

fn load_mcp_config_impl(
    workspace_root: &Path,
    global_dir: Option<&Path>,
    trust_workspace: WorkspaceMcpTrust,
) -> McpConfigLoad {
    let mut global = Vec::new();
    let mut diagnostics = Vec::new();

    let mut trust = BTreeMap::new();

    if let Some(global_dir) = global_dir {
        let path = global_dir.join(MCP_GLOBAL_CONFIG_FILE);
        let (entries, diags, flags) = read_config_file(&path, McpConfigSource::Global);
        global = entries;
        diagnostics.extend(diags);
        trust.extend(flags);
    }

    let path = workspace_root.join(MCP_PROJECT_CONFIG_FILE);
    let (workspace, diags, flags) = if trust_workspace == WorkspaceMcpTrust::Trusted {
        read_config_file(&path, McpConfigSource::Workspace)
    } else if path.exists() {
        // 文件存在但不被信任：明确留证（可读诊断），绝不静默忽略。
        (
            Vec::new(),
            vec![McpConfigDiagnostic::new(
                None,
                "workspace_mcp_config_untrusted",
                format!(
                    "workspace mcp config ignored (untrusted): `{}` 随仓库分发、内容不可信，\
                     默认不加载；确认信任后可设置 `{MCP_TRUST_WORKSPACE_ENV}=1` 启用。",
                    path.display()
                ),
            )],
            BTreeMap::new(),
        )
    } else {
        (Vec::new(), Vec::new(), BTreeMap::new())
    };
    diagnostics.extend(diags);
    // 项目层覆盖全局层（含信任决策）。
    trust.extend(flags);
    for (server_id, flag) in &trust {
        set_trust_read_only_hint(server_id, *flag);
    }

    // 项目层覆盖全局层（与 skills 同构：更局部的声明优先）。
    let mut merged: BTreeMap<String, McpServerEntry> = BTreeMap::new();
    for entry in global.into_iter().chain(workspace) {
        merged.insert(entry.id.clone(), entry);
    }

    McpConfigLoad {
        servers: merged.into_values().collect(),
        diagnostics,
    }
}

/// `$PONY_AGENT_HOME`；未设置时回退到 `config_dir/pony-agent`（与全局配置面同源的默认位置）。
pub fn pony_agent_home() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os(MCP_HOME_ENV) {
        let trimmed = raw.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"));
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.map(|dir| dir.join("pony-agent"))
}

fn read_config_file(
    path: &Path,
    source: McpConfigSource,
) -> (
    Vec<McpServerEntry>,
    Vec<McpConfigDiagnostic>,
    BTreeMap<String, bool>,
) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_mcp_config_detailed(&text, source),
        Err(err) => {
            // 缺文件是常态（没配 MCP 的用户），不算诊断噪声；只有"文件存在但读不了"才报。
            if err.kind() == std::io::ErrorKind::NotFound {
                (Vec::new(), Vec::new(), BTreeMap::new())
            } else {
                (
                    Vec::new(),
                    vec![McpConfigDiagnostic::new(
                        None,
                        "config_unreadable",
                        format!(
                            "读取 MCP 配置 `{}` 失败（{}）：{err}。",
                            path.display(),
                            source.as_str()
                        ),
                    )],
                    BTreeMap::new(),
                )
            }
        }
    }
}

/// 解析一份 MCP 配置文本。
///
/// 容忍：未知字段、`$schema`、`schemaVersion` 缺失；拒绝：整份非 JSON、未知 `schemaVersion`、
/// 单个 server 条目结构错误（**只禁用该条**）。
pub fn parse_mcp_config(
    text: &str,
    source: McpConfigSource,
) -> (Vec<McpServerEntry>, Vec<McpConfigDiagnostic>) {
    let (entries, diagnostics, _trust) = parse_mcp_config_detailed(text, source);
    (entries, diagnostics)
}

/// 解析一份 MCP 配置文本，并一并返回每个 server 的**本地信任决策**
/// （`trustReadOnlyHint`，见 [`trusts_read_only_hint`]）。
///
/// `parse_mcp_config` 保留 2 元组签名（既有调用方/验收契约），本函数是加载器内部形态。
pub fn parse_mcp_config_detailed(
    text: &str,
    source: McpConfigSource,
) -> (
    Vec<McpServerEntry>,
    Vec<McpConfigDiagnostic>,
    BTreeMap<String, bool>,
) {
    let mut diagnostics = Vec::new();
    let mut trust = BTreeMap::new();
    let root: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            diagnostics.push(McpConfigDiagnostic::new(
                None,
                "invalid_json",
                format!("MCP 配置（{}）不是合法 JSON：{err}。", source.as_str()),
            ));
            return (Vec::new(), diagnostics, trust);
        }
    };

    let Some(object) = root.as_object() else {
        diagnostics.push(McpConfigDiagnostic::new(
            None,
            "invalid_root",
            format!("MCP 配置（{}）根节点必须是对象。", source.as_str()),
        ));
        return (Vec::new(), diagnostics, trust);
    };

    if let Some(version) = object.get("schemaVersion").and_then(Value::as_str) {
        if version.trim() != MCP_CONFIG_SCHEMA_VERSION {
            diagnostics.push(McpConfigDiagnostic::new(
                None,
                "unsupported_schema_version",
                format!(
                    "MCP 配置 schemaVersion=`{version}` 不受支持（仅支持 `{MCP_CONFIG_SCHEMA_VERSION}`），已忽略该文件。"
                ),
            ));
            return (Vec::new(), diagnostics, trust);
        }
    }

    // 未知字段一律容忍；`mcpServers` 沿用生态写法。
    let Some(servers) = object.get("mcpServers") else {
        return (Vec::new(), diagnostics, trust);
    };
    let Some(servers) = servers.as_object() else {
        diagnostics.push(McpConfigDiagnostic::new(
            None,
            "invalid_mcp_servers",
            "MCP 配置的 `mcpServers` 必须是对象（server 名 → 配置）。",
        ));
        return (Vec::new(), diagnostics, trust);
    };

    let mut entries = Vec::new();
    for (id, raw) in servers {
        match parse_server_entry_full(id, raw, source) {
            Ok(Some((entry, trust_hint))) => {
                trust.insert(entry.id.clone(), trust_hint);
                entries.push(entry);
            }
            // `disabled: true`：静默跳过（用户显式关闭，不是错误）。
            Ok(None) => {}
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    entries.sort_by(|left, right| left.id.cmp(&right.id));
    (entries, diagnostics, trust)
}

fn parse_server_entry_full(
    id: &str,
    raw: &Value,
    source: McpConfigSource,
) -> Result<Option<(McpServerEntry, bool)>, McpConfigDiagnostic> {
    let id = id.trim();
    if id.is_empty() {
        return Err(McpConfigDiagnostic::new(
            Some(id),
            "invalid_server_entry",
            "MCP server 名不能为空。",
        ));
    }
    let Some(object) = raw.as_object() else {
        return Err(McpConfigDiagnostic::new(
            Some(id),
            "invalid_server_entry",
            format!("MCP server `{id}` 的配置必须是对象。"),
        ));
    };

    if object.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(None);
    }

    let Some(command) = object.get("command").and_then(Value::as_str) else {
        return Err(McpConfigDiagnostic::new(
            Some(id),
            "missing_command",
            format!("MCP server `{id}` 缺少必需的 `command` 字段，已禁用该 server。"),
        ));
    };
    if command.trim().is_empty() {
        return Err(McpConfigDiagnostic::new(
            Some(id),
            "missing_command",
            format!("MCP server `{id}` 的 `command` 不能为空，已禁用该 server。"),
        ));
    }

    let mut args = Vec::new();
    if let Some(raw_args) = object.get("args") {
        let Some(array) = raw_args.as_array() else {
            return Err(McpConfigDiagnostic::new(
                Some(id),
                "invalid_args",
                format!("MCP server `{id}` 的 `args` 必须是字符串数组，已禁用该 server。"),
            ));
        };
        for item in array {
            let Some(arg) = item.as_str() else {
                return Err(McpConfigDiagnostic::new(
                    Some(id),
                    "invalid_args",
                    format!("MCP server `{id}` 的 `args` 含非字符串元素，已禁用该 server。"),
                ));
            };
            args.push(arg.to_string());
        }
    }

    // L3-B-B：v1 只允许引用**显式命名空间**的环境变量（`PONY_MCP_*`），或该 server 在
    // `envAllowlist` 里逐个点名的变量；否则一份仓库配置就能把宿主凭据（OPENAI_API_KEY /
    // GITHUB_TOKEN …）送进第三方子进程。denylist 继续作为兜底。
    let mut env_allowlist: Vec<String> = Vec::new();
    if let Some(raw_allowlist) = object
        .get("envAllowlist")
        .or_else(|| object.get("env_allowlist"))
    {
        if let Some(array) = raw_allowlist.as_array() {
            env_allowlist.extend(
                array
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty()),
            );
        }
    }

    let mut env = Vec::new();
    if let Some(raw_env) = object.get("env") {
        let Some(map) = raw_env.as_object() else {
            return Err(McpConfigDiagnostic::new(
                Some(id),
                "invalid_env",
                format!("MCP server `{id}` 的 `env` 必须是对象（键 → `${{env:VAR}}`），已禁用该 server。"),
            ));
        };
        for (key, value) in map {
            let Some(raw_value) = value.as_str() else {
                return Err(McpConfigDiagnostic::new(
                    Some(id),
                    "invalid_env",
                    format!("MCP server `{id}` 的 env 键 `{key}` 必须是字符串，已禁用该 server。"),
                ));
            };
            // 凭据纪律（ADR 0066 decision 2）：只允许 `${env:VAR}` 间接引用。
            // 字面量值一律拒绝该条；诊断只报键名，绝不回显值。
            let Some(var_name) = env_placeholder_var(raw_value) else {
                return Err(McpConfigDiagnostic::new(
                    Some(id),
                    "literal_secret_forbidden",
                    format!(
                        "MCP server `{id}` 的 env 键 `{key}` 使用了字面量值；配置只允许 `${{env:VAR}}` 间接引用（避免凭据落盘），已禁用该 server。"
                    ),
                ));
            };
            let namespace_allowed = var_name.starts_with(MCP_ENV_NAMESPACE)
                || env_allowlist.iter().any(|allowed| allowed == &var_name);
            if !namespace_allowed {
                return Err(McpConfigDiagnostic::new(
                    Some(id),
                    "env_var_not_allowlisted",
                    format!(
                        "MCP server `{id}` 的 env 键 `{key}` 引用了未列入命名空间 `{MCP_ENV_NAMESPACE}*` \
                         的环境变量 `{var_name}`；若确需引用，请在该 server 的 `envAllowlist` 显式列出，\
                         已禁用该 server。"
                    ),
                ));
            }
            if is_denied_spec_env_key(&var_name) || is_denied_spec_env_key(key) {
                return Err(McpConfigDiagnostic::new(
                    Some(id),
                    "denied_env_key",
                    format!(
                        "MCP server `{id}` 的 env 键 `{key}` 属于进程/加载器劫持或凭据通道向量，已禁用该 server。"
                    ),
                ));
            }
            match std::env::var(&var_name) {
                Ok(resolved) if !resolved.is_empty() => env.push((key.clone(), resolved)),
                _ => {
                    return Err(McpConfigDiagnostic::new(
                        Some(id),
                        "env_var_missing",
                        format!(
                            "MCP server `{id}` 的 env 键 `{key}` 引用的环境变量 `{var_name}` 未设置，已禁用该 server。"
                        ),
                    ))
                }
            }
        }
    }

    // ADVISORY：同时接受 camelCase 与 snake_case（此前 snake_case 被静默忽略 ⇒ 默认零审批）。
    let requires_approval = object
        .get("requiresApproval")
        .or_else(|| object.get("requires_approval"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // B4：本地信任决策（默认 false = 不采信远端 readOnlyHint）。
    let trust_read_only_hint = object
        .get("trustReadOnlyHint")
        .or_else(|| object.get("trust_read_only_hint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(Some((McpServerEntry {
        id: id.to_string(),
        spec: McpServerSpec {
            id: id.to_string(),
            command: command.to_string(),
            args,
            env,
        },
        source,
        requires_approval,
    }, trust_read_only_hint)))
}

/// 解析 `${env:VAR}` 形式（严格：整串必须恰好是一个占位符）。
fn env_placeholder_var(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let rest = trimmed.strip_prefix("${env:")?;
    let name = rest.strip_suffix('}')?;
    if name.is_empty() || name.contains('}') || name.contains('$') {
        return None;
    }
    Some(name.to_string())
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 工具通道：McpToolProvider（impl ToolProvider）
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 一个已连接的 MCP server，作为 `ToolProvider`（ADR 0065 的唯一外部通道）。
///
/// 身份规则：`source_id = mcp:<server>`；`model_name = mcp__<server>__<tool>`；
/// `descriptor_id = mcp:<source_id>:<tool>`（registry 对 Mcp 来源要求
/// `mcp:<provenance.source_id>:` 前缀，故 source_id 自带 `mcp:` 时该串会出现两个 `mcp:` 段）。
/// **模型可见面只认 `model_name`**，`descriptor_id` 是实现内部标识。
pub struct McpToolProvider {
    source_id: String,
    server_id: String,
    server_name: String,
    requires_approval: bool,
    client: Option<McpClient>,
    tools: Vec<McpToolSpec>,
    /// descriptor_id → tool 名（`tools/call` 用它，避免按名字猜）。
    by_descriptor_id: BTreeMap<String, String>,
}

impl McpToolProvider {
    /// 连接一个 server：spawn + `initialize` 握手 + `tools/list`。
    ///
    /// 失败一律以 `<分源错误码>: <说明>` 返回（调用方可直接透出或记日志）。
    pub fn connect(entry: &McpServerEntry, timeout: Duration) -> Result<Self, String> {
        let timeout = timeout.max(Duration::from_millis(1));
        let client = McpClient::connect_with_timeouts(
            &entry.spec,
            timeout.max(DEFAULT_CONNECT_TIMEOUT.min(timeout)),
            timeout,
        )
        .map_err(|err| provider_error_string(&err))?;
        let tools = client
            .list_tools()
            .map_err(|err| provider_error_string(&err))?;

        let source_id = source_id_for(&entry.id);
        let mut by_descriptor_id = BTreeMap::new();
        for tool in &tools {
            by_descriptor_id.insert(descriptor_id_for(&source_id, &tool.name), tool.name.clone());
        }

        let server_name = client.server_name().to_string();
        Ok(Self {
            source_id,
            server_id: entry.id.clone(),
            server_name,
            requires_approval: entry.requires_approval,
            client: Some(client),
            tools,
            by_descriptor_id,
        })
    }

    /// server 名（`mcpServers` 的键）。
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    /// `initialize` 握手得到的 server 名称（可为空）。
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// 已发现的工具（按 `tools/list` 顺序）。
    pub fn tools(&self) -> &[McpToolSpec] {
        &self.tools
    }

    /// 非阻塞探测子进程是否已退出（崩溃诊断）。
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        let Some(client) = self.client.as_mut() else {
            return Err(format!(
                "{}: MCP server `{}` 会话已关闭。",
                codes::SERVER_UNAVAILABLE,
                self.server_id
            ));
        };
        client
            .try_wait()
            .map_err(|err| format!("{}: 探测 MCP server `{}` 进程失败：{err}。", codes::SERVER_UNAVAILABLE, self.server_id))
    }

    /// 关闭会话并回收子进程（幂等）。
    pub fn shutdown(mut self) -> Result<(), String> {
        // 子进程回收在 `McpClient::drop`（关 stdin → 宽限 → 进程组 SIGKILL → wait）。
        self.client.take();
        Ok(())
    }

    /// 资源面 v1 未接线：**结构性 fail-closed**，但错误码可读（ADR 0066 decision 6）。
    pub fn read_resource(&self, uri: &str) -> Result<Value, String> {
        let _ = uri;
        Err(crate::agent::mcp_resources::resources_not_wired_error(
            &self.source_id,
        ))
    }

    /// 该 server 的运行健康快照（`status` 只有 connected / failed 两种；
    /// 未连接的 server 由 [`McpToolChannel`] 以 `disabled`/`failed` 记录）。
    pub fn health(&self) -> McpServerHealth {
        McpServerHealth {
            server_id: self.server_id.clone(),
            source_id: self.source_id.clone(),
            status: "connected",
            error_code: None,
            tool_count: self.tools.len(),
            server_name: Some(self.server_name.clone()),
        }
    }

    fn call_tool(&self, tool: &str, arguments: Value) -> Result<Value, ToolProviderError> {
        // ADVISORY（取消透传）：桥接层经线程局部把本轮取消令牌传进来；已取消则不再发起调用，
        // 避免"停止后仍在等一个 5s 的 stdio 往返"。
        if crate::agent::tool_runtime::current_provider_cancellation()
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            return Err(ToolProviderError::new(
                "tool_cancelled",
                format!("MCP server `{}` 的调用在发起前已取消。", self.server_id),
            ));
        }
        let Some(client) = self.client.as_ref() else {
            return Err(ToolProviderError::new(
                codes::SERVER_UNAVAILABLE,
                format!("MCP server `{}` 会话已关闭。", self.server_id),
            ));
        };
        client.call_tool(tool, arguments).map_err(|err| {
            ToolProviderError::new(err.wire_code(), redact_secrets(err.message()))
        })
    }

    fn descriptor_for(&self, tool: &McpToolSpec) -> ToolDescriptor {
        let model_name = model_name_for(&self.server_id, &tool.name);
        let descriptor_id = descriptor_id_for(&self.source_id, &tool.name);
        let mut scopes = std::collections::BTreeSet::new();
        // L3-B4：**不信任远端自报的 readOnlyHint**。只有本地配置显式 opt-in 时才采信它；
        // 否则一律按可写（`WorkspaceWrite`）声明 —— read_only 会话因此不会被"远端自称只读"
        // 的破坏性工具绕过。
        let trusted_read_only = trusts_read_only_hint(&self.server_id) && tool.is_read_only();
        scopes.insert(if trusted_read_only {
            ToolPermissionScope::WorkspaceRead
        } else {
            ToolPermissionScope::WorkspaceWrite
        });
        ToolDescriptor {
            identity: ToolIdentity {
                descriptor_id,
                model_name: model_name.clone(),
                canonical_name: model_name.clone(),
                primitive_name: model_name.clone(),
                source: ToolDescriptorSource::Mcp,
            },
            // 模型可见名必须登记为 alias：registry 按 alias 解析模型发来的名字。
            aliases: vec![model_name],
            description: tool.description.clone().unwrap_or_default(),
            input_schema: tool.input_schema.clone(),
            kind: ToolKind::External,
            exposure: ToolExposure::ModelVisible,
            permission_declaration: ToolPermissionDeclaration {
                scopes,
                requires_approval: self.requires_approval,
                host_mediated: false,
                approval_mode: None,
            },
            execution_policy: ToolExecutionPolicy::default(),
            display_metadata: ToolDisplayMetadata::default(),
            handler_provenance: ToolHandlerProvenance {
                handler_kind: "mcp_stdio".to_string(),
                source_id: self.source_id.clone(),
            },
            source_revision: format!("{}-tools-v1", self.server_id),
            composed_descriptor_ids: Vec::new(),
        }
    }
}

impl ToolProvider for McpToolProvider {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn list_tools(&self) -> Vec<ToolDescriptor> {
        self.tools
            .iter()
            .map(|tool| self.descriptor_for(tool))
            .collect()
    }

    fn call(&self, descriptor_id: &str, arguments: Value) -> Result<Value, ToolProviderError> {
        let Some(tool) = self.by_descriptor_id.get(descriptor_id) else {
            return Err(ToolProviderError::new(
                codes::TOOL_NOT_FOUND,
                format!(
                    "MCP server `{}` 没有工具描述符 `{descriptor_id}`（未知工具）。",
                    self.server_id
                ),
            ));
        };
        self.call_tool(tool, arguments)
    }
}

impl std::fmt::Debug for McpToolProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpToolProvider")
            .field("source_id", &self.source_id)
            .field("server_id", &self.server_id)
            .field("tools", &self.tools.len())
            .finish()
    }
}

/// `source_id`：`mcp:<server>`。
pub fn source_id_for(server: &str) -> String {
    format!("mcp:{server}")
}

/// 模型可见名：`mcp__<server>__<tool>`。
pub fn model_name_for(server: &str, tool: &str) -> String {
    format!("{MCP_TOOL_NAME_PREFIX}{server}__{tool}")
}

/// 内部稳定标识：`mcp:<source_id>:<tool>`（registry 的 Mcp 命名空间校验要求此前缀）。
pub fn descriptor_id_for(source_id: &str, tool: &str) -> String {
    format!("mcp:{source_id}:{tool}")
}

/// 单个 server 的运行健康（`status ∈ {connected, disabled, failed}`，v1 无中间态）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerHealth {
    pub server_id: String,
    pub source_id: String,
    pub status: &'static str,
    pub error_code: Option<String>,
    pub tool_count: usize,
    pub server_name: Option<String>,
}

impl McpServerHealth {
    pub fn disabled(entry: &McpServerEntry) -> Self {
        Self {
            server_id: entry.id.clone(),
            source_id: source_id_for(&entry.id),
            status: "disabled",
            error_code: None,
            tool_count: 0,
            server_name: None,
        }
    }

    pub fn failed(entry: &McpServerEntry, error: &str) -> Self {
        Self {
            server_id: entry.id.clone(),
            source_id: source_id_for(&entry.id),
            status: "failed",
            error_code: Some(error_code_of(error)),
            tool_count: 0,
            server_name: None,
        }
    }
}

/// 从 `<code>: <message>` 形态里取出错误码（无码时 `handler_error`）。
fn error_code_of(message: &str) -> String {
    message
        .split_once(':')
        .map(|(code, _)| code.trim())
        .filter(|code| {
            !code.is_empty()
                && code
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .unwrap_or("handler_error")
        .to_string()
}

fn provider_error_string(err: &McpClientError) -> String {
    format!("{}: {}", err.wire_code(), redact_secrets(err.message()))
}

/// 日志/诊断脱敏：剥掉 URL 内嵌 userinfo，并遮蔽形如 `key=value` / `Bearer xxx` 的凭据片段。
///
/// 本模块自身**从不**把 env 值写进任何字符串；这里的兜底是为了让第三方 server 在错误信息里
/// 回显的凭据不会顺流进入日志（ADR 0066 decision 2）。
pub fn redact_secrets(text: &str) -> String {
    let mut redacted = String::with_capacity(text.len());
    for token in text.split_inclusive([' ', '\n', '\t', ',', ';']) {
        let trimmed = token.trim_end_matches([' ', '\n', '\t', ',', ';']);
        let suffix = &token[trimmed.len()..];
        if let Some(rest) = trimmed.strip_prefix("http://").or_else(|| trimmed.strip_prefix("https://"))
        {
            if rest.contains('@') {
                let scheme_len = trimmed.len() - rest.len();
                let authority_end = rest.find('/').unwrap_or(rest.len());
                let authority = &rest[..authority_end];
                if let Some(at) = authority.rfind('@') {
                    redacted.push_str(&trimmed[..scheme_len]);
                    redacted.push_str(&authority[at + 1..]);
                    redacted.push_str(&rest[authority_end..]);
                    redacted.push_str(suffix);
                    continue;
                }
            }
        }
        let lower = trimmed.to_ascii_lowercase();
        let looks_secret = lower.contains("bearer ")
            || lower.contains("authorization:")
            || lower.starts_with("sk-")
            || trimmed.contains("sk-")
            || lower.starts_with("token=")
            || lower.starts_with("api_key=")
            || lower.starts_with("apikey=")
            || lower.starts_with("password=")
            || lower.starts_with("secret=");
        if looks_secret {
            redacted.push_str("[redacted]");
            redacted.push_str(suffix);
            continue;
        }
        redacted.push_str(token);
    }
    redacted
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 通道：连接、注册、健康、工具面
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 一组 MCP server 的受治理通道。
#[derive(Default)]
pub struct McpToolChannel {
    providers: Vec<Arc<McpToolProvider>>,
    /// 连接失败的 server（fail-soft：不影响其它 server 与会话启动）。
    failures: Vec<McpServerHealth>,
    /// 被显式禁用/未连接的 server。
    disabled: Vec<McpServerHealth>,
}

impl McpToolChannel {
    /// 按配置连接所有 server。单条失败只记录，不影响其它 server。
    pub fn connect(load: &McpConfigLoad, timeout: Duration) -> Self {
        let mut providers = Vec::new();
        let mut failures = Vec::new();
        for entry in &load.servers {
            match McpToolProvider::connect(entry, timeout) {
                Ok(provider) => providers.push(Arc::new(provider)),
                Err(error) => failures.push(McpServerHealth::failed(entry, &error)),
            }
        }
        Self {
            providers,
            failures,
            disabled: Vec::new(),
        }
    }

    /// 使用默认请求预算连接。
    pub fn connect_default(load: &McpConfigLoad) -> Self {
        Self::connect(load, DEFAULT_TIMEOUT)
    }

    /// 已连接 provider 的快照（供宿主注册/清理）。
    pub fn providers(&self) -> &[Arc<McpToolProvider>] {
        &self.providers
    }

    /// 运行健康：已连接 + 失败 + 禁用。
    pub fn server_health(&self) -> Vec<McpServerHealth> {
        let mut health = self
            .providers
            .iter()
            .map(|provider| provider.health())
            .collect::<Vec<_>>();
        health.extend(self.failures.iter().cloned());
        health.extend(self.disabled.iter().cloned());
        health.sort_by(|left, right| left.server_id.cmp(&right.server_id));
        health
    }

    /// 把每个 provider 注册进治理分派器（`register_tool_provider`，追加语义）。
    ///
    /// 单条注册失败只记录健康状态（fail-soft），不影响其它 server。
    pub fn register_tool_providers(
        &mut self,
        dispatcher: &crate::agent::dispatcher::GovernedDispatcher,
    ) -> Vec<McpServerHealth> {
        let mut remaining = Vec::new();
        for provider in self.providers.drain(..) {
            match dispatcher.register_tool_provider(provider.clone()) {
                Ok(()) => remaining.push(provider),
                Err(error) => {
                    let health = McpServerHealth {
                        server_id: provider.server_id().to_string(),
                        source_id: provider.source_id().to_string(),
                        status: "failed",
                        error_code: Some(error_code_of(&error)),
                        tool_count: 0,
                        server_name: None,
                    };
                    self.failures.push(health);
                }
            }
        }
        self.providers = remaining;
        self.server_health()
    }

    /// 外部工具定义（稳定序 `(source_id, model_name)`），供模型可见工具面使用。
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = Vec::new();
        for provider in &self.providers {
            for tool in provider.tools() {
                let model_name = model_name_for(provider.server_id(), &tool.name);
                defs.push(ToolDefinition {
                    name: intern(&model_name),
                    description: intern(tool.description.as_deref().unwrap_or("")),
                    input_schema: tool.input_schema.clone(),
                });
            }
        }
        defs.sort_by(|left, right| left.name.cmp(right.name));
        defs
    }

    /// 关闭所有连接并回收子进程。
    pub fn shutdown(self) {
        for provider in self.providers {
            if let Ok(provider) = Arc::try_unwrap(provider) {
                let _ = provider.shutdown();
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 进程级通道槽（host 命令与模型工具面的单一入口）
// ─────────────────────────────────────────────────────────────────────────────────────────────

fn channel_slot() -> &'static Mutex<Option<Arc<McpToolChannel>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<McpToolChannel>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn tool_surface_cache() -> &'static Mutex<Option<Vec<ToolDefinition>>> {
    static CACHE: OnceLock<Mutex<Option<Vec<ToolDefinition>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// 安装（替换）进程级 MCP 通道。替换会清空工具面缓存 ⇒ 新工具在**下一轮**生效，
/// 轮次内工具面冻结（ADR 0065 decision 8）。
pub fn install_mcp_tool_channel(channel: Arc<McpToolChannel>) {
    if let Ok(mut slot) = channel_slot().lock() {
        *slot = Some(channel);
    }
    invalidate_tool_surface_cache();
}

/// 当前进程级 MCP 通道。
pub fn installed_mcp_tool_channel() -> Option<Arc<McpToolChannel>> {
    channel_slot().lock().ok().and_then(|slot| slot.clone())
}

/// 关闭并卸载进程级通道。
pub fn shutdown_mcp_tool_channel() {
    let channel = channel_slot()
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(channel) = channel {
        if let Ok(channel) = Arc::try_unwrap(channel) {
            channel.shutdown();
        }
    }
    invalidate_tool_surface_cache();
}

/// 已安装通道的健康快照（未安装 = 空）。
pub fn mcp_health() -> Vec<McpServerHealth> {
    installed_mcp_tool_channel()
        .map(|channel| channel.server_health())
        .unwrap_or_default()
}

fn invalidate_tool_surface_cache() {
    if let Ok(mut cache) = tool_surface_cache().lock() {
        *cache = None;
    }
}

/// 模型可见工具面：内建 45 项 + 已接入的外部工具（稳定排序，会话内冻结）。
///
/// 契约（ADR 0065 decision 8 / T2）：
/// - 外部工具按 `(source_id, model_name)` 稳定排序**追加在内建之后**，mid-turn 不得变化；
/// - MCP 未配置/连接失败时返回**内建面**，不 panic、不阻断会话启动（fail-soft）；
/// - 结果在进程内缓存，直到通道被替换（`install_mcp_tool_channel`）才失效。
pub fn surface_tool_definitions() -> Vec<ToolDefinition> {
    if let Ok(cache) = tool_surface_cache().lock() {
        if let Some(cached) = cache.as_ref() {
            return cached.clone();
        }
    }

    let mut definitions = crate::agent::tools::builtin_tools();
    if let Some(channel) = installed_mcp_tool_channel() {
        definitions.extend(channel.tool_definitions());
    }

    if let Ok(mut cache) = tool_surface_cache().lock() {
        *cache = Some(definitions.clone());
    }
    definitions
}

/// 动态工具名/描述 intern：`ToolDefinition.name/description` 是 `&'static str`。
///
/// 泄漏量 = 发现到的 MCP 工具名/描述数量（进程生命周期内一次性），与工具数同阶；同一字符串
/// 只泄漏一次（进程级表去重）。
fn intern(value: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<BTreeMap<String, &'static str>>> = OnceLock::new();
    let table = INTERNED.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut table = match table.lock() {
        Ok(table) => table,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(existing) = table.get(value) {
        return existing;
    }
    let leaked: &'static str = Box::leak(value.to_string().into_boxed_str());
    table.insert(value.to_string(), leaked);
    leaked
}

/// 按配置加载并连接（host 入口用）：返回 (配置结果, 通道)。
pub fn load_and_connect(workspace_root: &Path, timeout: Duration) -> (McpConfigLoad, McpToolChannel) {
    let load = load_mcp_config(workspace_root);
    let channel = McpToolChannel::connect(&load, timeout);
    (load, channel)
}
// ─────────────────────────────────────────────────────────────────────────────────────────────
// MCP 宿主 DTO（camelCase；只含脱敏后的字段，**不含 env 值**）
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 单个 MCP server 的配置视图（脱敏：只暴露命令名、参数个数与 env 键名）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfigView {
    pub server_id: String,
    pub source: String,
    pub command: String,
    pub arg_count: usize,
    pub env_keys: Vec<String>,
    pub requires_approval: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpConfigDiagnosticView {
    pub server: Option<String>,
    pub code: String,
    pub message: String,
}

/// 运行健康视图：`status ∈ {connected, disabled, failed}`（v1 无中间态）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpHealthView {
    pub server_id: String,
    pub source_id: String,
    pub status: String,
    pub error_code: Option<String>,
    pub tool_count: usize,
    pub server_name: Option<String>,
}

impl From<crate::agent::mcp_config::McpServerHealth> for McpHealthView {
    fn from(health: crate::agent::mcp_config::McpServerHealth) -> Self {
        Self {
            server_id: health.server_id,
            source_id: health.source_id,
            status: health.status.to_string(),
            error_code: health.error_code,
            tool_count: health.tool_count,
            server_name: health.server_name,
        }
    }
}

/// `load_mcp_sources` 的返回：配置事实 + 诊断 + 运行健康。
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpSourceLoadSummary {
    pub workspace_root: String,
    pub servers: Vec<McpServerConfigView>,
    pub diagnostics: Vec<McpConfigDiagnosticView>,
    pub health: Vec<McpHealthView>,
}
