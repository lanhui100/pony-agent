//! ACP 会话客户端（Pony as ACP Client，P1 空桩）。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2 / §4 P1 表。
//! 分层：本层建立在已实现的 [`crate::acp::transport::AcpTransport`] 之上，只做「一条 ACP 会话」
//! 的协议编排（`initialize` → `session/new` → `session/prompt` → `session/cancel`），不做事件
//! 映射 / 治理归约（那是 [`crate::acp::session`]）。
//!
//! **协议版本硬约束（fail-closed）**：`initialize` 响应里的 `protocolVersion` 若不是整数 `1`
//! （§4 P0：`protocolVersion` 用 `u32`），`connect` 必须返回
//! [`AcpTransportError::Protocol`]（错误码 `acp_protocol_error`），**禁止**降级继续用。
//!
//! P1 只冻结签名：本文件所有函数体 `unimplemented!()`，由 executor 实现；禁止在此落业务逻辑。

#![allow(unused_variables)]

use std::path::Path;
use std::sync::Arc;

use crate::acp::config::AcpAgentSpec;
use crate::acp::protocol::{
    AgentCapabilities, ContentBlock, CreateTerminalRequest, CreateTerminalResponse, McpServer,
    PromptResponse, ReadTextFileRequest, ReadTextFileResponse, RequestPermissionRequest,
    RequestPermissionResponse, SessionNewResponse, TerminalOutputResponse,
    WaitForTerminalExitRequest, WaitForTerminalExitResponse, WriteTextFileRequest,
    WriteTextFileResponse,
};
use crate::acp::transport::{AcpNotificationHandler, AcpTransport, AcpTransportError};

/// 入站 handler 的统一失败形态（fs/* / terminal/* 用；请求侧回 JSON-RPC error）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpHandlerError {
    /// 治理链拒绝（路径越界、无授权、审批拒绝）。
    Denied(String),
    /// 目标不存在（session / terminal / 文件）。
    NotFound(String),
    /// handler 内部错误（不向 agent 泄漏宿主细节）。
    Internal(String),
}

/// `session/request_permission` 的宿主应答器（**唯一审批交汇点**，§3.2.3）。
///
/// 实现必须走 `ask_control` 往返让用户决定；**fail-closed 硬约束**：无 handler / 未决即取消时
/// 返回 `cancelled`，任何情况下不得默认 `allow`（§6 风险表第一条）。
pub trait PermissionHandler: Send + Sync {
    fn decide(&self, request: &RequestPermissionRequest) -> RequestPermissionResponse;
}

/// `fs/*` 反向请求的宿主实现（§3.2.4：读走 `classify_path` + `AuthorizeStore`，写走 workspace 边界）。
pub trait FsHandler: Send + Sync {
    fn read_text_file(
        &self,
        request: &ReadTextFileRequest,
    ) -> Result<ReadTextFileResponse, AcpHandlerError>;

    fn write_text_file(
        &self,
        request: &WriteTextFileRequest,
    ) -> Result<WriteTextFileResponse, AcpHandlerError>;
}

/// `terminal/*` 反向请求的宿主实现（§3.2.5：落 `agent::process::ProcessManager`）。
pub trait TerminalHandler: Send + Sync {
    fn create(
        &self,
        request: &CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse, AcpHandlerError>;

    fn output(
        &self,
        request: &crate::acp::protocol::TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, AcpHandlerError>;

    fn wait_for_exit(
        &self,
        request: &WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse, AcpHandlerError>;

    fn kill(
        &self,
        request: &crate::acp::protocol::KillTerminalRequest,
    ) -> Result<(), AcpHandlerError>;

    fn release(
        &self,
        request: &crate::acp::protocol::ReleaseTerminalRequest,
    ) -> Result<(), AcpHandlerError>;
}

/// 一条已握手的 ACP 会话客户端。
#[allow(dead_code)]
pub struct AcpAgentClient {
    transport: AcpTransport,
    /// 清单 id（`acp_delegate` 的 `agent` 参数解析结果）。
    agent_id: String,
    /// `session/new` 返回的会话 id（握手完成前为 `None`）。
    session_id: Option<String>,
    /// `initialize` 协商后的 agent 能力（缺字段按 `false`，§6 风险表）。
    agent_capabilities: AgentCapabilities,
}

impl AcpAgentClient {
    /// 启动子进程并完成 `initialize` 握手。
    ///
    /// 行为契约：
    /// 1. `AcpTransport::spawn(spec, cwd, &[])`（env 白名单 / 进程组 / stderr 环形缓冲由 transport 收口）；
    /// 2. 发送 `initialize { protocolVersion: 1, clientCapabilities, clientInfo }`，能力按
    ///    `spec.capabilities_hint` 声明（`fs`/`terminal` 未声明 = 整块不下放）；
    /// 3. 校验响应 `protocolVersion == 1`，不等即 `Err(AcpTransportError::Protocol(..))`
    ///    （`acp_protocol_error`），**不重试、不降级**。
    pub fn connect(spec: &AcpAgentSpec, cwd: &Path) -> Result<Self, AcpTransportError> {
        unimplemented!("P1：spawn transport + initialize + protocolVersion==1 校验")
    }

    /// 创建会话：`session/new { cwd, mcpServers }` → `sessionId`。
    ///
    /// `cwd` 必须是 canonicalize 后的绝对路径（transport 已做绝对路径校验，§3.2.2）。
    pub fn new_session(
        &mut self,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
    ) -> Result<SessionNewResponse, AcpTransportError> {
        unimplemented!("P1：session/new 请求 + 记录 sessionId")
    }

    /// 发送一轮 prompt 并返回 agent 的停止原因。
    ///
    /// `session/update` 通知在 prompt 期间经 `on_update` 观测（事件映射由 `acp::session` 完成）；
    /// 请求超时预算固定为 `transport::ACP_REQUEST_TIMEOUT`（5000ms）。
    pub fn prompt(
        &self,
        session_id: &str,
        blocks: Vec<ContentBlock>,
        on_update: Arc<dyn AcpNotificationHandler>,
    ) -> Result<PromptResponse, AcpTransportError> {
        unimplemented!("P1：session/prompt + 注入 update 观测 handler + 等待 stopReason")
    }

    /// 取消当前轮：发送 `session/cancel` **notification**（无 id），并让挂起权限请求回 `cancelled`。
    pub fn cancel(&self, session_id: &str) -> Result<(), AcpTransportError> {
        unimplemented!("P1：notify(session/cancel) + 取消挂起 request_permission")
    }

    /// 注入三类反向请求 handler（权限 / 文件系统 / 终端）；未注入即整块 fail-closed。
    pub fn install_handlers(
        &self,
        permission: Arc<dyn PermissionHandler>,
        fs: Option<Arc<dyn FsHandler>>,
        terminal: Option<Arc<dyn TerminalHandler>>,
    ) {
        unimplemented!("P1：写 transport 的入站请求分发槽位（permission/fs/terminal 三路）")
    }

    /// 关闭会话与子进程（幂等；顺序由 `AcpTransport::close` 收口）。
    pub fn close(&mut self) {
        unimplemented!("P1：transport.close()")
    }
}
