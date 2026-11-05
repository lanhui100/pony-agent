//! 进程外插件宿主（契约 `pony-plugin-v1`，ADR 0067 / L2 扩展层）。
//!
//! # 形状
//!
//! ```text
//! manifest.rs   插件契约：解析 + 校验（未知字段/未知能力/版本不符一律拒绝）
//! capability.rs 隔离面：env_clear + 白名单（F8a 单一真值）、cwd 限定、能力协商
//! transport.rs  stdio JSON-RPC 2.0 传输（按行分隔，与 MCP / hooks 同构）
//! supervisor.rs 生命周期 spawn → initialize → 注册 → call → shutdown、超时杀进程组、fail-closed
//! ```
//!
//! # 生命周期与失败语义
//!
//! 1. [`PluginSupervisor::spawn`]：校验 manifest 与可执行文件 → `env_clear()` + 白名单 →
//!    cwd 限定会话工作区 → Unix 独立进程组；
//! 2. [`PluginSupervisor::initialize`]：`apiVersion` 协商 + 能力协商（声明外一律拒绝）→
//!    注册（`tools/list` ∩ manifest 白名单）；
//! 3. [`PluginSupervisor::call_tool`] / [`PluginSupervisor::call_tool_with_deadline`]：单次调用
//!    deadline 由调用方给定（缺省 [`PluginHostConfig::timeout_ms`]）；超时 SIGKILL 进程组并
//!    同步回收；
//! 4. [`PluginSupervisor::shutdown`]：`shutdown` → 关 stdin → 宽限 → 杀进程组；`Drop` 同样
//!    回收（宿主 drop 时不残留插件进程）。
//!
//! **崩溃 fail-closed**：插件崩溃/自行退出后进入不可用状态，本轮与后续调用都返回
//! `plugin_unavailable`，宿主**不重启、不重试**（避免重试风暴）；恢复 = 调用方重建 supervisor。
//!
//! # 对外消费面（ADR 0065 的**唯一**外部工具通道）
//!
//! [`PluginHostProvider`] 是插件宿主对外的唯一消费面，实现
//! `crate::agent::tools::ToolProvider`（`source_id` / `list_tools` / `call`）：
//! - `source_id = plugin:<name>`，模型可见名 `plugin__<name>__<tool>`
//!   （[`plugin_tool_model_name`]）；
//! - `descriptor_id = dynamic:plugin:<name>:<tool>`：registry 目前只接受
//!   `builtin:/mcp:/skill:/dynamic:` 四类前缀（`tools.rs::validate_descriptor_identity`），
//!   因此插件来源暂挂 `ToolDescriptorSource::Dynamic`；T1 若新增 `Plugin` 变体，只需改
//!   [`plugin_tool_descriptor_id`] 与前缀映射，注册面契约不变；
//! - 失败经 `ToolProviderError` 保留来源码家族（`plugin_unavailable` / `plugin_timeout` /
//!   `plugin_tool_error` / `plugin_unknown_tool` / `plugin_capability_denied` / …），
//!   绝不塌缩成 `handler_error`；
//! - 带调用方 deadline 的形态（ADR 0067 决策 3）是**内部** trait [`PluginCall`]，不对外：
//!   外部能力必须经 `dispatcher.register_tool_provider` 进治理管线，不允许第二条通道。
//!
//! # v1 未消费的 manifest 字段（能力模型）
//!
//! `PluginManifest` 固定 6 个 pub 字段（`apiVersion` / `name` / `version` / `capabilities` /
//! `tools` / `hookPoints`）。其中 **`capabilities` 的信任/授权语义在 v1 未消费**：
//! 宿主只做"已知能力且双向声明"的协商收敛（`tools` 授予、`hooks` 在 v1 拒绝），
//! 不据能力发放文件系统根、命令执行或网络授权（ADR 0067 决策 3 的 `fsRoots`/`exec`/`network`
//! 声明面未落地，见 [`manifest::PluginManifest`] 的偏差说明）。这些字段的**消费方在 T2/T3
//! 后续波次**（信任面 UI 与授权记录属 T5），因此现在既不得据其放行，也不得据此声称已授权。

