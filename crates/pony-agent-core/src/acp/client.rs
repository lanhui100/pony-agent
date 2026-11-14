//! ACP 会话客户端（Pony as ACP Client）。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2 / §4 P1 表。
//! 分层：本层建立在 [`crate::acp::transport::AcpTransport`] 之上，只做「一条 ACP 会话」
//! 的协议编排（`initialize` → `session/new` → `session/prompt` → `session/cancel`），不做事件
//! 映射 / 治理归约（那是 [`crate::acp::session`]）。
//!
//! **协议版本硬约束（fail-closed）**：`initialize` 响应里的 `protocolVersion` 若不是整数 `1`
//! （§4 P0：`protocolVersion` 用 `u32`），`connect` 必须返回
//! [`AcpTransportError::Protocol`]（错误码 `acp_protocol_error`），**禁止**降级继续用。
//!
//! 失败语义（禁吞异常）：所有失败都保留 `acp_*` 错误码分源；反向请求（permission/fs/terminal）
//! 未注入 handler 时一律 fail-closed（权限回 `cancelled`、其余回 JSON-RPC 错误），绝不默认放行。

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::acp::config::AcpAgentSpec;
use crate::acp::protocol::{
    method, AgentCapabilities, AuthCapabilities, CancelNotification, ClientCapabilities,
    ContentBlock, CreateTerminalRequest, CreateTerminalResponse, FileSystemCapabilities,
    Implementation, InitializeRequest, InitializeResponse, KillTerminalRequest, McpServer,
    PromptRequest, PromptResponse, ReadTextFileRequest, ReadTextFileResponse,
    ReleaseTerminalRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SessionNewRequest, SessionNewResponse, TerminalOutputRequest,
    TerminalOutputResponse, WaitForTerminalExitRequest, WaitForTerminalExitResponse,
    WriteTextFileRequest, WriteTextFileResponse, PROTOCOL_VERSION,
};
use crate::acp::transport::{
    AcpInboundRequest, AcpInboundRequestHandler, AcpInboundResponse, AcpNotificationHandler,
    AcpTransport, AcpTransportError, ACP_REQUEST_TIMEOUT,
};

/// JSON-RPC「方法不存在 / 能力未下放」错误码。
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC「无效参数」错误码。
const JSONRPC_INVALID_PARAMS: i64 = -32602;
/// JSON-RPC「内部错误」错误码。
const JSONRPC_INTERNAL_ERROR: i64 = -32603;
/// 治理链拒绝（宿主授权不足）。
const JSONRPC_PERMISSION_DENIED: i64 = -32001;
/// 目标不存在。
const JSONRPC_NOT_FOUND: i64 = -32002;

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

impl AcpHandlerError {
    /// 归约为 JSON-RPC 错误码（保留来源，不塌缩成单一错误）。
    fn json_rpc_code(&self) -> i64 {
        match self {
            Self::Denied(_) => JSONRPC_PERMISSION_DENIED,
            Self::NotFound(_) => JSONRPC_NOT_FOUND,
            Self::Internal(_) => JSONRPC_INTERNAL_ERROR,
        }
    }

    /// 面向 agent 的错误文案（只给分类 + 消息，不泄漏宿主内部细节）。
    fn message(&self) -> String {
        match self {
            Self::Denied(message) => format!("acp_handler_denied: {message}"),
            Self::NotFound(message) => format!("acp_handler_not_found: {message}"),
            Self::Internal(message) => format!("acp_handler_internal: {message}"),
        }
    }
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
        request: &TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, AcpHandlerError>;

    fn wait_for_exit(
        &self,
        request: &WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse, AcpHandlerError>;

    fn kill(&self, request: &KillTerminalRequest) -> Result<(), AcpHandlerError>;

    fn release(&self, request: &ReleaseTerminalRequest) -> Result<(), AcpHandlerError>;
}

/// 反向请求处理器的可热替换槽位。
///
/// `install_handlers(&self, ..)` 是冻结签名（`&self`），而 `AcpTransport::set_handler` 需要
/// `&mut self`：因此 `connect` 一次性装入一个**转发壳**，壳内部经本槽位读取真实 handler。
/// 未注入的槽位即 fail-closed（不默认放行）。
#[derive(Default)]
struct HandlerSlots {
    permission: Option<Arc<dyn PermissionHandler>>,
    fs: Option<Arc<dyn FsHandler>>,
    terminal: Option<Arc<dyn TerminalHandler>>,
}

/// `connect` 装入 transport 的入站请求分发器：按 method 路由到 handler 槽位。
struct DelegatingRequestHandler {
    slots: Arc<Mutex<HandlerSlots>>,
}

impl DelegatingRequestHandler {
    fn slot_snapshot(
        &self,
    ) -> (
        Option<Arc<dyn PermissionHandler>>,
        Option<Arc<dyn FsHandler>>,
        Option<Arc<dyn TerminalHandler>>,
    ) {
        let slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            slots.permission.clone(),
            slots.fs.clone(),
            slots.terminal.clone(),
        )
    }
}

