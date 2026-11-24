//! ACP 会话驱动 + 治理归约。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2.1（串行化）、§3.2.3（审批）、
//! §3.2.6（取消）、§3.2.7（事件映射）、§3.2.8（stopReason 归约）。
//!
//! 本层是 Pony 治理链与外部 agent 的唯一接缝：**不得**绕过 `ask_control` /
//! `path_permission` / `ProcessManager`。fail-closed 是本层的默认姿态：
//! - 反向请求没有宿主 handler ⇒ `cancelled`（绝不默认 `allow`）；
//! - 审批未决 / 超时 / 会话取消 ⇒ `cancelled`；
//! - 子进程崩溃 / 协议错乱 ⇒ `ToolResult { status: error }` + 真源 `acp_*` 错误码，不 panic。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::acp::client::{
    AcpAgentClient, AcpClientCapabilities, FsHandler, PermissionHandler, TerminalHandler,
};
use crate::acp::config::AcpAgentSpec;
use crate::acp::delegate_events::{map_acp_tool_status, ParentTurnRef};
use crate::acp::protocol::{
    method, CloseSessionRequest, ContentBlock, ListSessionsRequest, ListSessionsResponse,
    LoadSessionRequest, LoadSessionResponse, McpServer, PermissionOptionKind,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SessionNotification, SessionUpdate, StopReason, ToolCallLocation, ToolCallStatus, ToolKind,
};
use crate::acp::result::{
    reduce_final_text, reduce_usage, DelegateFinalText, DelegateUsage,
    ACP_DELEGATE_FINAL_TEXT_MAX_BYTES,
};
use crate::acp::transport::{AcpNotificationHandler, AcpTransportError, ACP_REQUEST_TIMEOUT};
use crate::agent::ask_persistence::{AskLifecycleState, PersistedAsk};
use crate::agent::budget::CancellationToken;
use crate::agent::dispatcher::{ControlRequestAuthorization, GovernedDispatcher};
use crate::agent::otel::log::{log_json, log_json_with_fields};
use crate::agent::runtime::TurnStreamEvent;
use crate::agent::telemetry::TurnToolActivity;
use crate::agent::tool_runtime::{
    PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
};
use crate::agent::tools::ToolResult;
use crate::agent::turn_flow::TurnEventSink;

/// `refusal` 归约后的错误码（§3.2.9 错误码分源）。
pub const ACP_ERROR_AGENT_REFUSED: &str = "acp_agent_refused";

/// `cancelled` 归约后的错误码（沿用既有 `tool_cancelled`，§3.2.8）。
pub const ACP_TOOL_CANCELLED: &str = "tool_cancelled";

/// 能力声明推导（B175 纯函数）：`caps = hint ∧ 实际将装配的 handler`。
///
/// - `spec.capabilities_hint` 是**上限**（用户声明"愿意下放"）：hint=false 时即使宿主有 handler
///   也**不声明**；
/// - `has_fs` / `has_terminal` 表示 connect **之前**已构造好、握手后一定会装配的 handler；
///   没有真实 handler 承载的能力不得声明（谎报会让外部 agent 发来无法治理的反向请求）；
/// - `initialize` 是握手第一条 ⇒ 本函数必须在 `connect_with_capabilities` **之前**调用。
pub fn capabilities_for(
    spec: &AcpAgentSpec,
    has_fs: bool,
    has_terminal: bool,
) -> AcpClientCapabilities {
    AcpClientCapabilities {
        fs: spec.capabilities_hint.fs && has_fs,
        terminal: spec.capabilities_hint.terminal && has_terminal,
    }
}

/// `acp_delegate` 工具名（`ToolResult.tool_name` 的产品面）。
const TOOL_ACP_DELEGATE: &str = "acp_delegate";

/// 审批轮询间隔（有界条件轮询：到达终态立即返回，不靠固定 sleep 当同步）。
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// prompt 期间取消令牌的轮询间隔（契约 `turn_cancel_poll_ms` ≤ 100ms，与 exec.rs 同范式）。
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// stopReason 的 Pony 归约结果（§3.2.8 映射表，5 → 4 类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpTurnOutcome {
    /// `end_turn` → `status: ok`。
    Ok,
    /// `max_tokens` / `max_turn_requests` → `status: ok` + `truncated: true`。
    Truncated,
    /// `refusal` → `status: error`，错误码 [`ACP_ERROR_AGENT_REFUSED`]。
    Refused,
    /// `cancelled` → `status: aborted`，错误码 [`ACP_TOOL_CANCELLED`]。
    Cancelled,
}

/// B176/B184：CLI 宿主形态的 fs/terminal handler 构造（`spawn_for_cli*` 与 `resume` 共用口径）。
///
/// 只有 `capabilities_hint` 为 true 才构造真实 handler（hint 是上限；handler 存在性参与
/// [`capabilities_for`] 推导 ⇒ hint=false 时 initialize 不声明）；permission 不在此处
///（CLI 无 Ask 面板 ⇒ 恒 `FailClosedPermissionHandler`）。
fn cli_capability_handlers(
    spec: &AcpAgentSpec,
    cwd: &Path,
) -> (Option<Arc<dyn FsHandler>>, Option<Arc<dyn TerminalHandler>>) {
    let fs_handler: Option<Arc<dyn FsHandler>> = spec.capabilities_hint.fs.then(|| {
        Arc::new(crate::acp::handlers::WorkspaceFsHandler::new(
            cwd.to_path_buf(),
            Arc::new(crate::agent::path_permission::AuthorizeStore::new()),
            crate::agent::path_permission::PathPurpose::Read,
        )) as Arc<dyn FsHandler>
    });
    let terminal_handler: Option<Arc<dyn TerminalHandler>> =
        spec.capabilities_hint.terminal.then(|| {
            Arc::new(
                crate::acp::handlers::ManagedTerminalHandler::with_workspace_root(
                    Arc::new(crate::agent::process::ProcessManager::new()),
                    cwd.to_path_buf(),
                ),
            ) as Arc<dyn TerminalHandler>
        });
    if spec.capabilities_hint.fs || spec.capabilities_hint.terminal {
        log_json_with_fields(
            "info",
            "acp capabilities 按真实 handler 装配推导（声明=可否决上限 ∧ handler 存在；permission 恒 fail-closed）",
            None,
            &[
                ("scope", json!("ponycli-acp")),
                ("fs", json!(fs_handler.is_some())),
                ("terminal", json!(terminal_handler.is_some())),
            ],
        );
    }
    (fs_handler, terminal_handler)
}

/// 挂起审批的取消标记：`cancel()` 只需翻标志，不必持有 dispatcher。
struct PermissionSlot {
    cancel_requested: AtomicBool,
}

impl PermissionSlot {
    fn new() -> Self {
        Self {
            cancel_requested: AtomicBool::new(false),
        }
    }

    fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Release);
    }

    fn is_cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Acquire)
    }
}

/// 会话级反向请求 handler：当前无宿主审批 dispatcher 绑定 ⇒ 一律 fail-closed `cancelled`。
///
/// Ask 往返的正规入口是 [`AcpAgentSession::resolve_permission`]（宿主持有 dispatcher 时显式
/// 调用）；prompt 期间由 transport 内联派发的权限请求没有 dispatcher 可依托，因此这里只能
/// fail-closed —— 这符合 §6 风险表第一条「绝不默认 allow」。
struct FailClosedPermissionHandler;

impl std::fmt::Debug for FailClosedPermissionHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FailClosedPermissionHandler")
    }
}

impl PermissionHandler for FailClosedPermissionHandler {
    fn decide(&self, request: &RequestPermissionRequest) -> RequestPermissionResponse {
        log_json_with_fields(
            "info",
            "acp 无宿主审批 dispatcher 绑定，session/request_permission fail-closed：cancelled",
            None,
            &[("tool_call_id", json!(request.tool_call.tool_call_id))],
        );
        RequestPermissionResponse {
            outcome: RequestPermissionOutcome::Cancelled,
        }
    }
}

/// 宿主审批 handler（B161）：把 agent 的 `session/request_permission` 接到
/// `GovernedDispatcher` 的既有往返面。
///
/// 语义（executor 实现时必须逐条兑现）：
/// 1. 把请求注册成 `PendingControlRequestKind::Approval`（携带 toolCall 与 options）；有界等待
///    （轮询间隔 ≤ `turn_cancel_poll_ms`，总预算 ≤ `ACP_REQUEST_TIMEOUT`，即 NFR
///    `acp.request_timeout_ms` = 5000ms）；
/// 2. 宿主 `approve_control_request` / `cancel_control_request`（或
///    `ask_control::answer_ask(dispatcher, request_id, authorization)`）消费后：
///    - 批准 ⇒ 从 `request.options` 里取宿主选择的 `optionId` 返回
///      `{outcome:{outcome:"selected",optionId}}`；`optionId` **必须 ∈ options**，不得越界编造；
///    - 取消 / 拒绝 / 超时 / dispatcher 不可用 ⇒ `{outcome:{outcome:"cancelled"}}`；
/// 3. **绝不默认 allow**：没有宿主响应（无面板/无回答）⇒ `cancelled`（§6 风险表第一条）。
#[allow(dead_code)]
pub struct DispatcherPermissionHandler {
    /// 既有治理往返面（与 ask/approval 共用同一 dispatcher）。
    dispatcher: Arc<GovernedDispatcher>,
    /// 可选的 **per-request** CAS 授权模板（仅用于把宿主批准映射成 `optionId`）。
    ///
    /// B161 生产接线：`ControlRequestAuthorization::for_request(pending, answer)` 是 **per-request**
    /// 事实，不能预造 ⇒ 生产路径用 [`Self::without_pinned_authorization`] 构造（本字段 `None`），
    /// 由宿主在 approve/cancel 时自行用 `for_request` 消费 CAS；本 handler 只负责注册与等待。
    /// 仅在测试/兼容路径可 pin 一份模板（`selected` 的 optionId 优先取其中的 `answer`）。
    authorization: Option<ControlRequestAuthorization>,
}

