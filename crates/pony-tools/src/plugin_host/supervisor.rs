//! 插件监督者：生命周期 `spawn → initialize（能力协商）→ 注册 → call → shutdown`。
//!
//! 失败语义（ADR 0067 决策 2/3，全部 fail-closed）：
//! - 插件崩溃/自行退出 → 本轮返回 `plugin_unavailable`，**不重启、不重试**（避免重试风暴把
//!   一个有病的插件放大成宿主级故障）；后续调用继续返回 `plugin_unavailable`，
//!   恢复方式是调用方重建 supervisor；
//! - 单次调用超时 → 立即 SIGKILL 进程组并同步回收（不留残留子进程），返回 `plugin_timeout`；
//! - 能力默认拒绝：manifest 未声明 / 未协商通过的能力一律拒绝；
//! - manifest 信任指纹：`spawn_with_trust` 钉住内容哈希，manifest 被替换即拒绝加载。
//!
//! 平台局限（ADR 0067 已知局限，需写进验收）：Windows 无进程组，
//! [`PluginTransport::terminate`] 只能 kill 直接子进程，插件派生的孙进程为"尽力回收"。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::capability::{resolve_workspace_dir, NegotiatedCapabilities};
use super::manifest::{
    hex_sha256, PluginManifest, PluginToolDecl, CAPABILITY_TOOLS, PLUGIN_API_VERSION,
};
use super::transport::PluginTransport;
use super::PluginHostError;

/// 单次插件调用的默认预算（毫秒）。与 hooks 的 5000ms 对齐：两者都是"外部进程 + 回合内同步"。
pub const DEFAULT_PLUGIN_TIMEOUT_MS: u64 = 5_000;

/// 握手（initialize / tools-list）预算下限。
///
/// 冷启动解释型插件（python/node，或 Windows 上 `python.exe` 首启）常超过 1s，
/// 用调用预算做握手会制造"看起来是超时、其实是冷启动"的假失败。
pub const MIN_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);

/// 单次调用 deadline 的取值上限：防止调用方误传巨大值把回合挂死。
pub const MAX_CALL_DEADLINE: Duration = Duration::from_secs(300);

/// 命令哈希的文件读取上限（超过则跳过，不把大二进制读进内存）。
const COMMAND_HASH_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 审计事件环形缓冲上限（进程内可观测面，持久化审计总线不在 T3 范围）。
const AUDIT_BUFFER_LIMIT: usize = 256;

/// 插件宿主审计事件。
///
/// ADR 0067 要求"审计日志记录插件名、命令哈希、能力集合、每次调用成败"；v1 没有持久化
/// 审计总线（属可观测性收口 T5/`docs/observability.yaml`），因此先落进程内缓冲 + stderr
/// 结构化行，两处都只含**元数据**，绝不含工具入参/出参内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginAuditEvent {
    /// 事件名：`plugin_load` / `plugin_initialized` / `plugin_call_ok` /
    /// `plugin_call_failed` / `plugin_crash` / `plugin_shutdown` / `plugin_untrusted`。
    pub event: String,
    /// 插件名（manifest 声明）。
    pub plugin: String,
    /// manifest 内容指纹（`sha256:<hex>`）。
    pub manifest_sha256: String,
    /// 可执行文件哈希（读取失败/超限时为 `None`，绝不猜测）。
    pub command_sha256: Option<String>,
    /// 每次加载协商后的能力集合（稳定排序）。
    pub capabilities: Vec<String>,
    /// 结构性细节（错误码、工具名；**不含**入参内容）。
    pub detail: String,
}

static AUDIT_BUFFER: Mutex<Vec<PluginAuditEvent>> = Mutex::new(Vec::new());

/// 读取进程内审计事件快照（诊断/验收用）。
pub fn plugin_audit_events() -> Vec<PluginAuditEvent> {
    AUDIT_BUFFER
        .lock()
        .map(|buffer| buffer.clone())
        .unwrap_or_default()
}

/// 清空进程内审计事件（测试隔离用）。
pub fn clear_plugin_audit_events() {
    if let Ok(mut buffer) = AUDIT_BUFFER.lock() {
        buffer.clear();
    }
}