impl AcpInboundRequestHandler for DelegatingRequestHandler {
    fn handle(&self, request: AcpInboundRequest) -> AcpInboundResponse {
        let (permission, fs, terminal) = self.slot_snapshot();
        match request.method.as_str() {
            // §6 硬约束：无 handler ⇒ 一律 cancelled（绝不 default allow）。
            method::SESSION_REQUEST_PERMISSION => match permission {
                Some(handler) => match from_params::<RequestPermissionRequest>(&request.params) {
                    Ok(parsed) => match serde_json::to_value(handler.decide(&parsed)) {
                        Ok(value) => AcpInboundResponse::Result(value),
                        Err(error) => internal_error(format!("序列化权限应答失败：{error}")),
                    },
                    Err(error) => invalid_params(&request.method, error),
                },
                None => {
                    cancelled_permission_response("未注入 PermissionHandler，权限请求 fail-closed");
                    match serde_json::to_value(RequestPermissionResponse {
                        outcome: RequestPermissionOutcome::Cancelled,
                    }) {
                        Ok(value) => AcpInboundResponse::Result(value),
                        Err(error) => internal_error(format!("构造权限取消应答失败：{error}")),
                    }
                }
            },
            method::FS_READ_TEXT_FILE => match fs {
                Some(handler) => handler_dispatch(&request, |parsed: &ReadTextFileRequest| {
                    handler.read_text_file(parsed).and_then(to_value)
                }),
                None => capability_not_declared(&request.method),
            },
            method::FS_WRITE_TEXT_FILE => match fs {
                Some(handler) => handler_dispatch(&request, |parsed: &WriteTextFileRequest| {
                    handler.write_text_file(parsed).and_then(to_value)
                }),
                None => capability_not_declared(&request.method),
            },
            method::TERMINAL_CREATE => match terminal {
                Some(handler) => handler_dispatch(&request, |parsed: &CreateTerminalRequest| {
                    handler.create(parsed).and_then(to_value)
                }),
                None => capability_not_declared(&request.method),
            },
            method::TERMINAL_OUTPUT => match terminal {
                Some(handler) => handler_dispatch(&request, |parsed: &TerminalOutputRequest| {
                    handler.output(parsed).and_then(to_value)
                }),
                None => capability_not_declared(&request.method),
            },
            method::TERMINAL_WAIT_FOR_EXIT => match terminal {
                Some(handler) => {
                    handler_dispatch(&request, |parsed: &WaitForTerminalExitRequest| {
                        handler.wait_for_exit(parsed).and_then(to_value)
                    })
                }
                None => capability_not_declared(&request.method),
            },
            method::TERMINAL_KILL => match terminal {
                Some(handler) => handler_dispatch(&request, |parsed: &KillTerminalRequest| {
                    handler.kill(parsed).map(|()| json!({}))
                }),
                None => capability_not_declared(&request.method),
            },
            method::TERMINAL_RELEASE => match terminal {
                Some(handler) => handler_dispatch(&request, |parsed: &ReleaseTerminalRequest| {
                    handler.release(parsed).map(|()| json!({}))
                }),
                None => capability_not_declared(&request.method),
            },
            other => AcpInboundResponse::Error {
                code: JSONRPC_METHOD_NOT_FOUND,
                message: format!("acp client 不实现方法 `{other}`（fail-closed）。"),
                data: Some(json!({ "code": "acp_unavailable" })),
            },
        }
    }
}

/// 序列化 handler 的应答（失败按 Internal 上报，不静默产出 `null`）。
fn to_value<T: serde::Serialize>(value: T) -> Result<Value, AcpHandlerError> {
    serde_json::to_value(value)
        .map_err(|error| AcpHandlerError::Internal(format!("序列化响应失败：{error}")))
}

fn from_params<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, String> {
    serde_json::from_value(params.clone()).map_err(|error| error.to_string())
}

/// fs/terminal 统一分发：解析参数 → handler → 结果/错误（错误码按 [`AcpHandlerError`] 分源）。
fn handler_dispatch<T, F>(request: &AcpInboundRequest, call: F) -> AcpInboundResponse
where
    T: for<'de> Deserialize<'de>,
    F: FnOnce(&T) -> Result<Value, AcpHandlerError>,
{
    match from_params::<T>(&request.params) {
        Ok(parsed) => match call(&parsed) {
            Ok(result) => AcpInboundResponse::Result(result),
            Err(error) => AcpInboundResponse::Error {
                code: error.json_rpc_code(),
                message: error.message(),
                data: Some(json!({ "method": request.method })),
            },
        },
        Err(error) => invalid_params(&request.method, error),
    }
}

