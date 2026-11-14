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
use crate::acp::ACP_DEFAULT_TURN_BUDGET;
use crate::agent::path_permission::{
    AuthorizeStore, PathPermissionChecker, PathPurpose, PermissionError,
};
use crate::agent::process::ProcessManager;
use crate::agent::tool_runtime::{
    ProcessBackend, ProcessStartRequest, ProcessState, SandboxRequest,
};

/// 单次 `fs/read_text_file` 的读取上限（fail-closed）。
///
/// 语义同源：文件工具整读上限 `tools/workspace.rs::MAX_FULL_READ_BYTES = 120_000`
/// （该常量对 `tools` 模块私有，此处以同一数值表达同一承诺：超过即拒绝，让外部 agent 用
/// `line`/`limit` 分片读，而不是把宿主内存拖爆）。
pub const ACP_FS_READ_MAX_BYTES: u64 = 120_000;

/// `terminal/wait_for_exit` 的轮询切片（≤ NFR `acp.request_timeout_ms` 的预算内快速响应）。
const TERMINAL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// 每个 terminal 的**每流（stdout / stderr 各自）**输出字节上限，缺省值（1 MiB）。
///
/// 语义（B1 复审 BLOCKING）：输出缓冲是**有界头部窗口**——只保留从启动起的前
/// `limit` 字节，超出部分永久丢弃并把对应 `stdout_truncated` / `stderr_truncated` 置真。
/// 没有这条上限时 append-only 累积会被一条 `yes` 这类洪水命令打成无界内存
/// （实测 8s ≈50MB，按 120s 预算外推 ≈760MB/terminal）。请求里的 `outputByteLimit`
/// 可调小/调大，但受 [`ACP_TERMINAL_OUTPUT_BYTE_LIMIT_MAX`] 硬顶约束。
pub const ACP_TERMINAL_OUTPUT_BYTE_LIMIT: usize = 1024 * 1024;

/// `outputByteLimit` 的宿主硬顶（16 MiB/流）：agent 自选值不得把宿主内存当无界缓冲。
pub const ACP_TERMINAL_OUTPUT_BYTE_LIMIT_MAX: usize = 16 * 1024 * 1024;

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
        // ADVISORY③：上限作用于**本次实际返回的内容**——先按 line/limit 流式切片，再判上限；
        // 无 line/limit 的整读仍以文件大小先行短路（避免把超大文件读进内存）。
        if request.line.is_none() && request.limit.is_none() && metadata.len() > ACP_FS_READ_MAX_BYTES {
            return Err(AcpHandlerError::Denied(format!(
                "permission_denied: 文件 {} 字节超过 ACP 读取上限 {ACP_FS_READ_MAX_BYTES} 字节，请用 line/limit 分片读取。",
                metadata.len()
            )));
        }
        read_slice(&canonical, request.line, request.limit, ACP_FS_READ_MAX_BYTES).map_err(
            |error| match error {
                SliceReadError::Io(error) => AcpHandlerError::Internal(format!(
                    "读取文件失败（{}）：{error}。",
                    self.describe_target(&canonical)
                )),
                SliceReadError::NotUtf8 => AcpHandlerError::Internal(format!(
                    "文件不是合法 UTF-8（{}），拒绝以文本读取。",
                    self.describe_target(&canonical)
                )),
                SliceReadError::TooLarge { actual } => AcpHandlerError::Denied(format!(
                    "permission_denied: 请求的行范围返回 {actual} 字节，超过 ACP 读取上限 {ACP_FS_READ_MAX_BYTES} 字节，请缩小 limit。"
                )),
            },
        ).map(|content| ReadTextFileResponse { content })
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

/// 流式行切片读取的失败面。
enum SliceReadError {
    Io(std::io::Error),
    NotUtf8,
    TooLarge { actual: usize },
}

