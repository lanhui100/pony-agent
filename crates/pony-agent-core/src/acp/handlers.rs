//! ACP 反向请求的宿主 handler（B175）：`fs/*` 与 `terminal/*` 的治理实现。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2.4（fs → `path_permission`）/
//! §3.2.5（terminal → `ProcessManager`）/ §4 P1 表；契约矩阵
//! `.dev-team/contract-matrix-acp-capabilities.md`（C1–C9）。
//!
//! 装配约束：这两个 handler 必须在 `connect_with_capabilities` **之前**构造好，其存在性决定
//! `initialize` 的 `clientCapabilities.fs/terminal` 声明（见
//! [`crate::acp::session::capabilities_for`]）；未声明能力时 agent 不会发对应反向请求。
//!
//! 失败语义（禁吞异常 / 禁伪造成功）：治理拒绝 → [`AcpHandlerError::Denied`]（消息保留
//! `permission_error.code`），目标不存在 → [`AcpHandlerError::NotFound`]，I/O 与内部失败 →
//! [`AcpHandlerError::Internal`]；全部映射为 JSON-RPC error，绝不返回空 content 冒充成功。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::acp::client::{AcpHandlerError, FsHandler, TerminalHandler};
use crate::acp::protocol::{
    CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest, ReadTextFileRequest,
    ReadTextFileResponse, ReleaseTerminalRequest, TerminalExitStatus, TerminalOutputRequest,
    TerminalOutputResponse, WaitForTerminalExitRequest, WaitForTerminalExitResponse,
    WriteTextFileRequest, WriteTextFileResponse,
};
use crate::acp::transport::ACP_REQUEST_TIMEOUT;
use crate::agent::path_permission::{
    AuthorizeStore, PathPermissionChecker, PathPurpose, PermissionError,
};
use crate::agent::process::ProcessManager;
use crate::agent::tool_runtime::{
    ProcessBackend, ProcessPollResult, ProcessStartRequest, ProcessState, SandboxRequest,
};

/// 单次 `fs/read_text_file` 的读取上限（fail-closed）。
///
/// 语义同源：文件工具整读上限 `tools/workspace.rs::MAX_FULL_READ_BYTES = 120_000`
/// （该常量对 `tools` 模块私有，此处以同一数值表达同一承诺：超过即拒绝，让外部 agent 用
/// `line`/`limit` 分片读，而不是把宿主内存拖爆）。
pub const ACP_FS_READ_MAX_BYTES: u64 = 120_000;

/// `terminal/wait_for_exit` 的轮询切片（≤ NFR `acp.request_timeout_ms` 的预算内快速响应）。
const TERMINAL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// 受控 tmp 目录名（workspace 根下的 `.tmp`，与 `tools/paths.rs` 的受控 tmp 布局一致）。
const CONTROLLED_TMP_DIR: &str = ".tmp";

/// `fs/read_text_file` / `fs/write_text_file` 的 workspace 治理实现。
///
/// # 语义
/// - **read**：`PathPermissionChecker::classify(.., PathPurpose::Read)`（内部按 purpose 调
///   `canonicalize_for_read`；相对路径按 workspace root 解析）；`AuthorizedExternal` 可放行、
///   未授权外部读 fail-closed；随后按 `line`/`limit`（1-based）切片读取。
/// - **write**：`classify(.., PathPurpose::Write)`（词法拒 `..` 组件与绝对逃逸 + 最近存在祖先
///   canonicalize 校验）；越界/未授权一律 fail-closed；写入用同目录 `tmp` + `rename` 原子落盘，
///   绝不 `create_dir_all` 到 workspace 外。
/// - **错误**：治理拒绝 → `Denied`（保留 `permission_error.code`）；不存在 → `NotFound`；
///   I/O 失败 → `Internal`（不向 agent 泄漏宿主绝对路径细节）。
/// - **禁用面**：不绕过 `PathPermissionChecker`；不跟随符号链接逃逸（由 canonicalize 收口）。
pub struct WorkspaceFsHandler {
    /// 会话 workspace root（canonical，来自 `ToolExecutionContext.workspace_root`）。
    workspace_root: PathBuf,
    /// 显式读授权清单（PA-080）：workspace 外读取需命中授权。
    authorize_store: Arc<AuthorizeStore>,
    /// 用途标签（审计/诊断用；read/write 方法各按自己的 purpose 判定）。
    purpose: PathPurpose,
    /// 统一路径判定器（与文件工具同一把尺）。
    path_checker: PathPermissionChecker,
}