/// 权限 fail-closed 诊断（只写进程 stderr，绝不污染协议 stdout）。
fn cancelled_permission_response(reason: &str) {
    eprintln!("[acp] {reason}");
}

fn invalid_params(method: &str, error: String) -> AcpInboundResponse {
    AcpInboundResponse::Error {
        code: JSONRPC_INVALID_PARAMS,
        message: format!("`{method}` 参数不符合 ACP v1 schema：{error}"),
        data: None,
    }
}

fn internal_error(message: String) -> AcpInboundResponse {
    AcpInboundResponse::Error {
        code: JSONRPC_INTERNAL_ERROR,
        message,
        data: None,
    }
}

fn capability_not_declared(method: &str) -> AcpInboundResponse {
    AcpInboundResponse::Error {
        code: JSONRPC_METHOD_NOT_FOUND,
        message: format!(
            "acp_unavailable: 宿主未声明对应能力，方法 `{method}` 不可用（fail-closed）。"
        ),
        data: Some(json!({ "code": "acp_unavailable" })),
    }
}

/// 一条已握手的 ACP 会话客户端。
pub struct AcpAgentClient {
    transport: AcpTransport,
    /// 清单 id（`acp_delegate` 的 `agent` 参数解析结果）。
    agent_id: String,
    /// `session/new` 返回的会话 id（握手完成前为 `None`）。
    session_id: Option<String>,
    /// `initialize` 协商后的 agent 能力（缺字段按 `false`，§6 风险表）。
    agent_capabilities: AgentCapabilities,
    /// 反向请求 handler 槽位（`install_handlers(&self)` 的热替换面，见 [`HandlerSlots`]）。
    handlers: Arc<Mutex<HandlerSlots>>,
    /// 单轮 `session/prompt` 的出站请求预算（默认 [`ACP_REQUEST_TIMEOUT`]）。
    ///
    /// 由 [`AcpAgentClient::set_prompt_budget`] 写入：整轮委派预算（`timeoutMs` /
    /// `--timeout-secs` / 执行策略）只影响本字段，**不是**固定 5000ms 硬顶。
    prompt_budget: Mutex<Duration>,
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
        let mut transport = AcpTransport::spawn(spec, cwd, &[])?;

        // 反向请求转发壳必须在首次 `initialize` 应答前就位：agent 可能在 initialize 期间就发
        // `session/request_permission`（fixture 的 `permission` 模式即如此）。
        let handlers = Arc::new(Mutex::new(HandlerSlots::default()));
        transport.set_handler(Arc::new(DelegatingRequestHandler {
            slots: Arc::clone(&handlers),
        }));