//! # 平台局限
//!
//! Windows 无进程组：终止只能杀直接子进程，插件派生的孙进程为"尽力回收"（彻底回收需 Job
//! Object，见 `agent/process/windows_job.rs`，其可见性收紧为 `pub(super)`，不在本批改动）。

pub mod capability;
pub mod manifest;
pub mod supervisor;
pub mod transport;

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::agent::tools::{
    ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy, ToolExposure,
    ToolHandlerProvenance, ToolIdentity, ToolKind, ToolPermissionDeclaration,
};

pub use capability::ENV_ALLOWLIST;
pub use manifest::{PluginManifest, PluginToolDecl, PLUGIN_API_VERSION};
pub use supervisor::{
    clear_plugin_audit_events, plugin_audit_events, PluginAuditEvent, PluginHostConfig,
    PluginSupervisor, PluginToolRegistration, DEFAULT_PLUGIN_TIMEOUT_MS,
};

/// 插件工具在模型可见面上的命名空间前缀。
pub const PLUGIN_TOOL_NAMESPACE: &str = "plugin";

/// 插件工具的模型可见名：`plugin__<name>__<tool>`。
///
/// 命名规则是稳定契约的一部分（ADR 0064「前缀稳定」）：插件名/工具名在会话内冻结，
/// 拼接结果必须与 [`parse_plugin_tool_model_name`] 可逆。
pub fn plugin_tool_model_name(plugin_name: &str, tool_name: &str) -> String {
    format!("{PLUGIN_TOOL_NAMESPACE}__{plugin_name}__{tool_name}")
}

/// 反解插件工具名（非法/非插件命名返回 `None`）。
pub fn parse_plugin_tool_model_name(model_name: &str) -> Option<(&str, &str)> {
    let rest = model_name.strip_prefix("plugin__")?;
    let (plugin, tool) = rest.split_once("__")?;
    if plugin.is_empty() || tool.is_empty() || tool.contains("__") {
        return None;
    }
    Some((plugin, tool))
}

/// 插件宿主的稳定错误分类。
///
/// 每个变体的 [`PluginHostError::code`] 是后续 T4 conformance kit 与错误码注册表
/// （`agent/error_code.rs`）的输入；错误码前缀 `plugin_*` 保证来源可区分
/// （ADR 0064 不变量 3「失败闭合且错误码能区分来源」）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginHostError {
    /// 进程启动失败。
    Spawn(String),
    /// manifest 契约不合规。
    Manifest(String),
    /// 握手（initialize / 注册）失败。
    Initialize(String),
    /// 单次调用超时（已杀进程组）。
    Timeout(String),
    /// 协议层错误（非法 JSON、id 不匹配、缺 result）。
    Protocol(String),
    /// 插件明确返回 `error`。
    Server(String),
    /// 会话不可用（管道关闭 / 已 fail-closed）。
    Unavailable(String),
    /// 能力未授予。
    CapabilityDenied(String),
}

impl PluginHostError {
    /// 稳定的机器可读错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Spawn(_) => "plugin_spawn_failed",
            Self::Manifest(_) => "plugin_manifest_invalid",
            Self::Initialize(_) => "plugin_initialize_failed",
            Self::Timeout(_) => "plugin_timeout",
            Self::Protocol(_) => "plugin_protocol_error",
            Self::Server(_) => "plugin_tool_error",
            Self::Unavailable(_) => "plugin_unavailable",
            Self::CapabilityDenied(_) => "plugin_capability_denied",
        }
    }

    /// 该错误是否意味着"进程级不可用"（需要立即终止并进入 fail-closed）。
    ///
    /// 区分依据：超时/管道关闭/协议错乱说明会话已不可信；插件显式 `error` 响应
    /// （[`PluginHostError::Server`]）只是单次工具失败，会话仍可继续。
    pub fn is_transport_level(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::Protocol(_) | Self::Unavailable(_) | Self::Spawn(_)
        )
    }

    /// 面向模型/日志的可读说明。
    pub fn message(&self) -> &str {
        match self {
            Self::Spawn(msg)
            | Self::Manifest(msg)
            | Self::Initialize(msg)
            | Self::Timeout(msg)
            | Self::Protocol(msg)
            | Self::Server(msg)
            | Self::Unavailable(msg)
            | Self::CapabilityDenied(msg) => msg,
        }
    }
}