impl WorkspaceFsHandler {
    /// 构造 handler（必须在 `connect_with_capabilities` 前调用）。
    pub fn new(
        workspace_root: PathBuf,
        authorize_store: Arc<AuthorizeStore>,
        purpose: PathPurpose,
    ) -> Self {
        // canonicalize 失败不致命（如权限受限的只读目录）：退化为原路径（与文件工具同口径）。
        let workspace_root = std::fs::canonicalize(&workspace_root).unwrap_or(workspace_root);
        Self {
            workspace_root,
            authorize_store,
            purpose,
            path_checker: PathPermissionChecker::with_default(),
        }
    }

    fn tmp_dir(&self) -> PathBuf {
        self.workspace_root.join(CONTROLLED_TMP_DIR)
    }

    /// 治理判定：`classify` 失败一律 fail-closed（保留 `permission_error.code` 语义）。
    fn classify(&self, raw_path: &str, purpose: PathPurpose) -> Result<PathBuf, AcpHandlerError> {
        self.path_checker
            .classify(
                raw_path,
                &self.workspace_root,
                &self.tmp_dir(),
                &self.authorize_store,
                purpose,
            )
            .map(|permission| permission.canonical)
            .map_err(|error| {
                // 结构化诊断带用途标签（`purpose` 是构造时注入的用途上下文，不只用于审计）。
                eprintln!(
                    "[acp][fs] purpose={} 拒绝 {}：{}",
                    self.purpose_label(),
                    raw_path,
                    error.code.as_str()
                );
                map_permission_error(error)
            })
    }

    /// 构造期用途标签（审计/诊断；每个方法仍按自己的 `PathPurpose` 判定）。
    fn purpose_label(&self) -> &'static str {
        match self.purpose {
            PathPurpose::Read => "read",
            PathPurpose::Write => "write",
        }
    }

    /// 不泄漏宿主绝对路径的目标描述（内部错误/不存在错误用）。
    fn describe_target(&self, path: &Path) -> String {
        match path.strip_prefix(&self.workspace_root) {
            Ok(relative) => format!("<workspace>/{}", relative.display()),
            // 外部路径只给文件名，绝不给宿主绝对目录结构。
            Err(_) => path
                .file_name()
                .map(|name| format!("<external>/{}", name.to_string_lossy()))
                .unwrap_or_else(|| "<external>".to_string()),
        }
    }
}