impl std::fmt::Debug for DispatcherPermissionHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 只打角色名：dispatcher/authorization 是治理内部事实，不进日志。
        formatter.write_str("DispatcherPermissionHandler")
    }
}

impl DispatcherPermissionHandler {
    /// 构造 handler（握手**前**构造，握手后、`new_session` 前
    /// 由 [`AcpAgentSession::spawn_with_full_options`] 装配）。
    pub fn new(
        dispatcher: Arc<GovernedDispatcher>,
        authorization: ControlRequestAuthorization,
    ) -> Self {
        Self {
            dispatcher,
            authorization: Some(authorization),
        }
    }

    /// B161 生产构造：不预造 per-request 授权（宿主在消费时用
    /// `ControlRequestAuthorization::for_request(pending, answer)` 派生 CAS 事实）。
    pub fn without_pinned_authorization(dispatcher: Arc<GovernedDispatcher>) -> Self {
        Self {
            dispatcher,
            authorization: None,
        }
    }
}

impl PermissionHandler for DispatcherPermissionHandler {
    fn decide(&self, request: &RequestPermissionRequest) -> RequestPermissionResponse {
        // 1) 注册 `PendingControlRequestKind::Approval`（与 ask/approval 同一张 CAS 表；
        //    沿用 `resolve_permission` 已验证的注册路径，审批不可被本层伪造或绕过）。
        let request_id = format!(
            "acp-permission-{}-{}",
            request.session_id,
            uuid::Uuid::new_v4()
        );
        let pending = register_approval(&self.dispatcher, request, &request_id);
        // 2) 有界等待宿主 approve / cancel（≤ NFR `acp.request_timeout_ms`）。本 handler 不持有
        //    会话取消信号（取消链路由 `AcpAgentSession::cancel()` 发 `session/cancel` 通知，
        //    agent 会以 `stopReason: cancelled` 收尾），故取消谓词恒 false。
        // 3) 无宿主 / 超时 / 已取消 ⇒ `cancelled`（绝不 allow、不 panic）。
        match await_approval(
            &self.dispatcher,
            self.authorization.as_ref(),
            &pending,
            request,
            &|| false,
        ) {
            Ok(response) => response,
            Err(error) => {
                log_json_with_fields(
                    "warn",
                    "acp 宿主审批往返失败，fail-closed：cancelled",
                    None,
                    &[("error", json!(error))],
                );
                cancelled_response()
            }
        }
    }
}

/// 把 `session/request_permission` 注册成 dispatcher 里的 Approval 待决请求。
///
/// 与 `resolve_permission` 共用同一路径：`hydrate_persisted_ask` 进入 dispatcher 的 CAS 表，
/// 只能被真实 `approve_control_request` / `cancel_control_request` 消费。
fn register_approval(
    dispatcher: &GovernedDispatcher,
    request: &RequestPermissionRequest,
    request_id: &str,
) -> PendingControlRequest {
    let record = PersistedAsk {
        request_id: request_id.to_string(),
        version: 1,
        nonce: format!("acp-nonce-{}", uuid::Uuid::new_v4()),
        session_id: request.session_id.clone(),
        run_id: None,
        turn_id: request.tool_call.tool_call_id.clone(),
        call_id: request.tool_call.tool_call_id.clone(),
        kind: PendingControlRequestKind::Approval,
        prompt: format!(
            "外部 ACP agent 请求工具权限：{}",
            request
                .tool_call
                .title
                .clone()
                .unwrap_or_else(|| request.tool_call.tool_call_id.clone())
        ),
        options: request
            .options
            .iter()
            .map(|option| option.option_id.clone())
            .collect(),
        multi_select: false,
        state: AskLifecycleState::Pending,
        created_at_ms: now_ms(),
        answered_at_ms: None,
    };
    dispatcher.hydrate_persisted_ask(&record, PendingControlRequestState::Pending)
}