impl fmt::Display for PluginHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for PluginHostError {}

/// 插件调用的**内部**接口：带调用方 deadline 的那一套（`call(&mut self, tool, args, deadline)`）。
///
/// 为什么是内部 trait 而不是对外 trait：ADR 0065「只允许一条外部通道」——外部能力只能经
/// `crate::agent::tools::ToolProvider` 进治理分派管线。带 deadline 的形态是本模块的实现细节
/// （ADR 0067 决策 3 的"单次调用 deadline 由调用方给定"），不构成第二条通道。
/// 对外消费面见 [`PluginHostProvider`]。
pub(crate) trait PluginCall {
    /// 本插件当前注册的工具（initialize 前回落到 manifest 声明）。
    fn plugin_tools(&self) -> Vec<PluginToolRegistration>;

    /// 调用插件工具，deadline 由调用方给定。
    fn plugin_call(
        &self,
        tool_name: &str,
        arguments: Value,
        deadline: Duration,
    ) -> Result<Value, PluginHostError>;
}

/// 插件宿主对外的唯一消费面：`impl crate::agent::tools::ToolProvider`。
///
/// 用法（`dispatcher.register_tool_provider(Arc::new(provider))`）：
///
/// ```no_run
/// # use std::sync::Arc;
/// # use pony_agent_core::agent::plugin_host::{PluginHostConfig, PluginHostProvider, PluginManifest, PluginSupervisor};
/// # fn demo(manifest: PluginManifest, command: std::path::PathBuf) -> Result<(), String> {
/// let mut supervisor = PluginSupervisor::spawn(manifest, command, PluginHostConfig::default())?;
/// supervisor.initialize()?;                  // 注册面在 initialize 后才有工具
/// let provider = Arc::new(PluginHostProvider::new(supervisor));
/// // dispatcher.register_tool_provider(provider)?;  // 治理管线唯一入口（ADR 0065）
/// # let _ = provider;
/// # Ok(())
/// # }
/// ```
///
/// `supervisor` 用 `Mutex` 包裹以满足 `ToolProvider: Send + Sync` 且 `call(&self, ..)`：
/// 同一插件的调用天然串行（stdio 一问一答），与 JSON-RPC 传输层语义一致。
pub struct PluginHostProvider {
    supervisor: std::sync::Mutex<PluginSupervisor>,
    source_id: String,
    plugin_name: String,
}

impl PluginHostProvider {
    /// 用插件自身声明的名字构造来源身份（`plugin:<name>`，单一真值来自 manifest）。
    pub fn new(supervisor: PluginSupervisor) -> Self {
        let plugin_name = supervisor.plugin_name().to_string();
        let source_id = format!("{PLUGIN_TOOL_NAMESPACE}:{plugin_name}");
        Self {
            supervisor: std::sync::Mutex::new(supervisor),
            source_id,
            plugin_name,
        }
    }

    /// 插件名（manifest 声明）。
    pub fn plugin_name(&self) -> &str {
        &self.plugin_name
    }

    /// manifest 内容指纹（信任面 / 审计用）。
    pub fn manifest_fingerprint(&self) -> String {
        self.supervisor
            .lock()
            .map(|supervisor| supervisor.manifest_fingerprint().to_string())
            .unwrap_or_default()
    }