impl FsHandler for WorkspaceFsHandler {
    fn read_text_file(
        &self,
        request: &ReadTextFileRequest,
    ) -> Result<ReadTextFileResponse, AcpHandlerError> {
        let canonical = self.classify(&request.path, PathPurpose::Read)?;
        let metadata = std::fs::metadata(&canonical).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AcpHandlerError::NotFound(format!(
                    "文件不存在：{}。",
                    self.describe_target(&canonical)
                ))
            } else {
                AcpHandlerError::Internal(format!(
                    "读取文件元数据失败（{}）：{error}。",
                    self.describe_target(&canonical)
                ))
            }
        })?;
        if !metadata.is_file() {
            return Err(AcpHandlerError::Denied(format!(
                "permission_denied: 目标不是普通文件（{}）。",
                self.describe_target(&canonical)
            )));
        }
        if metadata.len() > ACP_FS_READ_MAX_BYTES {
            // fail-closed：不截断、不假装成功——让 agent 用 line/limit 分片读。
            return Err(AcpHandlerError::Denied(format!(
                "permission_denied: 文件 {} 字节超过 ACP 读取上限 {ACP_FS_READ_MAX_BYTES} 字节，请用 line/limit 分片读取。",
                metadata.len()
            )));
        }
        let bytes = std::fs::read(&canonical).map_err(|error| {
            AcpHandlerError::Internal(format!(
                "读取文件失败（{}）：{error}。",
                self.describe_target(&canonical)
            ))
        })?;
        let text = String::from_utf8(bytes).map_err(|_| {
            AcpHandlerError::Internal(format!(
                "文件不是合法 UTF-8（{}），拒绝以文本读取。",
                self.describe_target(&canonical)
            ))
        })?;
        Ok(ReadTextFileResponse {
            content: slice_lines(&text, request.line, request.limit),
        })
    }

    fn write_text_file(
        &self,
        request: &WriteTextFileRequest,
    ) -> Result<WriteTextFileResponse, AcpHandlerError> {
        let canonical = self.classify(&request.path, PathPurpose::Write)?;
        let Some(parent) = canonical.parent() else {
            return Err(AcpHandlerError::Internal(
                "写入路径缺少父目录，已拒绝。".to_string(),
            ));
        };
        // 不在 ACP 请求里 create_dir_all：父目录必须已存在（失败即明确报错，不越界建目录）。
        if !parent.is_dir() {
            return Err(AcpHandlerError::Denied(format!(
                "outside_workspace_write_denied: 父目录不存在，已拒绝写入（{}）。",
                self.describe_target(&canonical)
            )));
        }
        // 原子写：同目录 tmp（随机名，create_new 防撞名）→ rename 覆盖。
        let tmp = parent.join(format!(".acp-write-{}.tmp", uuid::Uuid::new_v4()));
        let write_result = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .and_then(|mut file| std::io::Write::write_all(&mut file, request.content.as_bytes()));
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&tmp);
            return Err(AcpHandlerError::Internal(format!(
                "写入临时文件失败（{}）：{error}。",
                self.describe_target(&canonical)
            )));
        }
        if let Err(error) = std::fs::rename(&tmp, &canonical) {
            let _ = std::fs::remove_file(&tmp);
            return Err(AcpHandlerError::Internal(format!(
                "原子落盘失败（{}）：{error}。",
                self.describe_target(&canonical)
            )));
        }
        Ok(WriteTextFileResponse {})
    }
}

/// `classify` 的治理拒绝 → `AcpHandlerError::Denied`（保留真源码 `permission_denied` 等）。
fn map_permission_error(error: PermissionError) -> AcpHandlerError {
    AcpHandlerError::Denied(format!("{}: {}", error.code.as_str(), error.message))
}

/// `line`（1-based）/`limit` 切片；两者都缺省时逐字节原样返回（不做行尾归一）。
fn slice_lines(text: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_string();
    }
    let skip = line.unwrap_or(1).max(1) as usize - 1;
    let take = limit.map(|value| value as usize).unwrap_or(usize::MAX);
    text.split_inclusive('\n')
        .skip(skip)
        .take(take)
        .collect::<String>()
}

/// 一个已 `terminal/create` 的受管终端。
#[derive(Clone)]
struct TerminalEntry {
    /// ProcessManager 的 session 归属（跨 session 句柄一律 fail-closed）。
    session_id: String,
    /// ProcessManager 句柄（wire `terminalId` 是它外面的 opaque 包装，不可伪造）。
    handle: String,
}