        let capabilities = ClientCapabilities {
            // §3.2.4：未声明 = 整块不下放（外部 agent 只能在自己子进程里读写）。
            fs: spec.capabilities_hint.fs.then(|| FileSystemCapabilities {
                read_text_file: true,
                write_text_file: true,
            }),
            // §3.2.5：终端能力同理（未声明时 agent 不应发 terminal/*）。
            terminal: spec.capabilities_hint.terminal.then_some(true),
            auth: Some(AuthCapabilities { terminal: false }),
        };
        let request = InitializeRequest {
            protocol_version: PROTOCOL_VERSION,
            client_capabilities: capabilities,
            client_info: Some(Implementation {
                name: "pony-agent".to_string(),
                title: Some("Pony Agent".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
            }),
        };
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 initialize 请求失败：{error}。"))
        })?;
        let result = transport.request(method::INITIALIZE, params, ACP_REQUEST_TIMEOUT)?;
        let response: InitializeResponse = serde_json::from_value(result).map_err(|error| {
            AcpTransportError::Protocol(format!("initialize 响应不符合 ACP v1 schema：{error}。"))
        })?;
        if response.protocol_version != PROTOCOL_VERSION {
            // 不降级继续用：协议版本不同 = 会话不可信（fail-closed，不重试）。
            transport.close();
            return Err(AcpTransportError::Protocol(format!(
                "acp_protocol_error: agent 回报 protocolVersion={}，客户端只支持整数 {PROTOCOL_VERSION}。",
                response.protocol_version
            )));
        }

        Ok(Self {
            transport,
            agent_id: spec.id.clone(),
            session_id: None,
            agent_capabilities: response.agent_capabilities,
            handlers,
            prompt_budget: Mutex::new(ACP_REQUEST_TIMEOUT),
        })
    }

    /// 创建会话：`session/new { cwd, mcpServers }` → `sessionId`。
    ///
    /// `cwd` 必须是 canonicalize 后的绝对路径（transport 已做绝对路径校验，§3.2.2）。
    pub fn new_session(
        &mut self,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
    ) -> Result<SessionNewResponse, AcpTransportError> {
        if !cwd.is_absolute() {
            // 不静默 canonicalize 相对路径：调用方必须给绝对路径（§3.2.2）。
            return Err(AcpTransportError::Spawn(format!(
                "session/new 的 cwd 必须是绝对路径，实际 `{}`。",
                cwd.display()
            )));
        }
        let canonical = std::fs::canonicalize(cwd).map_err(|error| {
            AcpTransportError::Spawn(format!(
                "session/new 的 cwd `{}` 不可用（{error}）。",
                cwd.display()
            ))
        })?;
        let request = SessionNewRequest {
            cwd: canonical.to_string_lossy().into_owned(),
            mcp_servers,
            additional_directories: None,
        };
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 session/new 请求失败：{error}。"))
        })?;
        let result = self
            .transport
            .request(method::SESSION_NEW, params, ACP_REQUEST_TIMEOUT)?;
        let response: SessionNewResponse = serde_json::from_value(result).map_err(|error| {
            AcpTransportError::Protocol(format!("session/new 响应不符合 v1 schema：{error}。"))
        })?;
        self.session_id = Some(response.session_id.clone());
        Ok(response)
    }

    /// 设置单轮 `session/prompt` 的出站请求预算（additive；`&self` 热替换）。
    ///
    /// 语义：
    /// - 默认值 = [`ACP_REQUEST_TIMEOUT`]（5000ms，stdio 单请求基准）；
    /// - 整轮委派预算（`timeoutMs` / `--timeout-secs` / 执行策略）经本方法写入后，`prompt`
    ///   用它作为 `transport.request` 的 deadline，**不再**固定 5000ms 硬顶；
    /// - 零值/极小值由 transport 侧 `MIN_REQUEST_TIMEOUT` 兜底（不在此处改写语义）。
    pub fn set_prompt_budget(&self, budget: std::time::Duration) {
        if let Ok(mut slot) = self.prompt_budget.lock() {
            *slot = budget;
        }
    }

    /// 发送一轮 prompt 并返回 agent 的停止原因。
    ///
    /// `session/update` 通知在 prompt 期间经 `on_update` 观测（事件映射由 `acp::session` 完成）；
    /// deadline 取 [`AcpAgentClient::set_prompt_budget`] 的当前值（默认
    /// [`ACP_REQUEST_TIMEOUT`]），而非固定 5000ms。
    pub fn prompt(
        &self,
        session_id: &str,
        blocks: Vec<ContentBlock>,
        on_update: Arc<dyn AcpNotificationHandler>,
    ) -> Result<PromptResponse, AcpTransportError> {
        // 观测 handler 先就位再发请求：否则首条 update 会落在空槽位上被丢弃。
        self.transport.set_notification_handler(on_update);
        let request = PromptRequest {
            session_id: session_id.to_string(),
            prompt: blocks,
        };
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 session/prompt 请求失败：{error}。"))
        })?;
        let budget = self
            .prompt_budget
            .lock()
            .map(|budget| *budget)
            .unwrap_or(ACP_REQUEST_TIMEOUT);
        let result = self
            .transport
            .request(method::SESSION_PROMPT, params, budget)?;
        serde_json::from_value(result).map_err(|error| {
            AcpTransportError::Protocol(format!(
                "session/prompt 响应不符合 v1 schema（缺 stopReason）：{error}。"
            ))
        })
    }

    /// 取消当前轮：发送 `session/cancel` **notification**（无 id），并让挂起权限请求回 `cancelled`。
    pub fn cancel(&self, session_id: &str) -> Result<(), AcpTransportError> {
        let params = serde_json::to_value(CancelNotification {
            session_id: session_id.to_string(),
        })
        .map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 session/cancel 通知失败：{error}。"))
        })?;
        // 冻结契约（A2）：cancel 是 notification，绝不携带 id。
        self.transport.notify(method::SESSION_CANCEL, params)
    }

    /// 注入三类反向请求 handler（权限 / 文件系统 / 终端）；未注入即整块 fail-closed。
    pub fn install_handlers(
        &self,
        permission: Arc<dyn PermissionHandler>,
        fs: Option<Arc<dyn FsHandler>>,
        terminal: Option<Arc<dyn TerminalHandler>>,
    ) {
        let mut slots = self
            .handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        slots.permission = Some(permission);
        slots.fs = fs;
        slots.terminal = terminal;
    }

    /// 关闭会话与子进程（幂等；顺序由 [`AcpTransport::close`] 收口）。
    pub fn close(&mut self) {
        self.transport.close();
    }

    /// 清单 id（诊断 / 日志用）。
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// 当前会话 id（`session/new` 之后才有值）。
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// `initialize` 协商到的 agent 能力（缺字段按 `false`）。
    pub fn agent_capabilities(&self) -> &AgentCapabilities {
        &self.agent_capabilities
    }
}