    /// 生命周期协作：在宿主侧显式 initialize（`new` 之后、注册之前）。
    pub fn initialize(&self) -> Result<(), String> {
        self.supervisor
            .lock()
            .map_err(|_| "plugin_unavailable: provider 锁已中毒。".to_string())?
            .initialize()
    }

    /// 生命周期协作：优雅关闭（消费 provider；调用后不可再 call）。
    pub fn shutdown(self) -> Result<(), String> {
        let supervisor = self
            .supervisor
            .into_inner()
            .map_err(|_| "plugin_unavailable: provider 锁已中毒。".to_string())?;
        supervisor.shutdown()
    }

    /// 带调用方 deadline 的调用（ADR 0067 决策 3）；对外 `ToolProvider::call` 用配置超时。
    pub fn call_with_deadline(
        &self,
        tool_name: &str,
        arguments: Value,
        deadline: Duration,
    ) -> Result<Value, String> {
        self.plugin_call(tool_name, arguments, deadline)
            .map_err(|err| err.to_string())
    }
}

impl PluginCall for PluginHostProvider {
    fn plugin_tools(&self) -> Vec<PluginToolRegistration> {
        self.supervisor
            .lock()
            .map(|supervisor| {
                let registered = supervisor.tools().to_vec();
                if registered.is_empty() {
                    // initialize 之前：回落到 manifest 声明（注册面仍受 manifest 白名单约束）。
                    supervisor.declared_tools()
                } else {
                    registered
                }
            })
            .unwrap_or_default()
    }

    fn plugin_call(
        &self,
        tool_name: &str,
        arguments: Value,
        deadline: Duration,
    ) -> Result<Value, PluginHostError> {
        let mut supervisor = self.supervisor.lock().map_err(|_| {
            PluginHostError::Unavailable("provider 锁已中毒。".to_string())
        })?;
        supervisor
            .call_tool_with_deadline(tool_name, arguments, deadline)
            .map_err(classify_plugin_failure)
    }
}

impl crate::agent::tools::ToolProvider for PluginHostProvider {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    /// 外部工具 descriptor：模型可见名 `plugin__<name>__<tool>`，`descriptor_id` 采用 registry
    /// 当前唯一支持的动态来源前缀 `dynamic:plugin:<name>:<tool>`（T1 尚无 Plugin 变体）。
    fn list_tools(&self) -> Vec<ToolDescriptor> {
        let fingerprint = self.manifest_fingerprint();
        plugin_tool_descriptors(&self.plugin_name, &fingerprint, &self.plugin_tools())
    }

    /// 治理管线按 `descriptor_id` 调用（`ExternalToolHandler`），因此这里先反解 descriptor_id，
    /// 再兼容模型可见名（防御性：避免别名形状成为分派真值）。
    fn call(
        &self,
        descriptor_id: &str,
        arguments: Value,
    ) -> Result<Value, crate::agent::tools::ToolProviderError> {
        let tool_name = match parse_plugin_descriptor_id(descriptor_id) {
            Some((plugin_name, tool_name)) if plugin_name == self.plugin_name => tool_name,
            _ => match parse_plugin_tool_model_name(descriptor_id) {
                Some((plugin_name, tool_name)) if plugin_name == self.plugin_name => tool_name,
                _ => {
                    return Err(crate::agent::tools::ToolProviderError::new(
                        "plugin_unknown_tool",
                        format!(
                            "descriptor_id `{descriptor_id}` 不属于插件来源 `{}`。",
                            self.source_id
                        ),
                    ))
                }
            },
        };
        let deadline = self
            .supervisor
            .lock()
            .map(|supervisor| supervisor.call_timeout())
            .unwrap_or(Duration::from_millis(DEFAULT_PLUGIN_TIMEOUT_MS));
        self.plugin_call(tool_name, arguments, deadline)
            .map_err(|err| provider_error(err.to_string()))
    }
}