fn record_audit(
    event: &str,
    manifest: &PluginManifest,
    manifest_sha256: &str,
    command_sha256: Option<&String>,
    capabilities: &[String],
    detail: impl Into<String>,
) {
    let record = PluginAuditEvent {
        event: event.to_string(),
        plugin: manifest.name.clone(),
        manifest_sha256: manifest_sha256.to_string(),
        command_sha256: command_sha256.cloned(),
        capabilities: capabilities.to_vec(),
        detail: detail.into(),
    };
    if let Ok(mut buffer) = AUDIT_BUFFER.lock() {
        buffer.push(record.clone());
        if buffer.len() > AUDIT_BUFFER_LIMIT {
            buffer.remove(0);
        }
    }
    // 结构化 stderr 行：与进程内缓冲同一份事实，供宿主日志采集。
    if let Ok(line) = serde_json::to_string(&json!({
        "event": record.event,
        "plugin": record.plugin,
        "manifestSha256": record.manifest_sha256,
        "commandSha256": record.command_sha256,
        "capabilities": record.capabilities,
        "detail": record.detail,
    })) {
        eprintln!("[pony-agent][plugin][audit] {line}");
    }
}

/// 插件宿主配置。
///
/// 严格保持两个 pub 字段（验收测试按此形状冻结）。信任指纹不在配置里，而由
/// [`PluginSupervisor::spawn_with_trust`] 显式承载：**spawn 调用本身即"显式信任"动作**，
/// 宿主没有任何自动发现/配置扫描路径（不会随仓库里的 `plugin.json` 自动启用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginHostConfig {
    /// 会话工作区（插件子进程的 cwd，且是其唯一被允许的目录）。
    pub workspace_root: PathBuf,
    /// 单次调用超时毫秒（0 → [`DEFAULT_PLUGIN_TIMEOUT_MS`]）。
    pub timeout_ms: u64,
}

impl Default for PluginHostConfig {
    fn default() -> Self {
        let workspace_root = std::env::var_os("PONY_WORKSPACE_ROOT")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            workspace_root,
            timeout_ms: DEFAULT_PLUGIN_TIMEOUT_MS,
        }
    }
}

impl PluginHostConfig {
    pub fn with_workspace_root(mut self, workspace_root: impl Into<PathBuf>) -> Self {
        self.workspace_root = workspace_root.into();
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    fn effective_timeout(&self) -> Duration {
        let ms = if self.timeout_ms == 0 {
            DEFAULT_PLUGIN_TIMEOUT_MS
        } else {
            self.timeout_ms
        };
        Duration::from_millis(ms)
    }
}

/// 注册阶段确认的一个插件工具。
#[derive(Clone, Debug, PartialEq)]
pub struct PluginToolRegistration {
    /// 插件内的工具名（模型可见名为 `plugin__<plugin>__<tool>`）。
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// 插件监督者：一个进程 + 一个 JSON-RPC 会话 + 一份失败状态。
pub struct PluginSupervisor {
    manifest: PluginManifest,
    manifest_sha256: String,
    command: PathBuf,
    command_sha256: Option<String>,
    config: PluginHostConfig,
    transport: Option<PluginTransport>,
    negotiated: NegotiatedCapabilities,
    tools: Vec<PluginToolRegistration>,
    initialized: bool,
    /// fail-closed 状态：一旦置位，所有调用立即返回 `plugin_unavailable`（不重启、不重试）。
    failure: Option<String>,
}

impl std::fmt::Debug for PluginSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginSupervisor")
            .field("plugin", &self.manifest.name)
            .field("command", &self.command)
            .field("initialized", &self.initialized)
            .field("failure", &self.failure)
            .finish()
    }
}

impl PluginSupervisor {
    /// 启动插件进程（**不**做握手；握手见 [`PluginSupervisor::initialize`]）。
    ///
    /// 失败闭合点：
    /// - manifest 重新校验（struct literal 也可能绕过 `parse`）；
    /// - 可执行文件必须存在、是普通文件、Unix 上具备执行位；
    /// - `workspace_root` 必须存在且是目录（子进程 cwd 唯一合法根）。
    pub fn spawn(
        manifest: PluginManifest,
        command: PathBuf,
        config: PluginHostConfig,
    ) -> Result<Self, String> {
        Self::spawn_inner(manifest, command, config, None)
    }

