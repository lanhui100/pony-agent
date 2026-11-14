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

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::acp::client::{AcpAgentClient, PermissionHandler};
use crate::acp::config::AcpAgentSpec;
use crate::acp::protocol::{
    method, ContentBlock, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SessionNotification, SessionUpdate, StopReason, ToolCallLocation,
    ToolCallStatus, ToolKind,
};
use crate::acp::transport::{AcpNotificationHandler, AcpTransportError, ACP_REQUEST_TIMEOUT};
use crate::agent::ask_persistence::{AskLifecycleState, PersistedAsk};
use crate::agent::budget::CancellationToken;
use crate::agent::dispatcher::{ControlRequestAuthorization, GovernedDispatcher};
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

impl PermissionHandler for FailClosedPermissionHandler {
    fn decide(&self, request: &RequestPermissionRequest) -> RequestPermissionResponse {
        eprintln!(
            "[acp][session] 无宿主审批 dispatcher 绑定，session/request_permission(toolCallId={}) fail-closed：cancelled",
            request.tool_call.tool_call_id
        );
        RequestPermissionResponse {
            outcome: RequestPermissionOutcome::Cancelled,
        }
    }
}

/// prompt 期间的 `session/update` 收集器。
///
/// `TurnEventSink` 的冻结签名不带 `Send + Sync`，而通知回调来自 transport 的读线程：因此
/// 不能在回调里直接 `sink.emit`。这里先按到达顺序收集，prompt 返回后在调用方线程统一映射
/// 上行（事件顺序与载荷完整保留，不丢帧）；driver 侧如需真流式，应引入 `Sync` sink 面。
#[derive(Default)]
struct UpdateCollector {
    updates: Mutex<Vec<SessionUpdate>>,
    warnings: Mutex<Vec<String>>,
}

impl UpdateCollector {
    fn record_warning(&self, warning: String) {
        if let Ok(mut warnings) = self.warnings.lock() {
            warnings.push(warning);
        }
    }

    fn take(&self) -> (Vec<SessionUpdate>, Vec<String>) {
        let updates = self
            .updates
            .lock()
            .map(|mut updates| std::mem::take(&mut *updates))
            .unwrap_or_default();
        let warnings = self
            .warnings
            .lock()
            .map(|mut warnings| std::mem::take(&mut *warnings))
            .unwrap_or_default();
        (updates, warnings)
    }
}

