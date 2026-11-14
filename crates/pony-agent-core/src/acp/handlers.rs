//! ACP 反向请求的宿主 handler 骨架（B175）：`fs/*` 与 `terminal/*` 的治理实现入口。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2.4（fs → `path_permission`）/
//! §3.2.5（terminal → `ProcessManager`）/ §4 P1 表。
//!
//! **只写骨架 + 签名 + 语义文档**：方法体一律 `unimplemented!()`，由 executor 落地。
//! 装配约束：这两个 handler 必须在 `connect_with_capabilities` **之前**构造好，其存在性决定
//! `initialize` 的 `clientCapabilities.fs/terminal` 声明（见
//! [`crate::acp::session::capabilities_for`]）；未声明能力时 agent 不会发对应反向请求。

// P1 骨架：方法体统一 `unimplemented!()`，形参「未使用」属预期；executor 实现时移除本允许。
#![allow(unused_variables)]

use std::path::PathBuf;
use std::sync::Arc;

use crate::acp::client::{AcpHandlerError, FsHandler, TerminalHandler};
use crate::acp::protocol::{
    CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest, ReadTextFileRequest,
    ReadTextFileResponse, ReleaseTerminalRequest, TerminalOutputRequest, TerminalOutputResponse,
    WaitForTerminalExitRequest, WaitForTerminalExitResponse, WriteTextFileRequest,
    WriteTextFileResponse,
};
use crate::agent::path_permission::{AuthorizeStore, PathPermissionChecker, PathPurpose};
use crate::agent::process::ProcessManager;

/// `fs/read_text_file` / `fs/write_text_file` 的 workspace 治理实现骨架。
///
/// # 语义（executor 实现时必须逐条兑现）
/// - **read**：`path_checker.classify(path, workspace_root, tmp_dir, &authorize_store, PathPurpose::Read)`
///   （内部按 purpose 调 `canonicalize_for_read`；相对路径按 workspace root 解析）；`AuthorizedExternal`
///   可放行、`RequiresAuthorization` 必须 fail-closed；随后按 `line`/`limit`（1-based）读切片。
/// - **write**：`classify(..., PathPurpose::Write)` → 内部走 `canonicalize_for_write`（词法拒 `..`
///   组件与绝对逃逸 + 最近存在祖先的 canonicalize 校验）；`OutsideWorkspaceWriteDenied` 与
///   `PermissionDenied` 一律 fail-closed；绝不 `create_dir_all` 到 workspace 外。
/// - **错误**：治理拒绝 → [`AcpHandlerError::Denied`]（消息带 `permission_error.code` 与只读文案）；
///   文件不存在 → [`AcpHandlerError::NotFound`]；I/O 失败 → [`AcpHandlerError::Internal`]（不向
///   agent 泄漏宿主绝对路径细节，只给 root 相对形态）。
/// - **禁用面**：不得裸 `std::fs` 绕过 `PathPermissionChecker`；不得跟随符号链接逃逸（由
///   canonicalize 收口）。
#[allow(dead_code)]
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
        unimplemented!(
            "B175：WorkspaceFsHandler 构造（tmp_dir = workspace_root/.tmp + with_default checker）"
        )
    }
}

impl FsHandler for WorkspaceFsHandler {
    fn read_text_file(
        &self,
        request: &ReadTextFileRequest,
    ) -> Result<ReadTextFileResponse, AcpHandlerError> {
        unimplemented!("B175：classify(Read) → canonical → 按 line/limit 切片读取")
    }

    fn write_text_file(
        &self,
        request: &WriteTextFileRequest,
    ) -> Result<WriteTextFileResponse, AcpHandlerError> {
        unimplemented!("B175：classify(Write) → canonicalize_for_write 边界校验 → 原子写")
    }
}

/// `terminal/*` 的 ProcessManager 治理实现骨架。
///
/// # 语义（executor 实现时必须逐条兑现）
/// - `create` → [`ProcessManager`] 的 `ProcessBackend::start`（`ProcessStartRequest{session_id,
///   program, arguments, sandbox}`；`sandbox.isolate_environment = true`，env 只经
///   `environment_allowlist` 键名白名单）；返回 handle 即 wire `terminalId`。
/// - `output` → `ProcessBackend::poll`（`stdout`+`stderr` 按到达顺序拼接、`truncated` 取
///   `stdout_truncated || stderr_truncated`，`exitStatus` 由 `ProcessState/exit_code` 归约）。
/// - `wait_for_exit` → 有界轮询 `poll` 直到 `Exited` 或超时（超时 = `AcpHandlerError::Internal`，
///   不无限阻塞读线程）。
/// - `kill` → `kill_if_exists`（幂等；保留 handle 供最终 `output`）。
/// - `release` → `kill_if_exists` + 条目回收（`ProcessManager` 内部清理；释放后 handle 不再可用）。
/// - **禁用面**：不得出现裸 `std::process::Command`/`Child`（唯一进程编排口是 `ProcessManager`）；
///   `terminalId` 必须是不可伪造的 opaque handle，跨 session 一律 fail-closed（`NotFound`）。
#[allow(dead_code)]
pub struct ManagedTerminalHandler {
    /// 会话进程编排器（kill/cancel/kill_after/drain_stats 的既有回收语义）。
    process_manager: Arc<ProcessManager>,
}

impl ManagedTerminalHandler {
    /// 构造 handler（必须在 `connect_with_capabilities` 前调用）。
    pub fn new(process_manager: Arc<ProcessManager>) -> Self {
        unimplemented!("B175：ManagedTerminalHandler 构造")
    }
}

impl TerminalHandler for ManagedTerminalHandler {
    fn create(
        &self,
        request: &CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse, AcpHandlerError> {
        unimplemented!("B175：ProcessBackend::start → handle 作为 terminalId")
    }

    fn output(
        &self,
        request: &TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, AcpHandlerError> {
        unimplemented!("B175：ProcessBackend::poll → stdout/stderr + exitStatus/truncated")
    }

    fn wait_for_exit(
        &self,
        request: &WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse, AcpHandlerError> {
        unimplemented!("B175：有界轮询 poll 直到 Exited/timeout → exitCode/signal")
    }

    fn kill(&self, request: &KillTerminalRequest) -> Result<(), AcpHandlerError> {
        unimplemented!("B175：kill_if_exists（幂等，保留 handle）")
    }

    fn release(&self, request: &ReleaseTerminalRequest) -> Result<(), AcpHandlerError> {
        unimplemented!("B175：kill_if_exists + 条目回收（无残留子进程）")
    }
}
