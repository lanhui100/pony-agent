//! ACP 外部 agent 清单（`AcpAgentSpec`）与存储契约。
//!
//! 存储位置：`~/.config/pony-agent/acp-agents.json` —— 与 `providers.json` **同级但独立**，
//! 绝不改动 provider 配置 schema（真源 `docs/architecture/acp-client-integration.md` §4 P0 表）。
//!
//! 本文件 P0 只冻结类型 / 常量 / 函数签名；加载、写入、回退逻辑一律 `unimplemented!()`，
//! 由 P1 Executor 实现。内置两条默认 agent 是**数据契约**（不依赖任何 I/O）。

// P0 空桩：函数体统一 `unimplemented!()`，形参「未使用」属预期；P1 实现时移除本允许。
#![allow(unused_variables)]

use std::path::PathBuf;

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
        unimplemented!("P1：错误码映射（acp_config_io / acp_config_parse / acp_config_invalid / acp_config_agent_not_found）")
    }
}

/// 清单文件完整路径（`<config_dir>/pony-agent/acp-agents.json`）。
///
/// 必须是**绝对路径**；目录不存在时由加载方创建（P1 实现）。
pub fn agents_file_path() -> PathBuf {
    unimplemented!("P1：dirs::config_dir() + ACP_CONFIG_DIR_NAME + ACP_AGENTS_FILE_NAME")
}

/// 读取用户清单；文件不存在时返回内置默认（P1 实现）。
pub fn load_agents() -> Result<Vec<AcpAgentSpec>, AcpConfigError> {
    unimplemented!("P1：读文件 → parse → 不存在则 builtin_agent_specs()")
}

/// 原子写回用户清单（P1 实现：`*.tmp` + rename）。
pub fn save_agents(agents: &[AcpAgentSpec]) -> Result<(), AcpConfigError> {
    unimplemented!("P1：序列化 + 原子写")
}

/// 把 [`BUILTIN_AGENTS`] 物化为 [`AcpAgentSpec`]（P1 实现）。
pub fn builtin_agent_specs() -> Vec<AcpAgentSpec> {
    unimplemented!("P1：BUILTIN_AGENTS → Vec<AcpAgentSpec>")
}

/// 按 id 解析一条规格（先用户清单，后内置默认；P1 实现）。
pub fn resolve_agent(id: &str) -> Result<AcpAgentSpec, AcpConfigError> {
    unimplemented!("P1：load_agents() 中按 id 查找，未命中回退 builtin")
}