/// 按 `line`（1-based）/`limit` 流式读取，只把**所选行范围**装进内存并对其判上限。
///
/// 语义：既不因为文件整体很大而拒绝合法的分片请求，也不会把整文件读进内存；
/// 返回内容逐字节保留原行（含行尾换行），不做行尾归一。
fn read_slice(
    path: &Path,
    line: Option<u32>,
    limit: Option<u32>,
    max_bytes: u64,
) -> Result<String, SliceReadError> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).map_err(SliceReadError::Io)?;
    let mut reader = std::io::BufReader::new(file);
    let skip = line.unwrap_or(1).max(1) as usize - 1;
    let take = limit.map(|value| value as usize).unwrap_or(usize::MAX);
    let mut selected = String::new();
    let mut index = 0usize;
    let mut collected = 0usize;
    loop {
        let mut line_buffer = String::new();
        let read = reader.read_line(&mut line_buffer).map_err(SliceReadError::Io)?;
        if read == 0 {
            break;
        }
        if index >= skip && collected < take {
            collected += 1;
            if selected.len() + line_buffer.len() > max_bytes as usize {
                return Err(SliceReadError::TooLarge {
                    actual: selected.len() + line_buffer.len(),
                });
            }
            selected.push_str(&line_buffer);
        }
        index += 1;
        if collected >= take {
            break;
        }
    }
    if index == 0 && !selected.is_empty() {
        return Err(SliceReadError::NotUtf8);
    }
    Ok(selected)
}

/// 一个已 `terminal/create` 的受管终端。
struct TerminalEntry {
    /// ProcessManager 的 session 归属（跨 session 句柄一律 fail-closed）。
    session_id: String,
    /// ProcessManager 句柄（wire `terminalId` 是它外面的 opaque 包装，不可伪造）。
    handle: String,
    /// 累积输出缓冲（B1：`wait_for_exit` 的 poll 结果**不得丢弃**，否则 create→wait→output
    /// 的标准流程会返回空；这里累积，`output` 返回全量而非只返回「本片」）。
    ///
    /// 上界见 [`TerminalBuffer::append`]：每流最多 `limit` 字节（有界头部窗口）。
    buffer: Mutex<TerminalBuffer>,
    /// 每流字节上限（`outputByteLimit` 或 [`ACP_TERMINAL_OUTPUT_BYTE_LIMIT`]，硬顶 16 MiB）。
    limit: usize,
}

/// per-terminal 输出累积态。
#[derive(Default)]
struct TerminalBuffer {
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    exit_code: Option<i32>,
}

impl TerminalBuffer {
    /// 追加一片输出：**有界头部窗口**语义。
    ///
    /// - 只 append 剩余额度（`limit - 已缓冲字节`），超出部分丢弃；
    /// - 一旦某流被截断，`*_truncated` 永久置真，后续分片**不再**累加该流（既不会
    ///   「重复累积已丢弃字节」，也不会把缓冲顶破上限）；
    /// - 截断点按 UTF-8 字符边界回退，绝不切出非法字符串（不会 panic）。
    fn append(&mut self, limit: usize, chunk: &str, upstream_truncated: bool, stderr: bool) {
        if chunk.is_empty() {
            return;
        }
        let (buffer, truncated) = if stderr {
            (&mut self.stderr, &mut self.stderr_truncated)
        } else {
            (&mut self.stdout, &mut self.stdout_truncated)
        };
        if *truncated {
            return;
        }
        let remaining = limit.saturating_sub(buffer.len());
        if chunk.len() <= remaining {
            buffer.push_str(chunk);
            if upstream_truncated {
                *truncated = true;
            }
            return;
        }
        buffer.push_str(utf8_prefix(chunk, remaining));
        *truncated = true;
    }
}

/// 取 `text` 的前 `max_bytes` 字节（按 UTF-8 字符边界回退）。
fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// 解析 `outputByteLimit`：缺省 [`ACP_TERMINAL_OUTPUT_BYTE_LIMIT`]；0/非法 ⇒ 明确错误；
/// 超宿主硬顶 ⇒ 截到 [`ACP_TERMINAL_OUTPUT_BYTE_LIMIT_MAX`]（agent 自选值不得无界）。
fn resolve_output_byte_limit(requested: Option<u64>) -> Result<usize, AcpHandlerError> {
    match requested {
        None => Ok(ACP_TERMINAL_OUTPUT_BYTE_LIMIT),
        Some(0) => Err(AcpHandlerError::Denied(
            "permission_denied: terminal/create 的 outputByteLimit 必须是正字节数（0 不是合法上限）。"
                .to_string(),
        )),
        Some(limit) => {
            let limit = usize::try_from(limit).map_err(|_| {
                AcpHandlerError::Denied(
                    "permission_denied: terminal/create 的 outputByteLimit 超出可表示范围。"
                        .to_string(),
                )
            })?;
            Ok(limit.min(ACP_TERMINAL_OUTPUT_BYTE_LIMIT_MAX))
        }
    }
}