    /// 带信任指纹的启动：`expected_manifest_sha256` 必须等于 manifest 内容指纹，
    /// 否则拒绝加载（ADR 0067：路径 + manifest 内容哈希双重指纹，manifest 变更即失效）。
    pub fn spawn_with_trust(
        manifest: PluginManifest,
        command: PathBuf,
        config: PluginHostConfig,
        expected_manifest_sha256: &str,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            manifest,
            command,
            config,
            Some(expected_manifest_sha256.to_string()),
        )
    }

    fn spawn_inner(
        manifest: PluginManifest,
        command: PathBuf,
        config: PluginHostConfig,
        expected_manifest_sha256: Option<String>,
    ) -> Result<Self, String> {
        manifest.validate()?;

        let manifest_sha256 = manifest.fingerprint();
        if let Some(expected) = expected_manifest_sha256.as_deref() {
            if expected != manifest_sha256 {
                record_audit(
                    "plugin_untrusted",
                    &manifest,
                    &manifest_sha256,
                    None,
                    &manifest.capabilities,
                    format!("manifest 指纹不匹配（期望 {expected}）"),
                );
                return Err(format!(
                    "plugin_untrusted: manifest 指纹 {manifest_sha256} 与已信任指纹 {expected} 不一致（manifest 被替换，重新信任前拒绝加载）。"
                ));
            }
        }

        let metadata = std::fs::metadata(&command).map_err(|err| {
            format!(
                "plugin_spawn_failed: 插件可执行文件 {} 不可用（{err}）。",
                command.display()
            )
        })?;
        if !metadata.is_file() {
            return Err(format!(
                "plugin_spawn_failed: 插件命令 {} 不是普通文件。",
                command.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(format!(
                    "plugin_spawn_failed: 插件命令 {} 缺少执行位（Unix 需 chmod +x）。",
                    command.display()
                ));
            }
        }

        let workspace_root = resolve_workspace_dir(&config.workspace_root)?;
        let command_sha256 = file_sha256(&command);

        let transport = PluginTransport::spawn(&command, &[], &workspace_root, &[])
            .map_err(|err| err.to_string())?;

        record_audit(
            "plugin_load",
            &manifest,
            &manifest_sha256,
            command_sha256.as_ref(),
            &manifest.capabilities,
            format!("pid={} command={}", transport.pid(), command.display()),
        );

        Ok(Self {
            manifest,
            manifest_sha256,
            command,
            command_sha256,
            config,
            transport: Some(transport),
            // initialize 之前无任何能力（默认拒绝）。
            negotiated: NegotiatedCapabilities::empty(),
            tools: Vec::new(),
            initialized: false,
            failure: None,
        })
    }

    /// manifest 内容指纹（信任面用）。
    pub fn manifest_fingerprint(&self) -> &str {
        &self.manifest_sha256
    }

    /// 插件名（manifest 声明，工具命名空间 `plugin__<name>__<tool>` 的中段）。
    pub fn plugin_name(&self) -> &str {
        &self.manifest.name
    }

    /// 插件声明的工具（注册阶段确认前为空）。
    pub fn tools(&self) -> &[PluginToolRegistration] {
        &self.tools
    }

    /// manifest 声明的工具（initialize 之前的注册面兜底：仍受 manifest 白名单约束，不扩权）。
    pub fn declared_tools(&self) -> Vec<PluginToolRegistration> {
        self.manifest
            .tools
            .iter()
            .map(Self::registration_from_decl)
            .collect()
    }

    /// 本插件的默认单次调用预算（`ToolProvider::call` 没有 deadline 参数时使用）。
    pub fn call_timeout(&self) -> Duration {
        self.config.effective_timeout()
    }

    /// 是否已完成 initialize。
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// fail-closed 状态（`None` = 健康）。
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// 插件进程 pid（进程已终止/进入 fail-closed 后为 `None`）。
    pub fn child_pid(&self) -> Option<u32> {
        if self.failure.is_some() {
            return None;
        }
        self.transport.as_ref().map(PluginTransport::pid)
    }

    /// 非阻塞观察子进程退出状态（不吞退出码；供验收做有界轮询）。
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        match self.transport.as_mut() {
            Some(transport) => transport
                .try_wait_status()
                .map_err(|err| format!("plugin_unavailable: try_wait 失败：{err}。")),
            None => Ok(None),
        }
    }

    /// 插件进程是否仍在运行。
    pub fn is_running(&mut self) -> bool {
        self.transport
            .as_mut()
            .map(PluginTransport::is_running)
            .unwrap_or(false)
    }

    /// 握手：`initialize`（`apiVersion` + 能力协商）→ `notifications/initialized`
    /// → 注册（`tools/list` 与 manifest 声明交叉校验）。
    pub fn initialize(&mut self) -> Result<(), String> {
        if let Some(reason) = self.failure.clone() {
            return Err(reason);
        }
        if self.initialized {
            return Err("plugin_already_initialized: 插件已 initialize，重复握手被拒绝。".to_string());
        }
        let transport = self
            .transport
            .as_ref()
            .ok_or_else(|| "plugin_unavailable: 插件进程不存在。".to_string())?;

        let init_deadline = self.initialize_timeout();
        let params = json!({
            "apiVersion": PLUGIN_API_VERSION,
            "clientInfo": {
                "name": "pony-agent",
                "version": env!("CARGO_PKG_VERSION"),
            },
            // 宿主 v1 只实现 tools 能力；hooks 不迁移（Non-Goal）。
            "capabilities": [CAPABILITY_TOOLS],
        });

        let init = match transport.request("initialize", params, init_deadline) {
            Ok(value) => value,
            Err(err) => return Err(self.fail_closed("initialize", err)),
        };

        // apiVersion 协商：字段名宽容读（apiVersion / protocolVersion），但值必须精确匹配。
        let negotiated_api = init
            .get("apiVersion")
            .or_else(|| init.get("protocolVersion"))
            .and_then(Value::as_str);
        if let Some(api) = negotiated_api {
            if api != PLUGIN_API_VERSION {
                let reason = format!(
                    "plugin_api_version_mismatch: 插件协商的 apiVersion=`{api}` 与宿主支持的 `{PLUGIN_API_VERSION}` 不兼容。"
                );
                self.mark_failed(&reason);
                return Err(reason);
            }
        }

        let declared: Vec<String> = init
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            // 插件未回显能力 → 按 manifest 声明协商（只收窄不扩权：工具注册仍受 manifest 白名单约束）。
            .unwrap_or_else(|| self.manifest.capabilities.clone());

        match NegotiatedCapabilities::negotiate(&self.manifest, &declared) {
            Ok(negotiated) => self.negotiated = negotiated,
            Err(err) => {
                self.mark_failed(&err);
                return Err(err);
            }
        }

        if let Err(err) = transport.notify("notifications/initialized", json!({})) {
            return Err(self.fail_closed("notifications/initialized", err));
        }

        // 注册：tools/list 与 manifest 声明交叉校验（声明外一律拒绝）。
        if self.negotiated.allows(CAPABILITY_TOOLS) && self.manifest.declares_tools() {
            let registration_deadline = self.initialize_timeout();
            match transport.request("tools/list", json!({}), registration_deadline) {
                Ok(result) => {
                    self.tools = match Self::parse_tools_result(&self.manifest, &result) {
                        Ok(tools) => tools,
                        Err(err) => {
                            self.mark_failed(&err);
                            return Err(err);
                        }
                    };
                }
                // 宽容读法：插件未实现 tools/list（JSON-RPC method not found）时回落到
                // manifest 声明——注册面依旧只来自已校验的 manifest，不构成扩权。
                Err(PluginHostError::Server(detail)) => {
                    record_audit(
                        "plugin_tools_list_fallback",
                        &self.manifest,
                        &self.manifest_sha256,
                        self.command_sha256.as_ref(),
                        &self.negotiated.as_slice(),
                        detail,
                    );
                    self.tools = self.manifest.tools.iter().map(Self::registration_from_decl).collect();
                }
                Err(err) => return Err(self.fail_closed("tools/list", err)),
            }
        }

        self.initialized = true;
        record_audit(
            "plugin_initialized",
            &self.manifest,
            &self.manifest_sha256,
            self.command_sha256.as_ref(),
            &self.negotiated.as_slice(),
            format!("tools={}", self.tools.len()),
        );
        Ok(())
    }

    /// 调用插件工具（使用 [`PluginHostConfig::timeout_ms`] 作为 deadline）。
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        self.call_tool_with_deadline(name, arguments, self.config.effective_timeout())
    }

    /// 调用插件工具，deadline 由调用方给定（ADR 0067：单次调用超时来自调用方 deadline）。
    pub fn call_tool_with_deadline(
        &mut self,
        name: &str,
        arguments: Value,
        deadline: Duration,
    ) -> Result<Value, String> {
        if let Some(reason) = self.failure.clone() {
            // fail-closed：崩溃后不重试，避免重试风暴。
            return Err(format!(
                "plugin_unavailable: 插件 `{}` 已进入 fail-closed 状态，宿主不重启、不重试：{reason}",
                self.manifest.name
            ));
        }
        if !self.initialized {
            return Err(
                "plugin_unavailable: initialize() 尚未完成，拒绝调用插件工具。".to_string(),
            );
        }
        if !self.negotiated.allows(CAPABILITY_TOOLS) {
            return Err(format!(
                "plugin_capability_denied: 插件 `{}` 未协商到 tools 能力。",
                self.manifest.name
            ));
        }
        if !self.tools.iter().any(|tool| tool.name == name) {
            return Err(format!(
                "plugin_unknown_tool: 插件 `{}` 未注册工具 `{name}`（只允许 manifest 声明的工具）。",
                self.manifest.name
            ));
        }

        let deadline = deadline.max(Duration::from_millis(1)).min(MAX_CALL_DEADLINE);
        let transport = match self.transport.as_ref() {
            Some(transport) => transport,
            None => {
                return Err("plugin_unavailable: 插件进程不存在。".to_string());
            }
        };
        let params = json!({ "name": name, "arguments": arguments });
        match transport.request("tools/call", params, deadline) {
            Ok(value) => {
                if value.get("isError").and_then(Value::as_bool) == Some(true) {
                    record_audit(
                        "plugin_call_failed",
                        &self.manifest,
                        &self.manifest_sha256,
                        self.command_sha256.as_ref(),
                        &self.negotiated.as_slice(),
                        format!("tool={name} server_flagged_error"),
                    );
                    return Err(format!(
                        "plugin_tool_error: 插件 `{}` 的工具 `{name}` 返回 isError=true。",
                        self.manifest.name
                    ));
                }
                record_audit(
                    "plugin_call_ok",
                    &self.manifest,
                    &self.manifest_sha256,
                    self.command_sha256.as_ref(),
                    &self.negotiated.as_slice(),
                    format!("tool={name}"),
                );
                Ok(value)
            }
            Err(PluginHostError::Timeout(detail)) => {
                // ADR：超时杀进程组并同步回收，绝不留残留子进程。
                self.kill_transport();
                let reason = format!(
                    "plugin_timeout: 插件 `{}` 的工具 `{name}` 超过 {deadline:?} 未返回，已终止进程组（含孙进程）。{detail}",
                    self.manifest.name
                );
                self.mark_failed(&reason);
                Err(reason)
            }
            Err(err) => Err(self.fail_closed("tools/call", err)),
        }
    }

    /// 优雅关闭：发 `shutdown` → 关 stdin → 宽限 → 杀进程组 → 回收。
    ///
    /// 返回 `Err` 仅当进程无法被回收；插件在收到 `shutdown` 前自行退出（管道关闭）按成功处理。
    pub fn shutdown(mut self) -> Result<(), String> {
        let mut outcome = Ok(());
        if let Some(transport) = self.transport.as_mut() {
            let deadline = self
                .config
                .effective_timeout()
                .min(Duration::from_secs(2))
                .max(Duration::from_millis(1));
            if let Err(err) = transport.request("shutdown", json!({}), deadline) {
                record_audit(
                    "plugin_shutdown_unacknowledged",
                    &self.manifest,
                    &self.manifest_sha256,
                    self.command_sha256.as_ref(),
                    &self.negotiated.as_slice(),
                    err.to_string(),
                );
            }
            transport.terminate();
            if transport.is_running() {
                outcome = Err(format!(
                    "plugin_shutdown_failed: 插件 `{}` 进程未能被回收。",
                    self.manifest.name
                ));
            }
        }
        self.transport = None;
        self.initialized = false;
        record_audit(
            "plugin_shutdown",
            &self.manifest,
            &self.manifest_sha256,
            self.command_sha256.as_ref(),
            &self.negotiated.as_slice(),
            outcome.is_ok().to_string(),
        );
        outcome
    }

    fn initialize_timeout(&self) -> Duration {
        self.config
            .effective_timeout()
            .max(MIN_INITIALIZE_TIMEOUT)
    }

    /// 把 transport 级错误转换为 fail-closed 状态并返回稳定错误串。
    fn fail_closed(&mut self, stage: &str, err: PluginHostError) -> String {
        // 用 `message()` 而不是 `Display`：错误串由本函数负责加来源前缀，避免
        // `plugin_unavailable: ... plugin_timeout: ...` 这类嵌套双前缀。
        let detail = err.message().to_string();
        let crashed = err.is_transport_level();
        if crashed {
            self.kill_transport();
        }
        let reason = match err.code() {
            "plugin_timeout" => format!(
                "plugin_timeout: 插件 `{}` 在 `{stage}` 阶段超时，已终止进程组。{detail}",
                self.manifest.name
            ),
            code => format!(
                "plugin_unavailable: 插件 `{}` 在 `{stage}` 阶段不可用（{code}）。{detail}",
                self.manifest.name
            ),
        };
        record_audit(
            if matches!(err, PluginHostError::Server(_)) {
                "plugin_call_failed"
            } else {
                "plugin_crash"
            },
            &self.manifest,
            &self.manifest_sha256,
            self.command_sha256.as_ref(),
            &self.negotiated.as_slice(),
            format!("stage={stage} code={} detail={detail}", err.code()),
        );
        self.mark_failed(&reason);
        reason
    }

    fn mark_failed(&mut self, reason: &str) {
        if self.failure.is_none() {
            self.failure = Some(reason.to_string());
        }
    }

    fn kill_transport(&mut self) {
        if let Some(transport) = self.transport.as_mut() {
            transport.terminate();
        }
    }

    fn parse_tools_result(
        manifest: &PluginManifest,
        result: &Value,
    ) -> Result<Vec<PluginToolRegistration>, String> {
        // 宽容读法：`{tools:[...]}` 或裸数组。
        let entries = if let Some(tools) = result.get("tools").and_then(Value::as_array) {
            tools.clone()
        } else if let Some(tools) = result.as_array() {
            tools.clone()
        } else {
            return Err(format!(
                "plugin_protocol_error: tools/list 结果既不是 `{{tools:[...]}}` 也不是数组：{result}"
            ));
        };

        let mut registrations = Vec::with_capacity(entries.len());
        for entry in entries {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!("plugin_protocol_error: tools/list 条目缺少 name：{entry}")
                })?
                .to_string();
            // 声明外一律拒绝：注册面不得超出 manifest 白名单。
            let declared = manifest
                .tools
                .iter()
                .find(|decl| decl.name == name)
                .ok_or_else(|| {
                    format!(
                        "plugin_protocol_error: 插件注册了 manifest 未声明的工具 `{name}`（声明外一律拒绝）。"
                    )
                })?;
            registrations.push(PluginToolRegistration {
                name,
                description: entry
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| declared.description.clone()),
                input_schema: entry
                    .get("inputSchema")
                    .filter(|schema| schema.is_object())
                    .cloned()
                    .unwrap_or_else(|| declared.input_schema.clone()),
            });
        }
        Ok(registrations)
    }

    fn registration_from_decl(decl: &PluginToolDecl) -> PluginToolRegistration {
        PluginToolRegistration {
            name: decl.name.clone(),
            description: decl.description.clone(),
            input_schema: decl.input_schema.clone(),
        }
    }
}

impl Drop for PluginSupervisor {
    fn drop(&mut self) {
        // 宿主 drop 时 kill 进程组并回收（ADR 0067 决策 2）。
        if let Some(transport) = self.transport.as_mut() {
            transport.terminate();
        }
        self.transport = None;
    }
}

/// 计算文件 sha256（有界读取；失败或超限返回 `None`，绝不猜测）。
fn file_sha256(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > COMMAND_HASH_MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!("sha256:{}", hex_sha256(&bytes)))
}