/// 有界等待宿主决定（轮询间隔 [`PERMISSION_POLL_INTERVAL`]，总预算 [`ACP_REQUEST_TIMEOUT`]）。
///
/// 终态语义：`Consumed` ⇒ `selected`（optionId 只从 `request.options` 里取）；
/// `Cancelled/Expired`、请求消失、`is_cancelled()`、超时 ⇒ `cancelled`（fail-closed）。
fn await_approval(
    dispatcher: &GovernedDispatcher,
    authorization: Option<&ControlRequestAuthorization>,
    pending: &PendingControlRequest,
    request: &RequestPermissionRequest,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<RequestPermissionResponse, String> {
    let deadline = Instant::now() + ACP_REQUEST_TIMEOUT;
    loop {
        match dispatcher.pending_request(&pending.request_id) {
            Some(current) => match current.state {
                PendingControlRequestState::Pending => {}
                // 宿主已批准：把批准映射成请求 options 里的一个 optionId（绝不越界取值）。
                PendingControlRequestState::Consumed => {
                    return selected_response(request, authorization)
                }
                // Cancelled / Expired：批准已不可能，fail-closed。
                _ => return Ok(cancelled_response()),
            },
            // 请求从 CAS 表消失（不该发生）：fail-closed，不静默当成 allow。
            None => {
                log_json_with_fields(
                    "warn",
                    "acp 审批请求已从 dispatcher 消失，fail-closed：cancelled",
                    None,
                    &[("request_id", json!(pending.request_id))],
                );
                return Ok(cancelled_response());
            }
        }
        if is_cancelled() {
            settle_cancel(dispatcher, &pending.request_id);
            return Ok(cancelled_response());
        }
        if Instant::now() >= deadline {
            // 超时不是批准：显式把待决审批置 Cancelled 后再回 cancelled（不留 Pending 悬挂）。
            settle_cancel(dispatcher, &pending.request_id);
            return Ok(cancelled_response());
        }
        std::thread::sleep(PERMISSION_POLL_INTERVAL);
    }
}

/// prompt 期间的 `session/update` **实时**转发器（B162）。
///
/// `TurnEventSink` 的冻结签名不带 `Send + Sync`，读线程回调不能直写 sink；因此读线程回调把
/// `SessionUpdate` 经 `mpsc` 送回调用线程，调用线程 `recv_timeout` 逐条 `emit_update` ——
/// **不**在 prompt 返回后批量 flush（委派期间 UI 实时可见 `turn:delta` / `turn:tool`）。
/// 通道断开 = prompt 线程结束（正常或 panic），剩余帧先抽干再归约。
struct UpdateForwarder {
    /// 上行通道（读线程 → 调用线程）：**无界 mpsc**。
    ///
    /// 取舍（B162 时序契约）：读线程回调内 `send` 绝不阻塞（std mpsc 无界 ⇒ 无背压、无丢弃、
    /// 保序）；内存上界 = 单轮尚未消费的 update 帧数（消费即释放，帧平均几 KB）。
    /// 不选「有界 + 丢弃」：丢弃会破坏 R2（顺序）/ R3（不丢最后一条）语义。发送失败
    ///（调用线程已退出）静默丢弃，不 panic。
    tx: mpsc::Sender<SessionUpdate>,
    /// 解析失败 / 非 update 通知的诊断（与 update 流分离，最终随归约结果记账）。
    warnings: Arc<Mutex<Vec<String>>>,
}

impl AcpNotificationHandler for UpdateForwarder {
    fn handle_notification(&self, method: &str, params: Value) {
        if method != method::SESSION_UPDATE {
            // 非 update 通知不静默：结构化日志 + 记一条 warning（不进 UI 事件流）。
            let warning = format!("acp_unexpected_notification: `{method}`");
            log_json_with_fields(
                "info",
                "acp 收到未处理的入站通知（不计入 update 流）",
                None,
                &[("method", json!(method)), ("params", json!(params))],
            );
            if let Ok(mut warnings) = self.warnings.lock() {
                warnings.push(warning);
            }
            return;
        }
        match serde_json::from_value::<SessionNotification>(params) {
            Ok(notification) => {
                // 调用线程已退出（prompt 结束）⇒ 丢弃即可，绝不 panic。
                let _ = self.tx.send(notification.update);
            }
            Err(error) => {
                // 载荷不合 v1 schema：记 warning 并继续（不 panic、不静默）。
                if let Ok(mut warnings) = self.warnings.lock() {
                    warnings.push(format!(
                        "acp_protocol_error: session/update 载荷不符合 v1 schema：{error}"
                    ));
                }
            }
        }
    }
}

/// 一条 ACP 会话（进程级），把外部 agent 的 update / stopReason / 反向请求映射进 Pony。
pub struct AcpAgentSession {
    client: AcpAgentClient,
    /// 会话串行闸（§3.2.1）：opencode 明确「一个 session 同时只能跑一个 prompt」。
    /// 并发委派必须走「每任务一个子进程」或排队，**不得**并发打同一 session。
    prompt_gate: Mutex<()>,
    session_id: String,
    /// 取消令牌（§3.2.6）：宿主 `stop_turn` 被接受后，先 `session/cancel` notification，
    /// 再把所有挂起 `session/request_permission` 回 `cancelled`。
    ///
    /// 由 [`AcpAgentSession::set_cancellation_token`] 写入（`&self` 热替换）：prompt 期间按
    /// `turn_cancel_poll_ms`（≤100ms）轮询，触发即 `client.cancel` + 归约 aborted / `tool_cancelled`。
    cancellation: Mutex<Option<Arc<CancellationToken>>>,
    /// 当前挂起的审批槽位（`cancel()` 翻标志 → `resolve_permission` 立即回 `cancelled`）。
    pending_permissions: Mutex<Vec<Arc<PermissionSlot>>>,
    /// 本会话的**本地**取消标志：`cancel()` 只翻它，绝不去 cancel 宿主注入的共享 turn 令牌
    /// （单次委派超时取消不得连带取消整轮 turn）。
    local_cancel: AtomicBool,
    /// wave-acp-delegate C1（additive）：本次委派的**父 turn 身份**（三要素缺一即视同无上下文）。
    ///
    /// 由 [`AcpAgentSession::set_parent_turn`] 在 prompt 之前注入；委派期间的
    /// `tool_call` / `tool_call_update` 经 `acp::delegate_events::emit_parent_tool_activity`
    /// 以父 session/turn 落 `turn:tool`（C1.1/C1.5）。`None` ⇒ 只走本地 sink（C1.3 静默跳过）。
    parent_turn: Mutex<Option<ParentTurnRef>>,
}

impl AcpAgentSession {
    /// 建会话：`AcpAgentClient::connect` → `install_handlers` → `new_session`。
    ///
    /// `cwd` 必须是绝对路径（从 `ToolExecutionContext.workspace_root` 取）。
    ///
    /// B174：`mcpServers` 走 spec 三态（缺省 ⇒ inherit：复用 Pony 既有 MCP 配置；显式 `[]` ⇒
    /// 禁用；显式条目 ⇒ 仅用条目），由 [`crate::acp::config::resolve_mcp_servers_in`] 解析后
    /// 经 [`Self::spawn_with_mcp`] 下发。既有签名不变；配置不可用时 fail-closed
    /// （`AcpTransportError::Unavailable`，消息保留 `acp_config_*` 码），不静默降级为空列表。
    pub fn spawn(spec: &AcpAgentSpec, cwd: &Path) -> Result<Self, AcpTransportError> {
        let mcp_servers = crate::acp::config::resolve_mcp_servers_in(spec, cwd)
            .map_err(|error| AcpTransportError::Unavailable(error.to_string()))?;
        Self::spawn_with_mcp(spec, cwd, mcp_servers)
    }

    /// B174 additive：用**已解析的** wire `mcpServers` 建会话（不叠加 inherit）。
    ///
    /// 与 [`Self::spawn`] 共用同一路径：先 `connect`（含 `protocolVersion == 1` 校验与
    /// fs/terminal 能力 fail-closed 不下放），再 `session/new { cwd, mcpServers }`。
    pub fn spawn_with_mcp(
        spec: &AcpAgentSpec,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
    ) -> Result<Self, AcpTransportError> {
        // 既有等价行为：无真实 fs/terminal handler ⇒ 不声明任何能力。
        Self::spawn_with_mcp_and_handlers(spec, cwd, mcp_servers, None, None)
    }

    /// B175 amend additive：**先按「将要装配的 handler」定 caps，再握手**，握手成功后立即装配。
    ///
    /// 这是 capacitiesHint 在生产路径真正生效的唯一入口（`install_*` 只能在 connect 之后调用，
    /// 而 `initialize` 是握手第一条，故能力必须在 connect 前决定）：
    /// 1. `caps = capabilities_for(spec, fs.is_some(), terminal.is_some())`（hint 是上限；
    ///    声明 = hint ∧ 将要装的 handler）→ `connect_with_capabilities`；
    /// 2. 校验不变式：`caps.fs == true` ⇒ `fs.is_some()`、`caps.terminal == true` ⇒
    ///    `terminal.is_some()`，否则 fail-closed `AcpTransportError::Unavailable`
    ///    （禁「声明了却没实现」）；
    /// 3. 握手成功后、`new_session` 之前装配 handler（`install_fs_handler` /
    ///    `install_terminal_handler`，保留 FailClosedPermissionHandler 审批面）；
    /// 4. `spawn` / `spawn_with_mcp` 等价于本入口传 `(None, None)`（不声明、不装配）。
    pub fn spawn_with_mcp_and_handlers(
        spec: &AcpAgentSpec,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
        fs: Option<Arc<dyn FsHandler>>,
        terminal: Option<Arc<dyn TerminalHandler>>,
    ) -> Result<Self, AcpTransportError> {
        // 既有等价行为：无 permission handler ⇒ FailClosedPermissionHandler（一律 cancelled）。
        Self::spawn_with_full_options(spec, cwd, mcp_servers, fs, terminal, None)
    }

    /// B161 additive：`spawn_with_mcp_and_handlers` + 可选宿主审批 handler。
    ///
    /// 装配顺序（与 fs/terminal 同一时点，均在 `new_session` 之前）：
    /// 1. **握手前**按 `capabilities_for(spec, fs.is_some(), terminal.is_some())` 定 caps →
    ///    `connect_with_capabilities`（permission 无 wire 声明位，不参与 caps）；
    /// 2. 声明与装配一致性校验：`caps.fs==true ⇒ fs.is_some()`、`caps.terminal==true ⇒
    ///    terminal.is_some()`，否则 `AcpTransportError::Unavailable`（fail-closed）；
    /// 3. 握手成功后、`new_session` 前装配：`permission` 缺省回落
    ///    `FailClosedPermissionHandler`（**无 permission handler ⇒ 一律 cancelled**）；
    ///    fs/terminal 仅在声明后装配（`HandlerSlots.declared` 闸门，不追认）；
    /// 4. `request_permission` 始终有 handler 兜底；**有 handler 但宿主无响应/超时 ⇒
    ///    cancelled**（在 `PermissionHandler` 实现内收口，绝不 allow）。
    pub fn spawn_with_full_options(
        spec: &AcpAgentSpec,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
        fs: Option<Arc<dyn FsHandler>>,
        terminal: Option<Arc<dyn TerminalHandler>>,
        permission: Option<Arc<dyn PermissionHandler>>,
    ) -> Result<Self, AcpTransportError> {
        // 既有等价行为：不带 additionalDirectories。
        Self::spawn_with_full_options_and_dirs(
            spec,
            cwd,
            mcp_servers,
            fs,
            terminal,
            permission,
            &[],
        )
    }

    /// B181 additive：`spawn_with_full_options` + `additionalDirectories` 透传。
    ///
    /// `additional_directories` 经 [`AcpAgentClient::new_session_with_dirs`] 校验/规范化：
    /// 必须绝对路径、禁 glob、存在且是目录 ⇒ canonical 化后下行；任一失败 fail-closed
    /// （`AcpTransportError::Spawn`，不静默丢弃）。能力声明/装配顺序与
    /// [`Self::spawn_with_full_options`] 完全一致（声明在 connect 前定稿，handler 在
    /// `new_session` 前装配）。
    pub fn spawn_with_full_options_and_dirs(
        spec: &AcpAgentSpec,
        cwd: &Path,
        mcp_servers: Vec<McpServer>,
        fs: Option<Arc<dyn FsHandler>>,
        terminal: Option<Arc<dyn TerminalHandler>>,
        permission: Option<Arc<dyn PermissionHandler>>,
        additional_directories: &[PathBuf],
    ) -> Result<Self, AcpTransportError> {
        // 1) 握手前的能力推导（initialize 是第一条报文）。
        let capabilities = capabilities_for(spec, fs.is_some(), terminal.is_some());
        // 2) 声明与装配一致性：声明为 true 却没有 handler ⇒ fail-closed，绝不谎报。
        if capabilities.fs && fs.is_none() {
            return Err(AcpTransportError::Unavailable(
                "acp_capability_inconsistent: 声明了 clientCapabilities.fs 但未提供 FsHandler。"
                    .to_string(),
            ));
        }
        if capabilities.terminal && terminal.is_none() {
            return Err(AcpTransportError::Unavailable(
                "acp_capability_inconsistent: 声明了 clientCapabilities.terminal 但未提供 TerminalHandler。"
                    .to_string(),
            ));
        }
        // 握手（含 protocolVersion == 1 的 fail-closed 校验）由 client 收口。
        let mut client = AcpAgentClient::connect_with_capabilities(spec, cwd, capabilities)?;
        // 3) 同一时点装配：permission（缺省 fail-closed）+ 已声明的 fs/terminal。
        //
        // 注意：`session/request_permission` **不属于** `clientCapabilities` 面（ACP v1 只声明
        // fs/terminal），因此 permission **恒装配、不受 `HandlerSlots.declared` 闸门限制**
        //（闸门只作用于 fs/terminal 两项声明能力）；无 permission handler 时装配的
        // FailClosedPermissionHandler 保证「一律 cancelled」。
        let permission = permission.unwrap_or_else(|| Arc::new(FailClosedPermissionHandler));
        client.install_handlers(permission, None, None);
        if let Some(handler) = fs {
            client.install_fs_handler(handler);
        }
        if let Some(handler) = terminal {
            client.install_terminal_handler(handler);
        }
        // 4) 会话建立在装配完成之后（杜绝「new_session 期间反向请求无 handler」窗口）。
        // B181：additionalDirectories 的校验与 canonical 化在 client 内收口。
        let response = if additional_directories.is_empty() {
            client.new_session(cwd, mcp_servers)?
        } else {
            client.new_session_with_dirs(cwd, mcp_servers, additional_directories)?
        };
        Ok(Self {
            client,
            prompt_gate: Mutex::new(()),
            session_id: response.session_id,
            cancellation: Mutex::new(Some(Arc::new(CancellationToken::new()))),
            pending_permissions: Mutex::new(Vec::new()),
            local_cancel: AtomicBool::new(false),
            parent_turn: Mutex::new(None),
        })
    }

    /// B176：CLI 宿主形态（`ponycli acp`）的会话入口（additive）。
    ///
    /// 与 [`Self::spawn`] 的唯一差别：按 `spec.capabilities_hint` 装配**真实可用**的
    /// fs/terminal handler 之后再握手 —— handler 存在性参与 [`capabilities_for`] 的推导，
    /// 因此只有 hint=true 时 `initialize` 才真的声明对应能力（hint=false ⇒ 不装配、不声明）。
    ///
    /// - **fs**：`WorkspaceFsHandler`（workspace 边界 + PA-080 授权清单；CLI 无持久化授权 ⇒ 空表）；
    /// - **terminal**：CLI 侧可用公开构造的 `ProcessManager::new()`（无宿主注入点，不需要额外装配），
    ///   绑定 workspace root 做 cwd 边界 ⇒ 可诚实声明；
    /// - **permission**：恒 `None`（CLI 无 Ask 面板 ⇒ `FailClosedPermissionHandler`，一律 cancelled）。
    pub fn spawn_for_cli(spec: &AcpAgentSpec, cwd: &Path) -> Result<Self, AcpTransportError> {
        // 既有等价行为：不带 additionalDirectories。
        Self::spawn_for_cli_with_dirs(spec, cwd, &[])
    }

    /// B181 additive：CLI 宿主形态 + `additionalDirectories` 透传（校验/规范化在 client）。
    pub fn spawn_for_cli_with_dirs(
        spec: &AcpAgentSpec,
        cwd: &Path,
        additional_dirs: &[PathBuf],
    ) -> Result<Self, AcpTransportError> {
        let mcp_servers = crate::acp::config::resolve_mcp_servers_in(spec, cwd)
            .map_err(|error| AcpTransportError::Unavailable(error.to_string()))?;
        let (fs_handler, terminal_handler) = cli_capability_handlers(spec, cwd);
        Self::spawn_with_full_options_and_dirs(
            spec,
            cwd,
            mcp_servers,
            fs_handler,
            terminal_handler,
            None,
            additional_dirs,
        )
    }

    /// B185 additive：CLI **只建连、不建会话**（供 `--list-sessions` / 将来的 `--close-session` 复用）。
    ///
    /// 语义（executor 实现时必须逐条兑现）：
    /// 1. 与 [`Self::spawn_for_cli_with_dirs`] **同一 caps/handlers 口径**：`capabilities_hint`
    ///    驱动 fs/terminal handler 与 `initialize.clientCapabilities`（caps 在 connect 前定稿；
    ///    hint=false 不装配、不声明），permission 恒 fail-closed；
    /// 2. connect 成功后装配 handler；
    /// 3. **不得创建会话**（不调用 `session/new`、不调用 `session/load`）：本入口返回的会话对象
    ///    `session_id()` 为空串（未建会话占位），仅可安全用于不依赖会话 id 的方法
    ///    （`list_sessions` / 将来的 `close_session` 显式传 id）；
    /// 4. 供 CLI `ponycli acp --list-sessions` 使用：只 connect → `session/list` → 输出 JSONL。
    #[allow(unused_variables)]
    pub fn connect_for_cli(
        spec: &AcpAgentSpec,
        cwd: &Path,
        additional_dirs: &[PathBuf],
    ) -> Result<Self, AcpTransportError> {
        // 1) caps/handlers 与 `spawn_for_cli_with_dirs` 同口径（caps 必须在 connect 前定稿）。
        let (fs_handler, terminal_handler) = cli_capability_handlers(spec, cwd);
        let capabilities = capabilities_for(spec, fs_handler.is_some(), terminal_handler.is_some());
        let client = AcpAgentClient::connect_with_capabilities(spec, cwd, capabilities)?;
        // 2) 装配次序同 `spawn_with_full_options`（permission 恒 fail-closed；fs/terminal 仅声明后装）。
        client.install_handlers(
            Arc::new(FailClosedPermissionHandler) as Arc<dyn PermissionHandler>,
            None,
            None,
        );
        if let Some(handler) = fs_handler {
            client.install_fs_handler(handler);
        }
        if let Some(handler) = terminal_handler {
            client.install_terminal_handler(handler);
        }
        // 3) `additionalDirectories` 与 `session/new|load` 同一 fail-closed 校验口径：即使本入口
        //    不建会话，也提前把非法目录拒掉（调用方一旦复用该连接建会话不会带病下发）。
        let _ = crate::acp::client::canonicalize_additional_directories(additional_dirs)?;
        // 4) **不建会话**：不调 `session/new`、不调 `session/load`（建会话属 spawn/resume 职责），
        //    `session_id` 用空串占位 —— 本入口只供 `--list-sessions` / 将来的 `--close-session`
        //    这类**无会话操作**复用；依赖会话 id 的 prompt/cancel/load 在该对象上无意义。
        Ok(Self {
            client,
            prompt_gate: Mutex::new(()),
            session_id: String::new(),
            cancellation: Mutex::new(Some(Arc::new(CancellationToken::new()))),
            pending_permissions: Mutex::new(Vec::new()),
            local_cancel: AtomicBool::new(false),
            parent_turn: Mutex::new(None),
        })
    }

    /// B184 additive：CLI `--resume` 建连（**跳过 `session/new`**，用 `session/load` 接续既有会话）。
    ///
    /// 语义（executor 实现时必须逐条兑现）：
    /// 1. **connect**：与 [`Self::spawn_for_cli_with_dirs`] 同口径组装 handlers/caps ——
    ///    `capabilities_hint` 驱动 fs/terminal handler 与 `initialize.clientCapabilities`
    ///    （caps 必须在 connect 前定稿），permission 恒 fail-closed；
    /// 2. **不得调用 `session/new`**：那会创建新会话，与 resume 语义冲突；
    /// 3. 调用 `client.load_session(LoadSessionRequest { cwd, session_id, mcp_servers:
    ///    resolve_mcp_servers_in(spec, cwd), additional_directories: 由 client canonical 化的
    ///    dirs, meta: None })`（字段已齐，见 `protocol.rs` 的 `LoadSessionRequest`）；
    /// 4. **session_id 同步**：响应带 `sessionId` ⇒ 用响应值；缺失 ⇒ 沿用请求的 `session_id`
    ///    （两形态兼容，不伪造）；
    /// 5. **失败面（绝不静默新建）**：agent 回 `-32601` 或 `initialize.agentCapabilities.loadSession
    ///    == false` ⇒ `AcpTransportError::Unsupported`（能力降级）；协议/版本不符 ⇒ `Protocol`；
    ///    任何失败都不得回落 `session/new` 或返回一个"新"会话。
    pub fn spawn_for_cli_resume(
        spec: &AcpAgentSpec,
        cwd: &Path,
        session_id: &str,
        additional_dirs: &[PathBuf],
    ) -> Result<Self, AcpTransportError> {
        // 1) caps/handlers 与 `spawn_for_cli_with_dirs` 同口径（caps 必须在 connect 前定稿）。
        let (fs_handler, terminal_handler) = cli_capability_handlers(spec, cwd);
        let capabilities = capabilities_for(spec, fs_handler.is_some(), terminal_handler.is_some());
        let mut client = AcpAgentClient::connect_with_capabilities(spec, cwd, capabilities)?;
        // 2) 提前判定 agent 是否支持恢复：`loadSession` 缺省/false ⇒ Unsupported（绝不静默新建）。
        if !client.agent_capabilities().load_session {
            client.close();
            return Err(AcpTransportError::Unsupported(format!(
                "agent `{}` 声明 loadSession=false，无法恢复会话 `{session_id}`（不静默新建会话）。",
                spec.id
            )));
        }
        // 3) 装配次序同 `spawn_with_full_options`（permission 恒 fail-closed，fs/terminal 只在声明后装）。
        client.install_handlers(
            Arc::new(FailClosedPermissionHandler) as Arc<dyn PermissionHandler>,
            None,
            None,
        );
        if let Some(handler) = fs_handler {
            client.install_fs_handler(handler);
        }
        if let Some(handler) = terminal_handler {
            client.install_terminal_handler(handler);
        }
        // 4) 跳过 `session/new`（那会新建会话），用 `session/load` 接续；mcpServers 同一三态解析。
        let mcp_servers = crate::acp::config::resolve_mcp_servers_in(spec, cwd)
            .map_err(|error| AcpTransportError::Unavailable(error.to_string()))?;
        let request = LoadSessionRequest {
            cwd: cwd.to_string_lossy().into_owned(),
            session_id: session_id.to_string(),
            mcp_servers,
            additional_directories: if additional_dirs.is_empty() {
                None
            } else {
                Some(
                    additional_dirs
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                )
            },
            meta: None,
        };
        let response = client.load_session(request)?;
        // 5) session_id 同步：响应带 sessionId ⇒ 用响应值；缺失沿用请求 id（client 已写 stderr 诊断）。
        let session_id = response
            .session_id
            .clone()
            .unwrap_or_else(|| session_id.to_string());
        Ok(Self {
            client,
            prompt_gate: Mutex::new(()),
            session_id,
            cancellation: Mutex::new(Some(Arc::new(CancellationToken::new()))),
            pending_permissions: Mutex::new(Vec::new()),
            local_cancel: AtomicBool::new(false),
            parent_turn: Mutex::new(None),
        })
    }

    /// B175 additive：装配 `fs/*` handler（热替换；保留既有 permission / terminal 槽位）。
    ///
    /// 只对**握手时已声明** fs 能力的会话有效：声明已在 `initialize` 固化，注入不追认新能力。
    pub fn install_fs_handler(&self, handler: Arc<dyn FsHandler>) {
        self.client.install_fs_handler(handler);
    }

    /// B175 additive：装配 `terminal/*` handler（热替换；保留既有 permission / fs 槽位）。
    pub fn install_terminal_handler(&self, handler: Arc<dyn TerminalHandler>) {
        self.client.install_terminal_handler(handler);
    }

    /// B185：把 `session/list` 结果格式化为**冻结 JSONL 行**（`ponycli acp --list-sessions` 共用）。
    ///
    /// 形状（`.dev-team/contract-matrix-acp-list-sessions.md` §2）：
    /// - 每会话一行 `{sessionId, cwd, title?, updatedAt?, additionalDirectories?}`（缺省不写 null）；
    /// - 响应带 `nextCursor` ⇒ 末尾追加一行 `{"nextCursor":"..."}`；
    /// - 空列表 ⇒ 空 `Vec`（调用方不打印任何行）；`-32601` ⇒ `Unsupported`（由 client 归因）。
    pub fn list_sessions_jsonl(
        &self,
        request: ListSessionsRequest,
    ) -> Result<Vec<String>, AcpTransportError> {
        let response = self.client.list_sessions(request)?;
        let mut lines: Vec<String> = Vec::with_capacity(response.sessions.len() + 1);
        for info in &response.sessions {
            let mut record = serde_json::Map::new();
            record.insert("sessionId".to_string(), serde_json::json!(info.session_id));
            record.insert("cwd".to_string(), serde_json::json!(info.cwd));
            if let Some(title) = &info.title {
                record.insert("title".to_string(), serde_json::json!(title));
            }
            if let Some(updated_at) = &info.updated_at {
                record.insert("updatedAt".to_string(), serde_json::json!(updated_at));
            }
            if !info.additional_directories.is_empty() {
                record.insert(
                    "additionalDirectories".to_string(),
                    serde_json::json!(info.additional_directories),
                );
            }
            let line = serde_json::to_string(&serde_json::Value::Object(record)).map_err(|error| {
                AcpTransportError::Protocol(format!("session/list 结果序列化失败：{error}。"))
            })?;
            lines.push(line);
        }
        if let Some(next_cursor) = &response.next_cursor {
            let line = serde_json::to_string(&serde_json::json!({ "nextCursor": next_cursor }))
                .map_err(|error| {
                    AcpTransportError::Protocol(format!(
                        "session/list nextCursor 序列化失败：{error}。"
                    ))
                })?;
            lines.push(line);
        }
        Ok(lines)
    }

    /// B179：`session/list` 签名空桩（executor 实现）。
    ///
    /// 语义：请求参数按官方 schema（`cwd?` / `cursor?` / `limit?`），返回 `sessions` 数组 +
    /// 可选 `nextCursor`（缺省 = 无更多页）；agy 只列已落盘会话。`-32601` ⇒
    /// `AcpTransportError::Unsupported`（能力降级，不报通用错误）。
    pub fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> Result<ListSessionsResponse, AcpTransportError> {
        // 分页/过滤参数原样透传；空 sessions 数组是合法结果；-32601 ⇒ Unsupported（由 client 归因）。
        self.client.list_sessions(request)
    }

    /// B179：`session/load` 签名空桩（executor 实现）。
    ///
    /// **两形态兼容**：响应可能不含 `sessionId`（opencode 只回 configOptions/modes；agy 回
    /// sessionId+models+modes 超集）⇒ 缺失时调用方沿用请求里的 session id；`additionalDirectories`
    /// 与 `session/new` 同一校验口径（client 侧 canonical 化 + glob 拒绝）。
    pub fn load_session(
        &mut self,
        request: LoadSessionRequest,
    ) -> Result<LoadSessionResponse, AcpTransportError> {
        // 两形态兼容由 client 收口（缺 sessionId ⇒ 沿用请求 id + stderr 诊断）；
        // 这里把会话标识同步到 session 层（与响应/请求 id 同源，不伪造）。
        let response = self.client.load_session(request)?;
        if let Some(session_id) = self.client.session_id() {
            self.session_id = session_id.to_string();
        }
        Ok(response)
    }

    /// B179：`session/close` 签名空桩（executor 实现）。
    ///
    /// 两者均幂等（不存在 id 回 `{}`）；`-32601` ⇒ `Unsupported` 降级（跳过该能力）。
    pub fn close_session(&self, request: CloseSessionRequest) -> Result<(), AcpTransportError> {
        // 幂等：不存在 id 由 agent 回 `{}`；-32601 由 client 降级为 Ok（能力跳过，不算失败）。
        self.client.close_session(request)
    }

    /// 设置整轮 prompt 预算（additive；转发给 [`AcpAgentClient::set_prompt_budget`]）。
    ///
    /// 调用方缺省必须传 [`crate::acp::ACP_DEFAULT_TURN_BUDGET`]（120s，与 CLI
    /// `--timeout-secs` / `ToolExecutionPolicy` 默认同源）；不传则仍回落到
    /// `ACP_REQUEST_TIMEOUT`（5000ms），真实 agent 的 >5s 一轮会被硬顶失败。
    pub fn set_turn_budget(&self, budget: Duration) {
        self.client.set_prompt_budget(budget);
        // ADVISORY ⑧：terminal 侧 `wait_for_exit` 的等待预算必须与 prompt 预算同源，
        // 否则调大轮次预算后长命令仍会被 120s 硬顶截断。
        self.client.set_terminal_turn_budget(budget);
    }

    /// 设置宿主取消令牌（additive；写入既有 `cancellation` 槽位，`&self` 热替换）。
    ///
    /// 语义（§3.2.6）：prompt 期间按 `turn_cancel_poll_ms`（≤100ms）轮询该令牌；一旦
    /// `is_cancelled()` ⇒ 发 `session/cancel` notification，挂起审批回 `cancelled`，本轮归约
    /// 为 `status: aborted` + 错误码 [`ACP_TOOL_CANCELLED`]。未设置时沿用会话自建令牌。
    pub fn set_cancellation_token(&self, token: Arc<CancellationToken>) {
        if let Ok(mut slot) = self.cancellation.lock() {
            *slot = Some(token);
        }
    }

    /// wave-acp-delegate C1（additive）：注入父 turn 身份三要素（父 session / 父 turn / call id）。
    ///
    /// 语义：委派期间的 ACP `tool_call` / `tool_call_update` 以**父 turn 身份**归集为 `turn:tool`
    /// 并落盘（C1.1）；文本 chunk 不发射到父正文（C1.2）。未注入 ⇒ 归集静默跳过（C1.3），
    /// 既有构造签名与行为不变（不新增必填构造参数）。
    pub(crate) fn set_parent_turn(&self, parent: ParentTurnRef) {
        if let Ok(mut slot) = self.parent_turn.lock() {
            *slot = Some(parent);
        }
    }

    /// 跑一轮 prompt（**实时** update 语义，B162）：串行闸 → `client.prompt` 在旁路线程执行 →
    /// 读线程回调把 `SessionUpdate` 经 `mpsc` 送回**调用线程** → 调用线程 `recv_timeout` 循环
    /// 把每一帧立即 [`Self::emit_update`]（不批量 flush）→ `stopReason` 归约成 [`ToolResult`]。
    ///
    /// - 时序：`turn:delta` / `turn:tool` 在 agent 产出时即刻上行；prompt 返回只做终局归约。
    /// - 预算：`client.prompt` 的 deadline 取 [`AcpAgentSession::set_turn_budget`]（默认
    ///   `ACP_REQUEST_TIMEOUT`），**不是**固定 5s 硬顶。
    /// - 取消：同一 `recv_timeout` 循环按 `turn_cancel_poll_ms`(≤100ms) 轮询令牌；触发即
    ///   `session/cancel`，仍须 < 预算返回 `aborted` + `tool_cancelled`（不改变取消语义）。
    /// - 归约口径：`ok` / `truncated` 仍为成功；`refused` → error `acp_agent_refused`；
    ///   `cancelled` → aborted `tool_cancelled`（与 B162 前逐字段一致）。
    pub fn prompt(&self, blocks: Vec<ContentBlock>, sink: &dyn TurnEventSink) -> ToolResult {
        let started = Instant::now();
        // §3.2.1 串行闸：同一 session 的并发 prompt 必须排队（不并发打同一 session）。
        let _gate = match self.prompt_gate.lock() {
            Ok(gate) => gate,
            Err(_) => {
                return self.error_result(
                    "acp_unavailable",
                    "会话串行闸已中毒（此前有 prompt 线程 panic）。".to_string(),
                    started,
                )
            }
        };

        // 会话已取消（本地标志或宿主令牌）：不再向 agent 发 prompt（fail-closed 归约为 aborted）。
        if self.cancellation_requested() {
            return self.outcome_result(
                AcpTurnOutcome::Cancelled,
                StopReason::Cancelled,
                &[],
                &[],
                started,
            );
        }

        // B162 实时语义：读线程回调经 mpsc 把 `SessionUpdate` 送回本线程，本线程
        // `recv_timeout` 逐条 `emit_update`（**不**等 prompt 返回后批量 flush）；同一循环按
        // `turn_cancel_poll_ms`(≤100ms) 轮询取消令牌（§3.2.6），触发即发 `session/cancel`。
        let (update_tx, update_rx) = mpsc::channel::<SessionUpdate>();
        let warnings = Arc::new(Mutex::new(Vec::<String>::new()));
        let observer = Arc::new(UpdateForwarder {
            tx: update_tx,
            warnings: Arc::clone(&warnings),
        }) as Arc<dyn AcpNotificationHandler>;
        let mut updates: Vec<SessionUpdate> = Vec::new();
        let (prompt_result, cancel_observed) = std::thread::scope(|scope| {
            let handle =
                scope.spawn(move || self.client.prompt(&self.session_id, blocks, observer));
            let mut cancel_sent = false;
            let mut cancel_observed = false;
            loop {
                match update_rx.recv_timeout(CANCEL_POLL_INTERVAL) {
                    Ok(update) => {
                        // 一帧即映射：委派期间 UI 立即看到 turn:delta / turn:tool。
                        self.emit_update(sink, &update);
                        updates.push(update);
                        continue;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        // prompt 线程结束且发送端已全部 drop：抽干剩余帧后退出循环。
                        while let Ok(update) = update_rx.try_recv() {
                            self.emit_update(sink, &update);
                            updates.push(update);
                        }
                        break;
                    }
                }
                // ⚠️ 终止条件必须看「prompt 线程是否结束」，不能只依赖通道断开：
                // transport 持有 observer 的 `Arc`（直到下一次 prompt 替换 handler），
                // 发送端因此**不会** drop ⇒ `recv_timeout` 永不返回 Disconnected。
                // 这里先抽干已入队帧（不丢最后一条 update），再退出循环。
                if handle.is_finished() {
                    while let Ok(update) = update_rx.try_recv() {
                        self.emit_update(sink, &update);
                        updates.push(update);
                    }
                    break;
                }
                // 每一片都重读槽位：`set_cancellation_token` 是 `&self` 热替换，prompt 运行期
                // 注入的令牌也必须被同一个轮询观察到（不能只读启动时的快照）。
                if !cancel_sent && self.cancellation_requested() {
                    cancel_sent = true;
                    cancel_observed = true;
                    if let Err(error) = self.client.cancel(&self.session_id) {
                        log_json_with_fields(
                            "warn",
                            "acp session/cancel 发送失败（取消仍按 aborted 归约）",
                            None,
                            &[("error", json!(error.to_string()))],
                        );
                    }
                }
            }
            (handle.join(), cancel_observed)
        });
        let response = match prompt_result {
            Ok(result) => result,
            // prompt 线程 panic：fail-closed 为不可用（不 panic 传播，不伪造成功）。
            Err(_) => Err(AcpTransportError::Unavailable(
                "session/prompt 执行线程 panic。".to_string(),
            )),
        };
        let warnings = warnings
            .lock()
            .map(|mut warnings| std::mem::take(&mut *warnings))
            .unwrap_or_default();

        match response {
            Ok(response) => self.outcome_result(
                Self::reduce_stop_reason(response.stop_reason),
                response.stop_reason,
                &updates,
                &warnings,
                started,
            ),
            // 取消已触发但 agent 不配合（根本没回包 ⇒ 传输层 timeout/断链）：归因优先于传输层
            // 错误分类，必须归约 aborted + tool_cancelled，不得退化成 acp_timeout。
            Err(error) if cancel_observed || self.cancellation_requested() => {
                log_json_with_fields(
                    "warn",
                    "acp 取消已触发但 agent 未回 cancelled，按 aborted 归约",
                    None,
                    &[("error", json!(error.to_string()))],
                );
                self.outcome_result(
                    AcpTurnOutcome::Cancelled,
                    StopReason::Cancelled,
                    &updates,
                    &warnings,
                    started,
                )
            }
            // 传输层失败：保留 §3.2.9 的 `acp_*` 错误码来源，不塌缩成通用错误。
            Err(error) => self.error_result(error.code(), error.to_string(), started),
        }
    }

    /// 取消：`session/cancel` notification + 挂起权限请求全部回 `cancelled`（§3.2.6）。
    pub fn cancel(&self) -> Result<(), AcpTransportError> {
        // 1) 只翻**本地**取消标志：本方法取消的是本会话的一次委派，**不得** cancel 宿主注入的
        //    共享 turn 令牌（那会把整个 turn 连带取消）；注入令牌全程只读（`is_cancelled` 观察）。
        self.local_cancel.store(true, Ordering::Release);
        // 2) 挂起审批全部取消，让 `resolve_permission` 立即归约成 cancelled。
        if let Ok(slots) = self.pending_permissions.lock() {
            for slot in slots.iter() {
                slot.request_cancel();
            }
        }
        // 3) 协议层取消通知（A2：notification，无 id）。
        self.client.cancel(&self.session_id)
    }

    /// 关闭会话并回收子进程（幂等）。
    pub fn close(&mut self) {
        self.client.close();
    }

    /// `session/request_permission` → `ask_control` 往返（§3.2.3）。
    ///
    /// 注册一条 `PendingControlRequestKind::Approval`，等宿主经真实
    /// `GovernedDispatcher::approve_control_request` / `cancel_control_request` 消费后回
    /// `{outcome:{outcome:"selected",optionId}}` / `{outcome:{outcome:"cancelled"}}`。
    /// **绝不**默认 allow；超时 / 会话取消 / 请求消失 ⇒ `cancelled`（fail-closed）。
    pub fn resolve_permission(
        &self,
        request: &RequestPermissionRequest,
        dispatcher: &GovernedDispatcher,
        authorization: &ControlRequestAuthorization,
    ) -> Result<RequestPermissionResponse, String> {
        let slot = Arc::new(PermissionSlot::new());
        let request_id = format!(
            "acp-permission-{}-{}",
            self.session_id,
            uuid::Uuid::new_v4()
        );
        // 与 `DispatcherPermissionHandler::decide` 共用同一注册路径（单一真源）。
        let pending = register_approval(dispatcher, request, &request_id);
        if let Ok(mut slots) = self.pending_permissions.lock() {
            slots.push(Arc::clone(&slot));
        }

        let outcome =
            self.await_permission_decision(dispatcher, authorization, &pending, request, &slot);

        if let Ok(mut slots) = self.pending_permissions.lock() {
            slots.retain(|candidate| !Arc::ptr_eq(candidate, &slot));
        }
        outcome
    }

    /// `session/update` → Pony 事件映射（§3.2.7）。
    ///
    /// - `agent_message_chunk` → `turn:delta`（text）
    /// - `agent_thought_chunk` → `turn:delta`（reasoning_content）
    /// - `tool_call` / `tool_call_update` → `turn:tool`（`TurnToolActivity`）
    /// - `plan` → `turn:tool` 子活动
    /// - `current_mode_update` / `available_commands_update` → 结构化日志（不进 UI）
    pub fn emit_update(&self, sink: &dyn TurnEventSink, update: &SessionUpdate) {
        match update {
            SessionUpdate::AgentMessageChunk { content, .. } => {
                if let Some(text) = content_text(content) {
                    sink.emit("turn:delta", self.delta_event(Some(text), None));
                }
            }
            SessionUpdate::AgentThoughtChunk { content, .. } => {
                if let Some(text) = content_text(content) {
                    sink.emit("turn:delta", self.delta_event(None, Some(text)));
                }
            }
            SessionUpdate::UserMessageChunk { .. } => {
                // 用户消息回显不进 UI（Pony 侧已有自己的 user 消息源）。
            }
            SessionUpdate::ToolCall {
                tool_call_id,
                title,
                kind,
                status,
                locations,
                raw_input,
                raw_output,
                ..
            } => {
                let activity = tool_activity(
                    tool_call_id,
                    title,
                    kind.as_ref(),
                    status.as_ref(),
                    Some(locations.as_slice()),
                    raw_input.as_ref(),
                    raw_output.as_ref(),
                );
                sink.emit("turn:tool", self.tool_event(activity));
                // wave-acp-delegate C1.1：同一活动以**父 turn 身份**归集（无父上下文 ⇒ 静默跳过）。
                self.relay_parent_tool_activity(
                    tool_call_id,
                    title,
                    kind.as_ref(),
                    status.as_ref(),
                    Some(locations.as_slice()),
                    raw_input.as_ref(),
                    raw_output.as_ref(),
                );
            }
            SessionUpdate::ToolCallUpdate {
                tool_call_id,
                title,
                kind,
                status,
                locations,
                raw_input,
                raw_output,
                ..
            } => {
                let title = title.as_deref().unwrap_or("ACP tool_call_update");
                let activity = tool_activity(
                    tool_call_id,
                    title,
                    kind.as_ref(),
                    status.as_ref(),
                    locations.as_deref(),
                    raw_input.as_ref(),
                    raw_output.as_ref(),
                );
                sink.emit("turn:tool", self.tool_event(activity));
                // wave-acp-delegate C1.1：父 turn 归集（与本地 sink 同一活动、不同词表）。
                self.relay_parent_tool_activity(
                    tool_call_id,
                    title,
                    kind.as_ref(),
                    status.as_ref(),
                    locations.as_deref(),
                    raw_input.as_ref(),
                    raw_output.as_ref(),
                );
            }
            SessionUpdate::Plan { entries } => {
                let summary = entries
                    .iter()
                    .map(|entry| entry.content.clone())
                    .collect::<Vec<_>>()
                    .join(" | ");
                let activity = TurnToolActivity {
                    id: format!("acp-plan:{}", self.session_id),
                    name: "plan".to_string(),
                    canonical_tool_name: None,
                    display_name_zh: Some("外部代理计划".to_string()),
                    status: "in_progress".to_string(),
                    description: summary,
                    arguments_text: None,
                    result_text: None,
                    duration_seconds: None,
                    parent_activity_id: None,
                    artifacts: None,
                    error: None,
                    capability_invocation: None,
                };
                sink.emit("turn:tool", self.tool_event(activity));
            }
            SessionUpdate::CurrentModeUpdate { current_mode_id } => {
                log_json_with_fields(
                    "info",
                    "acp current_mode_update",
                    None,
                    &[("current_mode_id", json!(current_mode_id))],
                );
            }
            SessionUpdate::AvailableCommandsUpdate { available_commands } => {
                log_json_with_fields(
                    "info",
                    "acp available_commands_update",
                    None,
                    &[("command_count", json!(available_commands.len()))],
                );
            }
            // B166：余下 6 个官方变体 + 未知兜底。按 §3.2.7 口径，这些 update 不进 UI 事件流，
            // 只做结构化诊断日志（usage/cost 后续可映射 telemetry；notice/compaction 待 P2 UI）。
            SessionUpdate::ConfigOptionUpdate(update) => {
                log_json_with_fields(
                    "info",
                    "acp config_option_update",
                    None,
                    &[("config_option_count", json!(update.config_options.len()))],
                );
            }
            SessionUpdate::SessionInfoUpdate(update) => {
                log_json_with_fields(
                    "info",
                    "acp session_info_update",
                    None,
                    &[
                        ("title", json!(update.title)),
                        ("updated_at", json!(update.updated_at)),
                    ],
                );
            }
            SessionUpdate::UsageUpdate(update) => {
                // usage→turn 事件映射位置：本 arm 是唯一入口；当前为结构化日志（不进 UI），
                // 保留 used/size/cost 供 P1 telemetry/预算消费。
                // C2.1：用量快照结构化落日志（数值保持 JSON number，供指标化采集）；
                // 结果 JSON 的 `usage` 由 `acp::result::reduce_usage` 在终局归约（单一真源）。
                let mut fields = vec![("used", json!(update.used)), ("size", json!(update.size))];
                if let Some(cost) = &update.cost {
                    fields.push(("cost_amount", json!(cost.amount)));
                    fields.push(("cost_currency", json!(cost.currency)));
                }
                log_json_with_fields("info", "acp usage_update", None, &fields);
            }
            SessionUpdate::Notice(notice) => {
                log_json_with_fields(
                    "info",
                    "acp notice",
                    None,
                    &[
                        ("severity", json!(notice.severity)),
                        ("title", json!(notice.title)),
                        ("description", json!(notice.description)),
                    ],
                );
            }
            SessionUpdate::CompactionUpdate(update) => {
                log_json_with_fields(
                    "info",
                    "acp compaction_update",
                    None,
                    &[
                        ("compaction_id", json!(update.compaction_id)),
                        ("status", json!(update.status)),
                    ],
                );
            }
            SessionUpdate::CompactionSummaryChunk(update) => {
                log_json_with_fields(
                    "info",
                    "acp compaction_summary_chunk",
                    None,
                    &[("compaction_id", json!(update.compaction_id))],
                );
            }
            // 未知/厂商私有 sessionUpdate：已由 `#[serde(other)]` 兜底反序列化，这里只记诊断，
            // 绝不 panic、绝不整条丢弃（B166）。
            SessionUpdate::Unknown => {
                log_json(
                    "warn",
                    "acp 未知 sessionUpdate 变体（已降级诊断，不中断本轮流）",
                    None,
                );
            }
        }
    }

    /// 把一条 ACP 工具活动归集为**父 turn** 的 `turn:tool`（C1.1–C1.5）。
    ///
    /// - 状态词表走 [`map_acp_tool_status`]（C1.4 单一真源），与本地 sink 的 ACP 词表分离：
    ///   本地事件保持 `pending|in_progress|completed|failed`（既有语义不变），父 turn 事件用
    ///   `running|done|failed|aborted`（父 turn 据此判 ToolCall/ToolResult）；
    /// - 无父上下文（未注入 / 三要素不完整）⇒ 直接返回，静默跳过（C1.3）；
    /// - 文本 chunk 不走本方法（C1.2：外部正文不得混入父 assistant 正文流）。
    fn relay_parent_tool_activity(
        &self,
        tool_call_id: &str,
        title: &str,
        kind: Option<&ToolKind>,
        status: Option<&ToolCallStatus>,
        locations: Option<&[ToolCallLocation]>,
        raw_input: Option<&Value>,
        raw_output: Option<&Value>,
    ) {
        let parent = self.parent_turn.lock().ok().and_then(|slot| slot.clone());
        let Some(parent) = parent else {
            return;
        };
        let activity = tool_activity_with_status(
            tool_call_id,
            title,
            kind,
            map_acp_tool_status(status),
            locations,
            raw_input,
            raw_output,
        );
        crate::acp::delegate_events::emit_parent_tool_activity(&parent, &activity);
    }

    /// stopReason → Pony 归约（§3.2.8）。纯函数，无 I/O。
    pub fn reduce_stop_reason(stop: StopReason) -> AcpTurnOutcome {
        match stop {
            StopReason::EndTurn => AcpTurnOutcome::Ok,
            StopReason::MaxTokens | StopReason::MaxTurnRequests => AcpTurnOutcome::Truncated,
            StopReason::Refusal => AcpTurnOutcome::Refused,
            StopReason::Cancelled => AcpTurnOutcome::Cancelled,
        }
    }

    /// 会话 id（`acp_delegate` 的诊断面）。
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 事件骨架（`turn_id` 用 ACP 会话 id 占位：ACP 没有 Pony 的 turn id）。
    fn base_event(&self, kind: &str, phase: Option<&str>) -> TurnStreamEvent {
        TurnStreamEvent {
            event_id: None,
            session_id: Some(self.session_id.clone()),
            turn_id: self.session_id.clone(),
            trace_id: crate::agent::otel::exported_trace_id(),
            cost_usd: None,
            kind: kind.to_string(),
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: phase.map(str::to_string),
            text: None,
            reasoning_content: None,
            error: None,
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            build_context_observation: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            trace_steps: None,
            trace_timeline: None,
            tool_activities: None,
            provider_call_records: None,
            hook_trace_records: None,
            session_summary: None,
            step: None,
            stop_reason: None,
            metrics: None,
            error_class: None,
        }
    }

    fn delta_event(&self, text: Option<String>, reasoning: Option<String>) -> TurnStreamEvent {
        let mut event = self.base_event("delta", Some("calling_model"));
        event.text = text;
        event.reasoning_content = reasoning;
        event
    }

    fn tool_event(&self, activity: TurnToolActivity) -> TurnStreamEvent {
        let mut event = self.base_event("tool", Some("executing_tool"));
        event.tool_activities = Some(vec![activity]);
        event
    }

    /// 宿主注入的共享令牌是否已取消（**只读**：本会话从不 cancel 它）。
    fn session_cancelled(&self) -> bool {
        self.cancellation
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|token| token.is_cancelled()))
            .unwrap_or(false)
    }

    /// 本轮是否已请求取消：本地 `cancel()` 标志或宿主令牌（任一成立即成立）。
    fn cancellation_requested(&self) -> bool {
        self.local_cancel.load(Ordering::Acquire) || self.session_cancelled()
    }

    /// 等宿主决定：有界条件轮询（5ms）+ 取消标志 + NFR 超时；任何非批准终态 ⇒ cancelled。
    fn await_permission_decision(
        &self,
        dispatcher: &GovernedDispatcher,
        authorization: &ControlRequestAuthorization,
        pending: &PendingControlRequest,
        request: &RequestPermissionRequest,
        slot: &PermissionSlot,
    ) -> Result<RequestPermissionResponse, String> {
        // 单一判定真源：`await_approval`（与 DispatcherPermissionHandler 共用）。
        await_approval(dispatcher, Some(authorization), pending, request, &|| {
            slot.is_cancel_requested() || self.cancellation_requested()
        })
    }

    /// 归约结果 → `ToolResult`（`ok` / `truncated` / `refused` / `cancelled`）。
    fn outcome_result(
        &self,
        outcome: AcpTurnOutcome,
        stop_reason: StopReason,
        updates: &[SessionUpdate],
        warnings: &[String],
        started: Instant,
    ) -> ToolResult {
        let stop_reason_text = serde_json::to_value(stop_reason)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{stop_reason:?}"));
        let labels = Labels {
            agent: self.client.agent_id().to_string(),
            session_id: self.session_id.clone(),
            stop_reason: stop_reason_text,
            updates: updates.len(),
            warnings: warnings.to_vec(),
        };
        // 正文 / 用量归约与是否「成功轮」解耦：**所有**成功面（`end_turn` 与 stopReason 上限收尾的
        // `max_tokens`/`max_turn_requests`）都必须带回正文，否则模型调用方拿到「成功但没有内容」
        // 的结果就只能凭 summary 猜（Gate-1 P0 语义修正）。失败面（refused/cancelled/error）
        // 一律走 `build_failure_result`，**不**产 `finalText`。
        let stop_reason_limit_hit = matches!(outcome, AcpTurnOutcome::Truncated);
        let final_text = reduce_final_text(updates);
        let usage = reduce_usage(updates);
        match outcome {
            AcpTurnOutcome::Ok | AcpTurnOutcome::Truncated => build_ok_result_with_final_text(
                &labels,
                &final_text,
                usage.as_ref(),
                stop_reason_limit_hit,
                started,
            ),
            AcpTurnOutcome::Refused => build_failure_result(
                &labels,
                "error",
                ACP_ERROR_AGENT_REFUSED,
                "外部 agent 拒绝执行该任务（stopReason: refusal）。".to_string(),
                started,
            ),
            AcpTurnOutcome::Cancelled => build_failure_result(
                &labels,
                "aborted",
                ACP_TOOL_CANCELLED,
                "外部 agent 轮次已被取消（stopReason: cancelled）。".to_string(),
                started,
            ),
        }
    }

    /// 传输层 / 会话层失败 → `ToolResult`（保留 `acp_*` 错误码来源，不塌缩）。
    fn error_result(&self, code: &str, message: String, started: Instant) -> ToolResult {
        let labels = Labels {
            agent: self.client.agent_id().to_string(),
            session_id: self.session_id.clone(),
            stop_reason: "n/a".to_string(),
            updates: 0,
            warnings: Vec::new(),
        };
        build_failure_result(&labels, "error", code, message, started)
    }
}