/// `terminal/*` 的 ProcessManager 治理实现。
///
/// # 语义
/// - `create` → [`ProcessManager`] 的 `ProcessBackend::start`（`sandbox.isolate_environment =
///   true`；env 只经 `environment_allowlist` 且拒绝凭据/加载器劫持键）；返回 handle 即 wire
///   `terminalId`（opaque 包装，cross-session 伪造 → `NotFound`）。
/// - `output` → `ProcessBackend::poll`（stdout + stderr 顺序拼接；`truncated` 取两流之或；
///   `exitStatus` 由 `exit_code` 归约，`signal` 无独立字段 ⇒ `None`）。
/// - `wait_for_exit` → 有界轮询 `poll` 直到 `Exited` 或 [`ACP_REQUEST_TIMEOUT`] 超时。
/// - `kill` → `kill_if_exists`（幂等；保留 handle 供最终 `output`）。
/// - `release` → `kill_if_exists` + 条目回收（释放后 handle 不再可用，且无残留子进程）。
/// - **禁用面**：不存在裸 `std::process` 调用；唯一进程编排口是 `ProcessManager`。
pub struct ManagedTerminalHandler {
    /// 会话进程编排器（kill/cancel/kill_after/drain_stats 的既有回收语义）。
    process_manager: Arc<ProcessManager>,
    /// wire `terminalId` → 受管句柄（opaque、不可跨会话伪造）。
    terminals: Mutex<HashMap<String, TerminalEntry>>,
}

impl ManagedTerminalHandler {
    /// 构造 handler（必须在 `connect_with_capabilities` 前调用）。
    pub fn new(process_manager: Arc<ProcessManager>) -> Self {
        Self {
            process_manager,
            terminals: Mutex::new(HashMap::new()),
        }
    }

    /// 解析 `terminalId`（未知 / 跨 session ⇒ `NotFound`，绝不 panic）。
    fn entry(&self, session_id: &str, terminal_id: &str) -> Result<TerminalEntry, AcpHandlerError> {
        let terminals = self.terminals.lock().map_err(|_| {
            AcpHandlerError::Internal("terminal 句柄表已中毒，fail-closed。".to_string())
        })?;
        match terminals.get(terminal_id) {
            Some(entry) if entry.session_id == session_id => Ok(entry.clone()),
            Some(_) => Err(AcpHandlerError::NotFound(format!(
                "terminalId `{terminal_id}` 不属于当前会话（跨 session 伪造已拒绝）。"
            ))),
            None => Err(AcpHandlerError::NotFound(format!(
                "未知 terminalId `{terminal_id}`（未创建或已 release）。"
            ))),
        }
    }

    fn poll(&self, entry: &TerminalEntry) -> Result<ProcessPollResult, AcpHandlerError> {
        self.process_manager
            .poll(&entry.session_id, &entry.handle)
            .map_err(|error| {
                AcpHandlerError::Internal(format!(
                    "terminal 输出轮询失败（{}）：{error}。",
                    entry.handle
                ))
            })
    }
}