/// `terminal/*` 的 ProcessManager 治理实现。
///
/// # 语义
/// - `create` → [`ProcessManager`] 的 `ProcessBackend::start`（`sandbox.isolate_environment =
///   true`；env 只经 `environment_allowlist` 且拒绝凭据/加载器劫持键）；可选 `cwd`（绝对路径
///   校验；若 handler 绑定 workspace root 则必须在 root 内）；返回 opaque `terminalId`
///   （cross-session 伪造 → `NotFound`）。
/// - `output` → `ProcessBackend::poll` **累积**（不丢帧）+ 返回全量 stdout/stderr。
/// - `wait_for_exit` → 有界轮询 `poll` 直到 `Exited` 或预算 [`ACP_DEFAULT_TURN_BUDGET`] 超时；
///   轮询结果同样累积进 per-terminal 缓冲（不吞输出）。
/// - `kill` → `kill_if_exists`（幂等；保留 handle 供最终 `output`）。
/// - `release` → `kill_if_exists` + 条目回收（释放后 handle 不再可用，且无残留子进程）。
/// - **禁用面**：不存在裸 `std::process` 调用；唯一进程编排口是 `ProcessManager`。
pub struct ManagedTerminalHandler {
    /// 会话进程编排器（kill/cancel/kill_after/drain_stats 的既有回收语义）。
    process_manager: Arc<ProcessManager>,
    /// 会话 workspace root（additive；绑定后 `cwd` 必须在其中，未绑定则仅做绝对路径校验）。
    workspace_root: Option<PathBuf>,
    /// wire `terminalId` → 受管句柄（opaque、不可跨会话伪造）。
    terminals: Mutex<HashMap<String, Arc<TerminalEntry>>>,
}

impl ManagedTerminalHandler {
    /// 构造 handler（必须在 `connect_with_capabilities` 前调用）。
    pub fn new(process_manager: Arc<ProcessManager>) -> Self {
        Self {
            process_manager,
            workspace_root: None,
            terminals: Mutex::new(HashMap::new()),
        }
    }

    /// additive：绑定会话 workspace root（`terminal/create.cwd` 的边界判定锚点）。
    pub fn with_workspace_root(process_manager: Arc<ProcessManager>, workspace_root: PathBuf) -> Self {
        Self {
            process_manager,
            workspace_root: Some(std::fs::canonicalize(&workspace_root).unwrap_or(workspace_root)),
            terminals: Mutex::new(HashMap::new()),
        }
    }