/// 由插件注册面构造 `ToolDescriptor`（`list_tools` 的唯一实现）。
///
/// 抽成自由函数是为了能在不 spawn 进程的前提下单测"descriptor 是否被 registry 接受"——
/// 这是跨通道一致性（`register_tool_provider`）的关键约束，不能只在集成测试里验证。
fn plugin_tool_descriptors(
    plugin_name: &str,
    manifest_fingerprint: &str,
    tools: &[PluginToolRegistration],
) -> Vec<ToolDescriptor> {
    let source_id = format!("{PLUGIN_TOOL_NAMESPACE}:{plugin_name}");
    tools
        .iter()
        .map(|tool| {
            let model_name = plugin_tool_model_name(plugin_name, &tool.name);
            ToolDescriptor {
                identity: ToolIdentity {
                    descriptor_id: plugin_tool_descriptor_id(plugin_name, &tool.name),
                    model_name: model_name.clone(),
                    canonical_name: model_name.clone(),
                    primitive_name: model_name.clone(),
                    source: ToolDescriptorSource::Dynamic,
                },
                // ADR 0065 决策 7：模型可见名必须同时登记进 `aliases`，否则
                // `ToolRegistrySnapshot::resolve("plugin__<name>__<tool>")` 取不到
                // （registry 的别名索引是模型名反查 descriptor 的唯一入口；MCP 侧同口径）。
                aliases: vec![model_name.clone()],
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
                kind: ToolKind::External,
                exposure: ToolExposure::ModelVisible,
                permission_declaration: ToolPermissionDeclaration {
                    scopes: BTreeSet::new(),
                    // 外部进程工具默认需要治理面审批（T5 信任面落地前的保守取值）。
                    requires_approval: true,
                    host_mediated: true,
                    approval_mode: None,
                },
                execution_policy: ToolExecutionPolicy {
                    default_timeout_ms: DEFAULT_PLUGIN_TIMEOUT_MS,
                    cancellable: true,
                    concurrent_safe: false,
                    ..ToolExecutionPolicy::default()
                },
                display_metadata: ToolDisplayMetadata::default(),
                handler_provenance: ToolHandlerProvenance {
                    handler_kind: "plugin_stdio_jsonrpc".to_string(),
                    source_id: source_id.clone(),
                },
                source_revision: manifest_fingerprint.to_string(),
                composed_descriptor_ids: Vec::new(),
            }
        })
        .collect()
}

/// `dynamic:plugin:<name>:<tool>`（registry 当前只接受 `builtin:/mcp:/skill:/dynamic:` 前缀）。
fn plugin_tool_descriptor_id(plugin_name: &str, tool_name: &str) -> String {
    format!("dynamic:{PLUGIN_TOOL_NAMESPACE}:{plugin_name}:{tool_name}")
}

/// 反解 [`plugin_tool_descriptor_id`]。
fn parse_plugin_descriptor_id(descriptor_id: &str) -> Option<(&str, &str)> {
    let rest = descriptor_id.strip_prefix("dynamic:plugin:")?;
    let (plugin_name, tool_name) = rest.split_once(':')?;
    if plugin_name.is_empty() || tool_name.is_empty() {
        return None;
    }
    Some((plugin_name, tool_name))
}

/// 把 supervisor 的稳定错误串（`plugin_*:` 前缀）还原为错误分类，避免在适配层丢失来源信息。
fn classify_plugin_failure(error: String) -> PluginHostError {
    if error.starts_with("plugin_timeout") {
        PluginHostError::Timeout(error)
    } else if error.starts_with("plugin_capability_denied") {
        PluginHostError::CapabilityDenied(error)
    } else if error.starts_with("plugin_unavailable") {
        PluginHostError::Unavailable(error)
    } else if error.starts_with("plugin_unknown_tool") || error.starts_with("plugin_tool_error") {
        PluginHostError::Server(error)
    } else if error.starts_with("plugin_manifest_invalid")
        || error.starts_with("plugin_api_version_mismatch")
        || error.starts_with("plugin_capability_unsupported")
    {
        PluginHostError::Manifest(error)
    } else {
        PluginHostError::Protocol(error)
    }
}

