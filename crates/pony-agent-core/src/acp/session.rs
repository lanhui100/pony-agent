//! ACP 会话驱动 + 治理归约（P1 空桩）。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2.1（串行化）、§3.2.3（审批）、
//! §3.2.6（取消）、§3.2.7（事件映射）、§3.2.8（stopReason 归约）。
//!
//! P1 只冻结签名：所有函数体 `unimplemented!()`，由 executor 实现。本层是 Pony 治理链与外部
//! agent 的唯一接缝：**不得**绕过 `ask_control` / `path_permission` / `ProcessManager`。

#![allow(unused_variables)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::acp::client::AcpAgentClient;
use crate::acp::config::AcpAgentSpec;
use crate::acp::protocol::{
    ContentBlock, RequestPermissionRequest, RequestPermissionResponse, SessionUpdate, StopReason,
};
use crate::acp::transport::AcpTransportError;
use crate::agent::budget::CancellationToken;
use crate::agent::dispatcher::{ControlRequestAuthorization, GovernedDispatcher};
use crate::agent::tools::ToolResult;
use crate::agent::turn_flow::TurnEventSink;

/// `refusal` 归约后的错误码（§3.2.9 错误码分源）。
pub const ACP_ERROR_AGENT_REFUSED: &str = "acp_agent_refused";

/// `cancelled` 归约后的错误码（沿用既有 `tool_cancelled`，§3.2.8）。
pub const ACP_TOOL_CANCELLED: &str = "tool_cancelled";

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

/// 一条 ACP 会话（进程级），把外部 agent 的 update / stopReason / 反向请求映射进 Pony。
#[allow(dead_code)]
pub struct AcpAgentSession {
    client: AcpAgentClient,
    /// 会话串行闸（§3.2.1）：opencode 明确「一个 session 同时只能跑一个 prompt」。
    /// 并发委派必须走「每任务一个子进程」或排队，**不得**并发打同一 session。
    prompt_gate: Mutex<()>,
    session_id: String,
    /// 取消令牌（§3.2.6）：宿主 `stop_turn` 被接受后，先 `session/cancel` notification，
    /// 再把所有挂起 `session/request_permission` 回 `cancelled`。
    cancellation: Option<Arc<CancellationToken>>,
}

impl AcpAgentSession {
    /// 建会话：`AcpAgentClient::connect` → `install_handlers` → `new_session`。
    ///
    /// `cwd` 必须是绝对路径（从 `ToolExecutionContext.workspace_root` 取）。
    pub fn spawn(spec: &AcpAgentSpec, cwd: &Path) -> Result<Self, AcpTransportError> {
        unimplemented!("P1：connect + install_handlers + new_session + 初始化串行闸/取消令牌")
    }

    /// 跑一轮 prompt：串行闸 → `client.prompt` → update 实时映射（[`Self::emit_update`]）→
    /// [`Self::reduce_stop_reason`] 归约成 [`ToolResult`]。
    ///
    /// 归约口径：`ok` / `truncated` 仍为成功；`refused` → error `acp_agent_refused`；
    /// `cancelled` → aborted `tool_cancelled`。
    pub fn prompt(&self, blocks: Vec<ContentBlock>, sink: &dyn TurnEventSink) -> ToolResult {
        unimplemented!("P1：串行闸 + prompt + update→TurnEventSink + stopReason 归约")
    }

    /// 取消：`session/cancel` notification + 挂起权限请求全部回 `cancelled`（§3.2.6）。
    pub fn cancel(&self) -> Result<(), AcpTransportError> {
        unimplemented!("P1：client.cancel + 取消挂起 permission")
    }

    /// 关闭会话并回收子进程（幂等）。
    pub fn close(&mut self) {
        unimplemented!("P1：client.close()")
    }

    /// `session/request_permission` → `ask_control` 往返（§3.2.3）。
    ///
    /// 由 `PermissionHandler` 的会话实现调用：注册一条 `PendingControlRequestKind::Approval`，
    /// 经 Ask 面板拿到用户决定后，用 `ask_control::answer_ask(dispatcher, request_id, auth)` 消费，
    /// 再回 `{outcome:{outcome:"selected",optionId}}` / `{outcome:{outcome:"cancelled"}}`。
    /// **绝不**默认 allow；handler 不可用 / 用户取消 ⇒ `cancelled`（fail-closed）。
    pub fn resolve_permission(
        &self,
        request: &RequestPermissionRequest,
        dispatcher: &GovernedDispatcher,
        authorization: &ControlRequestAuthorization,
    ) -> Result<RequestPermissionResponse, String> {
        unimplemented!("P1：ask_control::answer_ask 往返 + outcome 归约")
    }

    /// `session/update` → Pony 事件映射（§3.2.7）。
    ///
    /// - `agent_message_chunk` → `turn:delta`（text）
    /// - `agent_thought_chunk` → `turn:delta`（reasoning_content）
    /// - `tool_call` / `tool_call_update` → `turn:tool`（`TurnToolActivity`）
    /// - `plan` → `turn:tool` 子活动 / `TurnTraceStep`
    /// - `current_mode_update` / `available_commands_update` → 结构化日志（不进 UI）
    pub fn emit_update(&self, sink: &dyn TurnEventSink, update: &SessionUpdate) {
        unimplemented!("P1：8 变体 update → TurnEventSink 事件")
    }

    /// stopReason → Pony 归约（§3.2.8）。纯函数，无 I/O。
    pub fn reduce_stop_reason(stop: StopReason) -> AcpTurnOutcome {
        unimplemented!("P1：5 变体 stopReason → AcpTurnOutcome")
    }
}