    /// 解析 `terminalId`（未知 / 跨 session ⇒ `NotFound`，绝不 panic）。
    fn entry(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<Arc<TerminalEntry>, AcpHandlerError> {
        let terminals = self.terminals.lock().map_err(|_| {
            AcpHandlerError::Internal("terminal 句柄表已中毒，fail-closed。".to_string())
        })?;
        match terminals.get(terminal_id) {
            Some(entry) if entry.session_id == session_id => Ok(Arc::clone(entry)),
            Some(_) => Err(AcpHandlerError::NotFound(format!(
                "terminalId `{terminal_id}` 不属于当前会话（跨 session 伪造已拒绝）。"
            ))),
            None => Err(AcpHandlerError::NotFound(format!(
                "未知 terminalId `{terminal_id}`（未创建或已 release）。"
            ))),
        }
    }

    /// poll 一次并把结果**累积**进 per-terminal 缓冲（B1：绝不 drain 后丢弃），返回进程状态。
    fn poll_into(&self, entry: &TerminalEntry) -> Result<(ProcessState, Option<i32>), AcpHandlerError> {
        let poll = self
            .process_manager
            .poll(&entry.session_id, &entry.handle)
            .map_err(|error| {
                AcpHandlerError::Internal(format!(
                    "terminal 输出轮询失败（{}）：{error}。",
                    entry.handle
                ))
            })?;
        let mut buffer = entry.buffer.lock().map_err(|_| {
            AcpHandlerError::Internal("terminal 输出缓冲已中毒，fail-closed。".to_string())
        })?;
        // 有界头部窗口：超出 `entry.limit` 的部分直接丢弃（不重复累积，不顶破上限）。
        buffer.append(entry.limit, &poll.stdout, poll.stdout_truncated, false);
        buffer.append(entry.limit, &poll.stderr, poll.stderr_truncated, true);
        if let Some(code) = poll.exit_code {
            buffer.exit_code = Some(code);
        }
        Ok((poll.state, buffer.exit_code))
    }

    /// 解析并校验 `terminal/create` 的工作目录（ADVISORY①）。
    fn resolve_cwd(&self, requested: Option<&str>) -> Result<Option<String>, AcpHandlerError> {
        let explicit = requested.map(str::trim).filter(|value| !value.is_empty());
        let candidate = match explicit {
            Some(value) => {
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(AcpHandlerError::Denied(format!(
                        "permission_denied: terminal/create 的 cwd 必须是绝对路径，实际 `{value}`。"
                    )));
                }
                path
            }
            None => match self.workspace_root.clone() {
                Some(root) => root,
                None => return Ok(None),
            },
        };
        let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
            AcpHandlerError::Denied(format!(
                "permission_denied: terminal/create 的 cwd `{}` 不可用（{error}）。",
                candidate.display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(AcpHandlerError::Denied(format!(
                "permission_denied: terminal/create 的 cwd `{}` 不是目录。",
                canonical.display()
            )));
        }
        if let Some(root) = self.workspace_root.as_ref() {
            if !canonical.starts_with(root) {
                return Err(AcpHandlerError::Denied(format!(
                    "outside_workspace_write_denied: terminal/create 的 cwd `{}` 不在会话 workspace `{}` 内。",
                    canonical.display(),
                    root.display()
                )));
            }
        }
        Ok(Some(canonical.to_string_lossy().into_owned()))
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
            // ADVISORY②：键名先归一化再比对 deny 表（含 `NAME=value` 形态的绕过）——
            // 与 `is_denied_spec_env_key` 同一真源，绝不因拼写形态漏放。
            .filter_map(|variable| {
                let name = variable.name.trim();
                let key = name.split('=').next().unwrap_or(name).trim();
                if key.is_empty() || name.contains('=') || crate::agent::mcp_client::is_denied_spec_env_key(key) {
                    eprintln!(
                        "[acp][terminal] 丢弃 env 条目 `{}`（空键 / 含 `=` 的非法键 / 命中 deny 表）",
                        key
                    );
                    return None;
                }
                Some(format!("{name}={}", variable.value))
            })
            .collect::<Vec<_>>();
        let output_byte_limit = resolve_output_byte_limit(request.output_byte_limit)?;
        let cwd = self.resolve_cwd(request.cwd.as_deref())?;
        let start = ProcessStartRequest {
            session_id: request.session_id.clone(),
            program: request.command.clone(),
            arguments: request.args.clone().unwrap_or_default(),
            cwd,
            sandbox: SandboxRequest {
                workspace_root: self
                    .workspace_root
                    .as_ref()
                    .map(|root| root.display().to_string())
                    .or_else(|| request.cwd.clone())
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
            Arc::new(TerminalEntry {
                session_id: request.session_id.clone(),
                handle,
                buffer: Mutex::new(TerminalBuffer::default()),
                limit: output_byte_limit,
            }),
        );
        Ok(CreateTerminalResponse { terminal_id })
    }

    fn output(
        &self,
        request: &TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, AcpHandlerError> {
        let entry = self.entry(&request.session_id, &request.terminal_id)?;
        // 先 drain 一次（增量追加进**有界**缓冲），再返回累积内容：create→wait→output 也有内容，
        // 且返回量恒 ≤ 每流 `limit`（不会 clone 无界历史）。
        let (_, exit_code) = self.poll_into(&entry)?;
        let buffer = entry.buffer.lock().map_err(|_| {
            AcpHandlerError::Internal("terminal 输出缓冲已中毒，fail-closed。".to_string())
        })?;
        let mut output = buffer.stdout.clone();
        output.push_str(&buffer.stderr);
        Ok(TerminalOutputResponse {
            output,
            truncated: buffer.stdout_truncated || buffer.stderr_truncated,
            exit_status: exit_code.map(|code| TerminalExitStatus {
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
        // B4：预算取 NFR 派生的整轮预算（不是传输层单请求 5s）；且入站分发已移出读线程
        // （transport 专用分发线程），这里的阻塞等待不再拖住其它入站帧的读取。
        let deadline = Instant::now() + ACP_DEFAULT_TURN_BUDGET;
        loop {
            let (state, exit_code) = self.poll_into(&entry)?;
            if state == ProcessState::Exited {
                return Ok(WaitForTerminalExitResponse {
                    exit_code,
                    signal: None,
                });
            }
            if Instant::now() >= deadline {
                // 有界等待：不无限阻塞；超时明确报错（仍可 kill/release）。
                return Err(AcpHandlerError::Internal(format!(
                    "terminal/wait_for_exit 超过 {:?} 仍未退出（terminalId 仍可 kill/release）。",
                    ACP_DEFAULT_TURN_BUDGET
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