/// 错误串 → `ToolProviderError`：保留**来源错误码家族**（`plugin_*`），绝不塌缩成
/// `handler_error`（ADR 0065 决策 9）。message 里剥掉重复的前缀，避免
/// `plugin_timeout: plugin_timeout: ...`。
fn provider_error(error: String) -> crate::agent::tools::ToolProviderError {
    const DEFAULT_CODE: &str = "plugin_unavailable";
    match error.split_once(": ") {
        Some((code, message)) if code.starts_with("plugin_") => {
            crate::agent::tools::ToolProviderError::new(code, message)
        }
        _ => crate::agent::tools::ToolProviderError::new(DEFAULT_CODE, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_tool_names_are_namespaced_and_reversible() {
        let model_name = plugin_tool_model_name("echo", "say");
        assert_eq!(model_name, "plugin__echo__say");
        assert_eq!(
            parse_plugin_tool_model_name(&model_name),
            Some(("echo", "say"))
        );
        assert_eq!(parse_plugin_tool_model_name("plugin__echo"), None);
        assert_eq!(parse_plugin_tool_model_name("mcp__echo__say"), None);
        assert_eq!(parse_plugin_tool_model_name("plugin__echo__a__b"), None);
    }

    #[test]
    fn plugin_descriptors_satisfy_the_registry_source_contract() {
        let tools = vec![PluginToolRegistration {
            name: "echo".to_string(),
            description: "echo".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let descriptors = plugin_tool_descriptors("echo_fixture", "sha256:deadbeef", &tools);
        assert_eq!(descriptors.len(), 1);
        let descriptor = &descriptors[0];
        assert_eq!(descriptor.identity.model_name, "plugin__echo_fixture__echo");
        assert_eq!(
            descriptor.identity.descriptor_id,
            "dynamic:plugin:echo_fixture:echo"
        );
        assert_eq!(descriptor.handler_provenance.source_id, "plugin:echo_fixture");
        assert_eq!(descriptor.source_revision, "sha256:deadbeef");
        assert_eq!(
            descriptor.aliases,
            vec!["plugin__echo_fixture__echo".to_string()],
            "模型可见名必须登记进 aliases（ADR 0065 决策 7），否则 resolve 取不到"
        );

        // 关键：descriptor 必须被 registry 的外部来源接缝原样接受（否则
        // `dispatcher.register_tool_provider` 会在注册期失败）。
        let registry = crate::agent::tools::ToolRegistrySnapshot::builtin()
            .expect("builtin registry");
        let replaced = registry
            .replace_source("plugin:echo_fixture", "builtin-tools::plugin:echo_fixture", descriptors)
            .expect("plugin descriptors must be accepted by the registry source seam");
        assert!(replaced
            .descriptors
            .iter()
            .any(|item| item.identity.descriptor_id == "dynamic:plugin:echo_fixture:echo"));
        // 关键（ADR 0065 决策 7）：模型可见名必须可反查到 descriptor。
        let resolved = replaced
            .resolve("plugin__echo_fixture__echo")
            .expect("resolve(model_name) 必须命中插件 descriptor");
        assert_eq!(
            resolved.identity.descriptor_id,
            "dynamic:plugin:echo_fixture:echo"
        );
    }

    #[test]
    fn error_codes_are_source_distinguishable() {
        let error = PluginHostError::Timeout("慢".to_string());
        assert_eq!(error.code(), "plugin_timeout");
        assert!(error.is_transport_level());
        assert!(!PluginHostError::Server("工具报错".to_string()).is_transport_level());
    }
}