/// 归约输出共用的标签面（避免在多个构造函数里重复取字段）。
struct Labels {
    agent: String,
    session_id: String,
    stop_reason: String,
    updates: usize,
    warnings: Vec<String>,
}

/// 成功面结果 + 归约正文 / 用量（C0.1 / C0.2 / C0.3 / C2.1，additive）。
///
/// 覆盖两种成功轮：`end_turn`（`stop_reason_limit_hit=false`）与 stopReason 上限收尾的
/// `max_tokens`/`max_turn_requests`（`stop_reason_limit_hit=true`，既有 `truncated: true` 语义）。
///
/// 键面 = 既有成功键（`ok/tool/agent/sessionId/stopReason/truncated/updates/warnings/summary`）
/// + `finalText`（string）+ `finalTextTruncated`（bool）+ 有 `usage_update` 时的 `usage`。
/// `finalTextTruncated` 语义 = **正文不完整**：本侧 32k 截断，或外部 agent 因输出/轮次上限提前收尾。
/// 空正文时 `summary.text` 必须明示「未返回文本正文」，不得复用「已完成本轮任务」让调用方误判。
fn build_ok_result_with_final_text(
    labels: &Labels,
    final_text: &DelegateFinalText,
    usage: Option<&DelegateUsage>,
    stop_reason_limit_hit: bool,
    started: Instant,
) -> ToolResult {
    // 摘要优先级：空正文（C0.2 诚实提示）> stopReason 上限（C0.4 既有文案逐字保留）
    // > 本侧 32k 截断 > 正常完成。
    let summary = if final_text.text.is_empty() {
        format!(
            "外部 agent 未返回文本正文（updates={}），无可用内容可归约。",
            labels.updates
        )
    } else if stop_reason_limit_hit {
        "外部 agent 达到输出/轮次上限，结果可能被截断（stopReason 非 end_turn）。".to_string()
    } else if final_text.truncated {
        format!(
            "外部 agent 已完成本轮任务；正文超过 {ACP_DELEGATE_FINAL_TEXT_MAX_BYTES} 字节上限，已保留前缀。"
        )
    } else {
        "外部 agent 已完成本轮任务。".to_string()
    };
    let mut payload = serde_json::Map::new();
    payload.insert("ok".to_string(), json!(true));
    payload.insert("tool".to_string(), json!(TOOL_ACP_DELEGATE));
    payload.insert("agent".to_string(), json!(labels.agent));
    payload.insert("sessionId".to_string(), json!(labels.session_id));
    payload.insert("stopReason".to_string(), json!(labels.stop_reason));
    // 既有 `truncated` 键语义不变：stopReason 是否撞上限（max_tokens/max_turn_requests）。
    payload.insert("truncated".to_string(), json!(stop_reason_limit_hit));
    payload.insert("updates".to_string(), json!(labels.updates));
    payload.insert("warnings".to_string(), json!(labels.warnings));
    payload.insert("summary".to_string(), json!({ "text": summary }));
    // C0.1/C0.2/C0.3：正文恒存在（无正文 ⇒ 空串），截断标志恒为布尔。
    payload.insert("finalText".to_string(), json!(final_text.text));
    payload.insert(
        "finalTextTruncated".to_string(),
        json!(final_text.truncated || stop_reason_limit_hit),
    );
    // C2.1：无 usage_update ⇒ 键缺席（不用 null/0 占位，避免伪造用量）。
    if let Some(usage) = usage {
        payload.insert("usage".to_string(), usage.to_json());
    }
    ToolResult {
        tool_name: TOOL_ACP_DELEGATE.to_string(),
        status: "ok".to_string(),
        output: serde_json::to_string(&Value::Object(payload)).unwrap_or_else(|error| {
            format!("{{\"ok\":true,\"tool\":\"acp_delegate\",\"warnings\":[\"序列化输出失败：{error}\"]}}")
        }),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn build_failure_result(
    labels: &Labels,
    status: &str,
    code: &str,
    message: String,
    started: Instant,
) -> ToolResult {
    ToolResult {
        tool_name: TOOL_ACP_DELEGATE.to_string(),
        status: status.to_string(),
        output: serde_json::to_string(&json!({
            "ok": false,
            "tool": TOOL_ACP_DELEGATE,
            "agent": labels.agent,
            "sessionId": labels.session_id,
            "stopReason": labels.stop_reason,
            "updates": labels.updates,
            "warnings": labels.warnings,
            "error": {
                "code": code,
                "kind": code,
                "message": message,
                "hint": Value::Null,
            },
            "summary": { "text": message },
        }))
        .unwrap_or_else(|error| {
            format!("{{\"ok\":false,\"tool\":\"acp_delegate\",\"error\":{{\"code\":\"{code}\",\"kind\":\"{code}\",\"message\":\"序列化输出失败：{error}\"}}}}")
        }),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// 取消一个仍待决的审批（CAS 同源：从当前快照取 session/version/nonce）。
fn settle_cancel(dispatcher: &GovernedDispatcher, request_id: &str) {
    let Some(current) = dispatcher.pending_request(request_id) else {
        return;
    };
    if current.state != PendingControlRequestState::Pending {
        return;
    }
    let cas = ControlRequestAuthorization {
        session_id: current.session_id.clone(),
        expected_version: current.version,
        nonce: current.nonce.clone(),
        ..Default::default()
    };
    if let Err(error) = dispatcher.cancel_control_request(request_id, &cas) {
        // 不吞：CAS 冲突说明宿主抢先在消费，记录诊断；调用方仍按 fail-closed 回 cancelled。
        log_json_with_fields(
            "warn",
            "acp 置 Cancelled 失败（CAS 冲突：宿主可能抢先在消费）",
            None,
            &[
                ("request_id", json!(request_id)),
                ("error", json!(error.to_string())),
            ],
        );
    }
}

/// 批准 → `selected`：optionId 优先取宿主 answer 里的显式选择，否则取第一个 allow_* 选项。
/// **绝不**凭空构造 options 之外的 optionId。
fn selected_response(
    request: &RequestPermissionRequest,
    authorization: Option<&ControlRequestAuthorization>,
) -> Result<RequestPermissionResponse, String> {
    let from_answer = authorization
        .and_then(|authorization| authorization.answer.as_ref())
        .and_then(Value::as_object)
        .and_then(|answer| answer.get("optionId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let option_id = from_answer
        .filter(|candidate| {
            request
                .options
                .iter()
                .any(|option| &option.option_id == candidate)
        })
        .or_else(|| {
            request
                .options
                .iter()
                .find(|option| {
                    matches!(
                        option.kind,
                        PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                    )
                })
                .or_else(|| request.options.first())
                .map(|option| option.option_id.clone())
        });
    match option_id {
        Some(option_id) => Ok(RequestPermissionResponse {
            outcome: RequestPermissionOutcome::Selected { option_id },
        }),
        // 请求里没有任何可选 option：无法表达批准 ⇒ fail-closed cancelled。
        None => Ok(cancelled_response()),
    }
}

fn cancelled_response() -> RequestPermissionResponse {
    RequestPermissionResponse {
        outcome: RequestPermissionOutcome::Cancelled,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// 内容块 → 纯文本（非文本块不进 `turn:delta`：图片/资源由 `turn:tool` 侧体现）。
fn content_text(content: &ContentBlock) -> Option<String> {
    match content {
        ContentBlock::Text { text } => Some(text.clone()),
        other => {
            // 不静默：非文本块记结构化诊断（Pony 侧当前无富内容渲染通道）。
            log_json_with_fields(
                "info",
                "acp 非文本内容块不映射为 turn:delta（当前无富内容渲染通道）",
                None,
                &[(
                    "content",
                    serde_json::to_value(other).unwrap_or(Value::Null),
                )],
            );
            None
        }
    }
}

/// ACP 工具调用 → `TurnToolActivity`（§3.2.7 的 `turn:tool` 载荷）。
fn tool_activity(
    tool_call_id: &str,
    title: &str,
    kind: Option<&ToolKind>,
    status: Option<&ToolCallStatus>,
    locations: Option<&[ToolCallLocation]>,
    raw_input: Option<&Value>,
    raw_output: Option<&Value>,
) -> TurnToolActivity {
    // 本地 sink 沿用 ACP 原生词表（既有语义不变）；父 turn 归集走 `map_acp_tool_status`。
    tool_activity_with_status(
        tool_call_id,
        title,
        kind,
        status.map(tool_status_str).unwrap_or("pending"),
        locations,
        raw_input,
        raw_output,
    )
}

/// [`tool_activity`] 的「状态词表外置」版：同一载荷构造，状态串由调用方按通道词表决定。
fn tool_activity_with_status(
    tool_call_id: &str,
    title: &str,
    kind: Option<&ToolKind>,
    status: &str,
    locations: Option<&[ToolCallLocation]>,
    raw_input: Option<&Value>,
    raw_output: Option<&Value>,
) -> TurnToolActivity {
    TurnToolActivity {
        id: tool_call_id.to_string(),
        name: title.to_string(),
        canonical_tool_name: None,
        // 外部 agent 的工具体系与 Pony 隔离（方案 B 边界），展示名标注来源。
        display_name_zh: Some("外部代理工具".to_string()),
        status: status.to_string(),
        description: describe_tool_call(title, kind, locations),
        arguments_text: raw_input.map(json_text),
        result_text: raw_output.map(json_text),
        duration_seconds: None,
        parent_activity_id: None,
        artifacts: None,
        error: None,
        capability_invocation: None,
    }
}

fn tool_status_str(status: &ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "pending",
        ToolCallStatus::InProgress => "in_progress",
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
        // C1.4 additive：ACP `cancelled` 不再是「反序列化失败整帧丢弃」。
        ToolCallStatus::Cancelled => "cancelled",
        // B240：未知状态兜底（`#[serde(other)]`）——原样透出 `unknown`，
        // 不臆造成 completed/failed 任何一方的终态。
        ToolCallStatus::Unknown => "unknown",
    }
}

/// 可读描述：标题 + 工具类别 + 受影响路径（让「外部 agent 改了什么」在 trace 里可见）。
fn describe_tool_call(
    title: &str,
    kind: Option<&ToolKind>,
    locations: Option<&[ToolCallLocation]>,
) -> String {
    let mut description = title.to_string();
    if let Some(kind) = kind {
        description.push_str(&format!("（{}）", tool_kind_str(kind)));
    }
    if let Some(locations) = locations.filter(|locations| !locations.is_empty()) {
        let paths = locations
            .iter()
            .map(|location| location.path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        description.push_str(&format!(" @ {paths}"));
    }
    description
}

fn tool_kind_str(kind: &ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch_mode",
        ToolKind::Other => "other",
    }
}

fn json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| format!("<unserializable:{error}>"))
}
