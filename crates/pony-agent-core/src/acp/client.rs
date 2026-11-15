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
    CloseSessionRequest, ContentBlock, CreateTerminalRequest, CreateTerminalResponse,
    FileSystemCapabilities, Implementation, InitializeRequest, InitializeResponse,
    KillTerminalRequest, ListSessionsRequest, ListSessionsResponse, LoadSessionRequest,
    LoadSessionResponse, McpServer, PromptRequest, PromptResponse, ReadTextFileRequest,
    ReadTextFileResponse, ReleaseTerminalRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SessionNewRequest, SessionNewResponse,
    TerminalOutputRequest, TerminalOutputResponse, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse, PROTOCOL_VERSION,
};
use crate::acp::transport::{
    AcpInboundRequest, AcpInboundRequestHandler, AcpInboundResponse, AcpNotificationHandler,
    AcpTransport, AcpTransportError, ACP_HANDSHAKE_TIMEOUT, ACP_REQUEST_TIMEOUT,
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
// 注：`std::fmt::Debug` supertrait 是为了让 `DispatchContext`（含 `Arc<dyn PermissionHandler>`）
// 能继续 `#[derive(Debug)]`；handler 的 Debug 输出只打角色名，不泄漏治理内部状态。
pub trait PermissionHandler: Send + Sync + std::fmt::Debug {
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
    /// B175/B3：本次会话在 `initialize` 里**已声明**的能力。装配与反向请求分发都以它为闸门：
    /// 未声明的能力即使被 `install_*` 注入也一律 fail-closed（不追认、不执行）。
    declared: AcpClientCapabilities,
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
        // B3 fail-closed 闸门：未在 initialize 声明的能力，即使槽位被注入 handler 也不服务
        // 反向请求（否则「caps=false 却真实执行 fs/terminal」与声明面自相矛盾）。
        let fs = if slots.declared.fs {
            slots.fs.clone()
        } else {
            None
        };
        let terminal = if slots.declared.terminal {
            slots.terminal.clone()
        } else {
            None
        };
        (slots.permission.clone(), fs, terminal)
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

/// `initialize` 时声明的客户端能力（B175）。
///
/// `initialize` 是握手第一条 ⇒ 能力声明必须**在 `connect` 之前**决定（不能"先握手后装 handler"）。
/// 声明硬约束：传入本结构的能力必须 **≤ 实际会装配的 handler**；不得谎报（谎报会让外部 agent
/// 发来宿主没有治理链承载的 `fs/*` / `terminal/*` 请求）。`spec.capabilities_hint` 只作为上限，
/// 由 [`crate::acp::session::AcpAgentSession::capabilities_for`] 与本结构取交集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AcpClientCapabilities {
    /// 声明 `clientCapabilities.fs.{readTextFile,writeTextFile}`（需实际装配 [`FsHandler`]）。
    pub fs: bool,
    /// 声明 `clientCapabilities.terminal`（需实际装配 [`TerminalHandler`]）。
    pub terminal: bool,
}

impl AcpClientCapabilities {
    /// 全 false（既有 [`AcpAgentClient::connect`] 的等价语义）。
    pub const NONE: Self = Self {
        fs: false,
        terminal: false,
    };
}

impl AcpAgentClient {
    /// 启动子进程并完成 `initialize` 握手（等价于 [`Self::connect_with_capabilities`] + 全 false）。
    ///
    /// 既有调用点行为不变：不声明 `fs` / `terminal`（agent 不会发 `fs/*` / `terminal/*` 反向请求）。
    pub fn connect(spec: &AcpAgentSpec, cwd: &Path) -> Result<Self, AcpTransportError> {
        Self::connect_with_capabilities(spec, cwd, AcpClientCapabilities::NONE)
    }

    /// B175 additive：按传入能力声明握手（`initialize` 的 `clientCapabilities.fs/terminal`）。
    ///
    /// 声明公式：`caps = spec.capabilities_hint ∧ 实际将装配的 handler`（hint 是上限：hint=false
    /// 时即使有 handler 也不声明）。caps 必须在 **connect 之前** 由调用方决定（initialize 是握手
    /// 第一条）；握手后 [`Self::install_fs_handler`] / [`Self::install_terminal_handler`] 只允许
    /// 替换**已声明**能力的实现，不能让会话追认未声明的能力（fail-closed）。
    ///
    /// 行为契约：
    /// 1. `AcpTransport::spawn(spec, cwd, &[])`（env 白名单 / 进程组 / stderr 环形缓冲由 transport 收口）；
    /// 2. 发送 `initialize { protocolVersion: 1, clientCapabilities{caps}, clientInfo }`；
    /// 3. 校验响应 `protocolVersion == 1`，不等即 `Err(AcpTransportError::Protocol(..))`
    ///    （`acp_protocol_error`），**不重试、不降级**。
    pub fn connect_with_capabilities(
        spec: &AcpAgentSpec,
        cwd: &Path,
        capabilities: AcpClientCapabilities,
    ) -> Result<Self, AcpTransportError> {
        // 既有入口等价：`_meta` 缺省不发送。
        Self::connect_with_capabilities_and_meta(spec, cwd, capabilities, None)
    }

    /// B180 additive：`connect_with_capabilities` + `clientCapabilities._meta` 透传。
    ///
    /// `_meta` 是 ACP v1 的扩展面包（实现方不得假设内容）：opencode 需要客户端回显
    /// `{"opencode/child-session-updates": true}` 才推子会话/compaction 更新；另有
    /// `session.compaction` / `elicitation.form` 等私有键。宿主**只透传**，不做语义解读。
    ///
    /// 既有 4 个入口（`connect` / `connect_with_capabilities` / `spawn*`）签名不变，均等价于
    /// 本入口的 `client_meta = None`。
    pub fn connect_with_capabilities_and_meta(
        spec: &AcpAgentSpec,
        cwd: &Path,
        capabilities: AcpClientCapabilities,
        client_meta: Option<Value>,
    ) -> Result<Self, AcpTransportError> {
        let mut transport = AcpTransport::spawn(spec, cwd, &[])?;

        // 反向请求转发壳必须在首次 `initialize` 应答前就位：agent 可能在 initialize 期间就发
        // `session/request_permission`（fixture 的 `permission` 模式即如此）。
        // B3：同时记录本次会话**将声明**的能力，作为后续装配/分发闸门。
        let handlers = Arc::new(Mutex::new(HandlerSlots {
            declared: capabilities,
            ..HandlerSlots::default()
        }));
        transport.set_handler(Arc::new(DelegatingRequestHandler {
            slots: Arc::clone(&handlers),
        }));

        // B175：能力声明只由调用方传入的 caps 推导（caps 已由 session 侧与 hint 取交集）；
        // 未声明的能力 ⇒ agent 不会发反向请求 ⇒ fail-closed（无 handler 时回 JSON-RPC error）。
        let wire_capabilities = ClientCapabilities {
            fs: capabilities.fs.then(|| FileSystemCapabilities {
                read_text_file: true,
                write_text_file: true,
            }),
            terminal: capabilities.terminal.then_some(true),
            auth: Some(AuthCapabilities { terminal: false }),
            meta: client_meta,
        };
        let request = InitializeRequest {
            protocol_version: PROTOCOL_VERSION,
            client_capabilities: wire_capabilities,
            client_info: Some(Implementation {
                name: "pony-agent".to_string(),
                title: Some("Pony Agent".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
            }),
        };
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 initialize 请求失败：{error}。"))
        })?;
        // B178-1：握手预算独立（真机 `agy acp` 冷启动 + 初始化可 >5s），不用单请求 5s。
        let result = transport.request(method::INITIALIZE, params, ACP_HANDSHAKE_TIMEOUT)?;
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
        self.new_session_inner(cwd, mcp_servers, None)
    }

    /// B181 additive：带 `additionalDirectories` 的建会话。
    ///
    /// **校验责任在 client**（opencode 会强校验并回 `-32602`，agy 不校验 ⇒ 宿主必须自己把关）：
    /// - 必须是**绝对路径**（相对路径直接拒绝，不做隐式 cwd 拼接）；
    /// - **禁止 glob 元字符**（`*` `?` `[` `]` `{` `}`），否则拒绝；
    /// - 必须存在且是目录，经 `std::fs::canonicalize` 归一后下行（去重、保持调用顺序）；
    /// - 任一校验失败 ⇒ `AcpTransportError::Spawn`（fail-closed，绝不静默丢弃该目录）。
    pub fn new_session_with_dirs(
        &mut self,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
        additional_directories: &[std::path::PathBuf],
    ) -> Result<SessionNewResponse, AcpTransportError> {
        let dirs = canonicalize_additional_directories(additional_directories)?;
        let additional = if dirs.is_empty() { None } else { Some(dirs) };
        self.new_session_inner(cwd, mcp_servers, additional)
    }

    fn new_session_inner(
        &mut self,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
        additional_directories: Option<Vec<String>>,
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
            additional_directories,
        };
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 session/new 请求失败：{error}。"))
        })?;
        let result = self
            .transport
            // B178-1：`session/new` 同属握手阶段，用握手预算（真机会话初始化可 >5s）。
            .request(method::SESSION_NEW, params, ACP_HANDSHAKE_TIMEOUT)?;
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

    /// B175 additive：仅热替换 `fs/*` handler（保留既有 permission / terminal 槽位）。
    ///
    /// 只能替换**握手时已声明**的 fs 能力实现；未声明时注入不改变线上能力（capability 声明已在
    /// `initialize` 固化，不会追认）。
    pub fn install_fs_handler(&self, handler: Arc<dyn FsHandler>) {
        let mut slots = self
            .handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !slots.declared.fs {
            // B3：握手已固化「不声明 fs」⇒ 装配被拒绝（不追认新能力；绝不执行反向请求）。
            eprintln!(
                "[acp][client] 拒绝装配 FsHandler：本会话 initialize 未声明 clientCapabilities.fs（fail-closed，不追认）。"
            );
            return;
        }
        slots.fs = Some(handler);
    }

    /// B175 additive：仅热替换 `terminal/*` handler（保留既有 permission / fs 槽位）。
    pub fn install_terminal_handler(&self, handler: Arc<dyn TerminalHandler>) {
        let mut slots = self
            .handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !slots.declared.terminal {
            eprintln!(
                "[acp][client] 拒绝装配 TerminalHandler：本会话 initialize 未声明 clientCapabilities.terminal（fail-closed，不追认）。"
            );
            return;
        }
        slots.terminal = Some(handler);
    }

    /// B179：`session/list`（分页参数原样透传；`nextCursor` 缺省 ⇒ `None`）。
    ///
    /// 预算用握手档（[`ACP_HANDSHAKE_TIMEOUT`]）：agy 列已落盘会话可能较慢，5s 单请求易误杀；
    /// `-32601` 由 transport 归因为 [`AcpTransportError::Unsupported`]（调用方降级跳过）。
    pub fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, AcpTransportError> {
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 session/list 请求失败：{error}。"))
        })?;
        let result = self
            .transport
            .request(method::SESSION_LIST, params, ACP_HANDSHAKE_TIMEOUT)?;
        serde_json::from_value(result).map_err(|error| {
            AcpTransportError::Protocol(format!(
                "session/list 响应不符合 v1 schema：{error}。"
            ))
        })
    }

    /// B179：`session/load`（**两形态兼容**：响应缺 `sessionId` ⇒ 沿用请求 id + stderr 诊断）。
    ///
    /// `cwd` 必须绝对路径、`additionalDirectories` 与 `session/new` 同一 fail-closed 校验口径
    ///（canonical 化 / 拒 glob / 拒相对 / 去重保序）。成功后把本 client 的会话标识更新为
    /// `响应 sessionId ?? 请求 sessionId`。
    pub fn load_session(
        &mut self,
        request: LoadSessionRequest,
    ) -> Result<LoadSessionResponse, AcpTransportError> {
        if !Path::new(&request.cwd).is_absolute() {
            return Err(AcpTransportError::Spawn(format!(
                "session/load 的 cwd 必须是绝对路径，实际 `{}`。",
                request.cwd
            )));
        }
        let requested: Vec<std::path::PathBuf> = request
            .additional_directories
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect();
        let canonical_dirs = canonicalize_additional_directories(&requested)?;
        let mut request = request;
        request.additional_directories = if canonical_dirs.is_empty() {
            None
        } else {
            Some(canonical_dirs)
        };
        let fallback_session_id = request.session_id.clone();
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 session/load 请求失败：{error}。"))
        })?;
        let result = self
            .transport
            .request(method::SESSION_LOAD, params, ACP_HANDSHAKE_TIMEOUT)?;
        let response: LoadSessionResponse = serde_json::from_value(result).map_err(|error| {
            AcpTransportError::Protocol(format!(
                "session/load 响应不符合 v1 schema：{error}。"
            ))
        })?;
        match response.session_id.as_deref() {
            Some(session_id) => self.session_id = Some(session_id.to_string()),
            None => {
                // 两形态兼容：opencode 只回 configOptions/modes ⇒ 沿用请求 id（不伪造）。
                eprintln!(
                    "[acp][client] session/load 响应缺 sessionId，沿用请求中的会话标识 `{fallback_session_id}`。"
                );
                self.session_id = Some(fallback_session_id);
            }
        }
        Ok(response)
    }

    /// B179：`session/close`（幂等；`-32601` ⇒ 能力降级，**不视为失败**）。
    pub fn close_session(&self, request: CloseSessionRequest) -> Result<(), AcpTransportError> {
        let params = serde_json::to_value(request).map_err(|error| {
            AcpTransportError::Protocol(format!("序列化 session/close 请求失败：{error}。"))
        })?;
        match self
            .transport
            .request(method::SESSION_CLOSE, params, ACP_REQUEST_TIMEOUT)
        {
            // 成功与幂等（agent 回 `{}`）同路。
            Ok(_) => Ok(()),
            Err(AcpTransportError::Unsupported(message)) => {
                // 能力降级：agent 不支持 session/close 不构成失败（调用方跳过该能力即可）。
                eprintln!("[acp][client] session/close 不受支持，降级跳过：{message}");
                Ok(())
            }
            Err(error) => Err(error),
        }
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

/// B181：`additionalDirectories` 的宿主侧校验 + canonical 化（`new_session_with_dirs` 唯一入口）。
///
/// 规则（fail-closed，任一不满足即 `AcpTransportError::Spawn`）：
/// 1. 必须绝对路径；
/// 2. 禁止 glob 元字符（`*` `?` `[` `]` `{` `}`）；
/// 3. 必须存在且是目录，`canonicalize` 归一（符号链接按真实落点）；
/// 4. 去重（保持首次出现顺序）。
pub fn canonicalize_additional_directories(
    directories: &[std::path::PathBuf],
) -> Result<Vec<String>, AcpTransportError> {
    const GLOB_META: [char; 6] = ['*', '?', '[', ']', '{', '}'];
    let mut normalized: Vec<String> = Vec::with_capacity(directories.len());
    for directory in directories {
        let raw = directory.to_string_lossy().into_owned();
        if !directory.is_absolute() {
            return Err(AcpTransportError::Spawn(format!(
                "acp_additional_directories_invalid: `{raw}` 必须是绝对路径（禁止相对路径）。"
            )));
        }
        if raw.chars().any(|character| GLOB_META.contains(&character)) {
            return Err(AcpTransportError::Spawn(format!(
                "acp_additional_directories_invalid: `{raw}` 含 glob 元字符，additionalDirectories 只接受具体目录。"
            )));
        }
        let canonical = std::fs::canonicalize(directory).map_err(|error| {
            AcpTransportError::Spawn(format!(
                "acp_additional_directories_invalid: `{raw}` 不可 canonical 化（{error}）。"
            ))
        })?;
        let metadata = std::fs::metadata(&canonical).map_err(|error| {
            AcpTransportError::Spawn(format!(
                "acp_additional_directories_invalid: `{raw}` 元数据不可读（{error}）。"
            ))
        })?;
        if !metadata.is_dir() {
            return Err(AcpTransportError::Spawn(format!(
                "acp_additional_directories_invalid: `{raw}` 不是目录。"
            )));
        }
        let normalized_entry = canonical.to_string_lossy().into_owned();
        if !normalized.contains(&normalized_entry) {
            normalized.push(normalized_entry);
        }
    }
    Ok(normalized)
}