impl TerminalHandler for ManagedTerminalHandler {
    fn create(
        &self,
        request: &CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse, AcpHandlerError> {
        if request.session_id.trim().is_empty() {
            return Err(AcpHandlerError::Denied(
                "permission_denied: terminal/create 缺少 sessionId。".to_string(),
            ));
        }
        if request.command.trim().is_empty() {
            return Err(AcpHandlerError::Internal(
                "terminal/create 缺少 command，已拒绝。".to_string(),
            ));
        }
        let environment_allowlist = request
            .env
            .as_deref()
            .unwrap_or(&[])
            .iter()
            // 显式 env 仍必须过凭据/加载器劫持拒绝表（与 MCP spec env 同一口径）。
            .filter(|variable| !crate::agent::mcp_client::is_denied_spec_env_key(&variable.name))
            .map(|variable| format!("{}={}", variable.name, variable.value))
            .collect::<Vec<_>>();
        let start = ProcessStartRequest {
            session_id: request.session_id.clone(),
            program: request.command.clone(),
            arguments: request.args.clone().unwrap_or_default(),
            sandbox: SandboxRequest {
                workspace_root: request
                    .cwd
                    .clone()
                    .or_else(|| {
                        std::env::current_dir()
                            .ok()
                            .map(|path| path.display().to_string())
                    })
                    .unwrap_or_default(),
                allow_network: false,
                environment_allowlist,
                // 外部 agent 的终端请求一律走最小环境（不继承 provider key / 代理 / 会话密钥）。
                isolate_environment: true,
            },
        };
        let handle = self.process_manager.start(&start).map_err(|error| {
            AcpHandlerError::Internal(format!("terminal/create 启动失败：{error}。"))
        })?;
        let terminal_id = format!("acp-term-{}", uuid::Uuid::new_v4());
        let mut terminals = self.terminals.lock().map_err(|_| {
            AcpHandlerError::Internal("terminal 句柄表已中毒，fail-closed。".to_string())
        })?;
        terminals.insert(
            terminal_id.clone(),
            TerminalEntry {
                session_id: request.session_id.clone(),
                handle,
            },
        );
        Ok(CreateTerminalResponse { terminal_id })
    }

    fn output(
        &self,
        request: &TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, AcpHandlerError> {
        let entry = self.entry(&request.session_id, &request.terminal_id)?;
        let poll = self.poll(&entry)?;
        // 契约 §4：stdout + stderr 顺序拼接；truncated 取两流之或。
        let mut output = poll.stdout;
        output.push_str(&poll.stderr);
        Ok(TerminalOutputResponse {
            output,
            truncated: poll.stdout_truncated || poll.stderr_truncated,
            exit_status: poll.exit_code.map(|code| TerminalExitStatus {
                exit_code: Some(code),
                // ProcessPollResult 无独立 signal 字段 ⇒ 契约映射 None。
                signal: None,
            }),
        })
    }

    fn wait_for_exit(
        &self,
        request: &WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse, AcpHandlerError> {
        let entry = self.entry(&request.session_id, &request.terminal_id)?;
        let deadline = Instant::now() + ACP_REQUEST_TIMEOUT;
        loop {
            let poll = self.poll(&entry)?;
            if poll.state == ProcessState::Exited {
                return Ok(WaitForTerminalExitResponse {
                    exit_code: poll.exit_code,
                    signal: None,
                });
            }
            if Instant::now() >= deadline {
                // 有界等待：不无限阻塞 transport 读线程；超时明确报错（仍可 kill/release）。
                return Err(AcpHandlerError::Internal(format!(
                    "terminal/wait_for_exit 超过 {:?} 仍未退出（terminalId 仍可 kill/release）。",
                    ACP_REQUEST_TIMEOUT
                )));
            }
            std::thread::sleep(TERMINAL_POLL_INTERVAL);
        }
    }

    fn kill(&self, request: &KillTerminalRequest) -> Result<(), AcpHandlerError> {
        let entry = self.entry(&request.session_id, &request.terminal_id)?;
        // 幂等：已退出的进程被 `kill_if_exists` 回收条目后再次调用仍返回 Ok。
        self.process_manager
            .kill_if_exists(&entry.session_id, &entry.handle)
            .map_err(|error| {
                AcpHandlerError::Internal(format!(
                    "terminal/kill 失败（{}）：{error}。",
                    entry.handle
                ))
            })
    }

    fn release(&self, request: &ReleaseTerminalRequest) -> Result<(), AcpHandlerError> {
        let entry = self.entry(&request.session_id, &request.terminal_id)?;
        // 先确保进程树终止（幂等），再回收 wire 句柄：释放后不再可用，且无残留。
        self.process_manager
            .kill_if_exists(&entry.session_id, &entry.handle)
            .map_err(|error| {
                AcpHandlerError::Internal(format!(
                    "terminal/release 终止进程失败（{}）：{error}。",
                    entry.handle
                ))
            })?;
        if let Ok(mut terminals) = self.terminals.lock() {
            terminals.remove(&request.terminal_id);
        }
        Ok(())
    }
}