impl AcpNotificationHandler for UpdateCollector {
    fn handle_notification(&self, method: &str, params: Value) {
        if method != method::SESSION_UPDATE {
            // 非 update 通知不静默：结构化日志 + 记一条 warning（不进 UI 事件流）。
            let warning = format!("acp_unexpected_notification: `{method}`");
            eprintln!("[acp][session] 收到未处理的入站通知 `{method}`：{params}");
            self.record_warning(warning);
            return;
        }
        match serde_json::from_value::<SessionNotification>(params) {
            Ok(notification) => {
                if let Ok(mut updates) = self.updates.lock() {
                    updates.push(notification.update);
                }
            }
            Err(error) => {
                // 载荷不合 v1 schema：记 warning 并继续（不 panic、不静默）。
                self.record_warning(format!(
                    "acp_protocol_error: session/update 载荷不符合 v1 schema：{error}"
                ));
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
}

impl AcpAgentSession {
    /// 建会话：`AcpAgentClient::connect` → `install_handlers` → `new_session`。
    ///
    /// `cwd` 必须是绝对路径（从 `ToolExecutionContext.workspace_root` 取）。
    pub fn spawn(spec: &AcpAgentSpec, cwd: &Path) -> Result<Self, AcpTransportError> {
        // 握手（含 protocolVersion == 1 的 fail-closed 校验）由 client 收口。
        let mut client = AcpAgentClient::connect(spec, cwd)?;
        // fs/terminal 一律不下放（None）：没有治理链 handler 就不声明能力（§3.2.4/§3.2.5）。
        client.install_handlers(Arc::new(FailClosedPermissionHandler), None, None);
        let response = client.new_session(cwd, Vec::new())?;
        Ok(Self {
            client,
            prompt_gate: Mutex::new(()),
            session_id: response.session_id,
            cancellation: Mutex::new(Some(Arc::new(CancellationToken::new()))),
            pending_permissions: Mutex::new(Vec::new()),
        })
    }

    /// 设置整轮 prompt 预算（additive；转发给 [`AcpAgentClient::set_prompt_budget`]）。
    ///
    /// 默认值 = `ACP_REQUEST_TIMEOUT`（5000ms）。整轮委派预算（`timeoutMs` /
    /// `--timeout-secs` / 执行策略）必须经本方法进入会话，否则真实 agent 的 >5s 一轮会被硬顶失败。
    pub fn set_turn_budget(&self, budget: Duration) {
        self.client.set_prompt_budget(budget);
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

    /// 跑一轮 prompt：串行闸 → `client.prompt` → update 实时映射（[`Self::emit_update`]）→
    /// [`Self::reduce_stop_reason`] 归约成 [`ToolResult`]。
    ///
    /// 预算：`client.prompt` 的 deadline 取 [`AcpAgentSession::set_turn_budget`]（默认
    /// `ACP_REQUEST_TIMEOUT`），**不是**固定 5s 硬顶。
    /// 取消：prompt 期间轮询 [`AcpAgentSession::set_cancellation_token`] 的令牌。
    /// 归约口径：`ok` / `truncated` 仍为成功；`refused` → error `acp_agent_refused`；
    /// `cancelled` → aborted `tool_cancelled`。
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

        // 会话已取消：不再向 agent 发 prompt（fail-closed 归约为 aborted）。
        if self.session_cancelled() {
            return self.outcome_result(
                AcpTurnOutcome::Cancelled,
                StopReason::Cancelled,
                &[],
                &[],
                started,
            );
        }

        let collector = Arc::new(UpdateCollector::default());
        let observer = Arc::clone(&collector) as Arc<dyn AcpNotificationHandler>;

        // §3.2.6 取消联动：`client.prompt` 在协议层阻塞等 agent 应答，因此把 prompt 放进行内
        // 线程，本线程按 `turn_cancel_poll_ms`(≤100ms) 轮询取消令牌；一旦触发即发
        // `session/cancel` notification（agent 随后以 `stopReason: cancelled` 回应本轮），
        // 从而不必等满整轮预算。取消发送失败只记诊断，不改变 prompt 的归约结果。
        let prompt_result = std::thread::scope(|scope| {
            let handle =
                scope.spawn(move || self.client.prompt(&self.session_id, blocks, observer));
            let mut cancel_sent = false;
            while !handle.is_finished() {
                // 每一片都重读槽位：`set_cancellation_token` 是 `&self` 热替换，prompt 运行期
                // 注入的令牌也必须被同一个轮询观察到（不能只读启动时的快照）。
                if !cancel_sent && self.session_cancelled() {
                    cancel_sent = true;
                    if let Err(error) = self.client.cancel(&self.session_id) {
                        eprintln!(
                            "[acp][session] session/cancel 发送失败（取消仍按 aborted 归约）：{error}"
                        );
                    }
                }
                std::thread::sleep(CANCEL_POLL_INTERVAL);
            }
            handle.join()
        });
        let response = match prompt_result {
            Ok(result) => result,
            // prompt 线程 panic：fail-closed 为不可用（不 panic 传播，不伪造成功）。
            Err(_) => Err(AcpTransportError::Unavailable(
                "session/prompt 执行线程 panic。".to_string(),
            )),
        };

        // 先按到达顺序把 update 映射进 TurnEventSink，再归约 stopReason。
        let (updates, warnings) = collector.take();
        for update in &updates {
            self.emit_update(sink, update);
        }

        match response {
            Ok(response) => self.outcome_result(
                Self::reduce_stop_reason(response.stop_reason),
                response.stop_reason,
                &updates,
                &warnings,
                started,
            ),
            // 传输层失败：保留 §3.2.9 的 `acp_*` 错误码来源，不塌缩成通用错误。
            Err(error) => self.error_result(error.code(), error.to_string(), started),
        }
    }

    /// 取消：`session/cancel` notification + 挂起权限请求全部回 `cancelled`（§3.2.6）。
    pub fn cancel(&self) -> Result<(), AcpTransportError> {
        // 1) 先翻挂起审批的取消标志：让 `resolve_permission` 立即归约成 cancelled。
        if let Ok(slots) = self.pending_permissions.lock() {
            for slot in slots.iter() {
                slot.request_cancel();
            }
        }
        // 2) 取消令牌：prompt / 后续轮次立即观察（不再向 agent 发新请求）。
        if let Ok(slot) = self.cancellation.lock() {
            if let Some(token) = slot.as_ref() {
                token.cancel();
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
        // 用 dispatcher 的公开镜像seam 注册审批：进入与 `approve_control_request` /
        // `cancel_control_request` **同一张** CAS 表，审批不可被本层伪造或绕过。
        let record = PersistedAsk {
            request_id: request_id.clone(),
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
        let pending =
            dispatcher.hydrate_persisted_ask(&record, PendingControlRequestState::Pending);
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
                let activity = tool_activity(
                    tool_call_id,
                    title.as_deref().unwrap_or("ACP tool_call_update"),
                    kind.as_ref(),
                    status.as_ref(),
                    locations.as_deref(),
                    raw_input.as_ref(),
                    raw_output.as_ref(),
                );
                sink.emit("turn:tool", self.tool_event(activity));
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
                eprintln!("[acp][session] current_mode_update: mode={current_mode_id}");
            }
            SessionUpdate::AvailableCommandsUpdate { available_commands } => {
                eprintln!(
                    "[acp][session] available_commands_update: {} 条命令",
                    available_commands.len()
                );
            }
        }
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

    fn session_cancelled(&self) -> bool {
        self.cancellation
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|token| token.is_cancelled()))
            .unwrap_or(false)
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
                    eprintln!(
                        "[acp][session] 审批请求 `{}` 已从 dispatcher 消失，fail-closed：cancelled",
                        pending.request_id
                    );
                    return Ok(cancelled_response());
                }
            }

            if slot.is_cancel_requested() || self.session_cancelled() {
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
        match outcome {
            AcpTurnOutcome::Ok | AcpTurnOutcome::Truncated => {
                build_ok_result(&labels, outcome == AcpTurnOutcome::Truncated, started)
            }
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

fn build_ok_result(labels: &Labels, truncated: bool, started: Instant) -> ToolResult {
    let summary = if truncated {
        "外部 agent 达到输出/轮次上限，结果可能被截断（stopReason 非 end_turn）。"
    } else {
        "外部 agent 已完成本轮任务。"
    };
    ToolResult {
        tool_name: TOOL_ACP_DELEGATE.to_string(),
        status: "ok".to_string(),
        output: serde_json::to_string(&json!({
            "ok": true,
            "tool": TOOL_ACP_DELEGATE,
            "agent": labels.agent,
            "sessionId": labels.session_id,
            "stopReason": labels.stop_reason,
            "truncated": truncated,
            "updates": labels.updates,
            "warnings": labels.warnings,
            "summary": { "text": summary },
        }))
        .unwrap_or_else(|error| format!("{{\"ok\":true,\"tool\":\"acp_delegate\",\"warnings\":[\"序列化输出失败：{error}\"]}}")),
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
        eprintln!("[acp][session] 置 Cancelled 失败（{request_id}）：{error}");
    }
}

/// 批准 → `selected`：optionId 优先取宿主 answer 里的显式选择，否则取第一个 allow_* 选项。
/// **绝不**凭空构造 options 之外的 optionId。
fn selected_response(
    request: &RequestPermissionRequest,
    authorization: &ControlRequestAuthorization,
) -> Result<RequestPermissionResponse, String> {
    let from_answer = authorization
        .answer
        .as_ref()
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
            eprintln!(
                "[acp][session] 非文本内容块不映射为 turn:delta：{}",
                serde_json::to_string(other)
                    .unwrap_or_else(|error| format!("<unserializable:{error}>"))
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
    TurnToolActivity {
        id: tool_call_id.to_string(),
        name: title.to_string(),
        canonical_tool_name: None,
        // 外部 agent 的工具体系与 Pony 隔离（方案 B 边界），展示名标注来源。
        display_name_zh: Some("外部代理工具".to_string()),
        status: status.map(tool_status_str).unwrap_or("pending").to_string(),
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
