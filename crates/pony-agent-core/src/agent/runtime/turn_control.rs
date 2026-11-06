use super::*;

/// wave-64 R2：流式轮次的**错误归类**（取消 vs 失败）——唯一判据，纯函数、无副作用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StreamErrorClass {
    /// 用户已请求停止 **且** provider 读通道以取消码结束 ⇒ 终态必须是 `turn:cancelled`。
    Cancelled,
    /// 其余一切错误 ⇒ 保持既有 `turn:failed` 语义（逐字不变）。
    Failed,
}

/// wave-64 R2 归类判据（契约 §2 R2）：
///
/// - `cancel_requested == true`（该轮已有停止请求，唯一取消源 = checkpoint 停止位）
///   **且** `error` 以 `ProviderStreamGuard::CANCELLED_ERROR_CODE`
///   （wire 码 `provider_stream_cancelled`）开头 ⇒ [`StreamErrorClass::Cancelled`]；
/// - 其余情况（含"有停止请求但错误不是取消码"、"取消码但无停止请求"）⇒
///   [`StreamErrorClass::Failed`]，即真实失败路径的行为完全不变 —— 绝不把真实错误吞成取消。
pub(crate) fn classify_stream_error(cancel_requested: bool, error: &str) -> StreamErrorClass {
    if cancel_requested
        && error.starts_with(crate::agent::provider::ProviderStreamGuard::CANCELLED_ERROR_CODE)
    {
        StreamErrorClass::Cancelled
    } else {
        StreamErrorClass::Failed
    }
}

/// W1b：控制结果已挂起、但找不到可恢复的待处理请求（`call_id` / 审批记录丢失）时的错误码。
///
/// 语义：这不是“等待用户”的可恢复暂停，而是一次必须让用户看见的失败终态。
pub(super) const CONTROL_OUTCOME_PENDING_WITHOUT_REQUEST_CODE: &str =
    "control_outcome_pending_without_request";

/// W1b：构造“挂起但无待处理请求”的失败终态文案。
///
/// 形态与既有 provider 错误码一致（`<code>: <message>`），既保住机器可判的错误码，
/// 又给用户一条能读懂的中文说明（而不是一条永远不会恢复的 `turn:suspended`）。
pub(super) fn build_control_outcome_pending_without_request_error() -> String {
    format!(
        "{CONTROL_OUTCOME_PENDING_WITHOUT_REQUEST_CODE}: 工具调用已挂起但未找到可恢复的待处理请求（call_id/审批丢失），本轮已按失败收口；请重试或重新发起该工具调用。"
    )
}

/// W1b 补充：`Approval` 类挂起当前**没有**“批准后自动重放工具”的实现。
///
/// 若仍下发 `turn:suspended`，TUI 会弹出审批框；用户批准后控制面只消费请求，并不会恢复
/// 原工具调用，最终仍是“批准了但没后续”。因此这里显式失败收口，给出可读错误码，避免
/// 把“未实现”伪装成“等待用户”。
pub(super) const APPROVAL_RESUME_UNAVAILABLE_CODE: &str = "approval_resume_unavailable";

pub(super) fn build_approval_resume_unavailable_error() -> String {
    format!(
        "{APPROVAL_RESUME_UNAVAILABLE_CODE}: 该工具需要审批，但当前 TUI/运行时尚不支持“批准后自动重放该工具调用”；本轮已按失败收口。请调整权限/工具参数后重试。"
    )
}

/// C3：Ask 挂起**没有可恢复的 run 身份**时的错误码（ask-resume 工单，与 W1b 同族）。
///
/// 语义：`Interaction`（Ask）本应经宿主应答 + `graph_resume_ask` 恢复，但恢复的前提是挂起
/// 请求持久化了真实 `run_id`（`bind_ask_wait` 需要它才能把 Ask 绑到 graph ask-wait）。判据
/// 只有「`Interaction` 且 `run_id == None`」这一种形态 —— **不得**扩大成「无绑定即失败」，
/// 否则既有 `AgentRuntime` 单测里「治理路径 + 无 store」的合法 suspended 语义会被误杀。
pub const ASK_RESUME_UNAVAILABLE_CODE: &str = "ask_resume_unavailable";

/// C3：构造「Ask 挂起但没有 run 身份」的失败终态文案（首行 `ask_resume_unavailable: <中文说明>`）。
pub(super) fn build_ask_resume_unavailable_error() -> String {
    format!(
        "{ASK_RESUME_UNAVAILABLE_CODE}: 本轮向用户提问（Ask）时没有可恢复的 graph run 身份，作答后无法自动继续本轮；本轮已按失败收口。请重新发起该请求。"
    )
}

/// F3：从「不可恢复挂起」的失败终态文案里取出**机器可判的错误码**（首段 = 冒号前）。
///
/// 存在的理由：流式/非流式两处的结构化日志必须打印**与终态派发完全一致**的 code。此前日志硬编码
/// `control_outcome_pending_without_request`，而实际派发可能是 `approval_resume_unavailable` 或
/// `ask_resume_unavailable` ⇒ 排障时「日志码 → 上报码」对不上。本函数只截首段，不改终态文案本身。
pub(super) fn suspension_error_code(message: &str) -> &str {
    message.split(':').next().unwrap_or(message).trim()
}

/// W1b：挂起结果是否**没有可恢复通道**。
///
/// - `None`：治理路径已产出 `control_outcome_pending`，但匹配不到待处理请求（call_id 丢失/被消费）；
/// - `Some(Approval)`：**wave-65 C1** 起按运行时能力位判定 —— 宿主具备「审批挂起可恢复」
///   （`host_approval_resume == true`）**且**该请求带会话身份（`session_id.is_some()`）时为
///   **可恢复**（宿主可列出审批 → 批准/拒绝 → 续轮）；否则保持既有不可恢复语义
///   （见 [`APPROVAL_RESUME_UNAVAILABLE_CODE`]）；
/// - `Some(Interaction)`：Ask 可经宿主应答恢复，保持 suspended —— **除非**该请求没有 run 身份
///   （见 [`ASK_RESUME_UNAVAILABLE_CODE`]）：没有 run 就没有 ask-wait 可绑定，作答后必然死等。
pub(super) fn suspension_is_unrecoverable(
    suspension: &ToolTurnSuspension,
    host_approval_resume: bool,
) -> bool {
    match &suspension.pending_request {
        None => true,
        Some(request) => match request.request_kind {
            PendingControlRequestKind::Approval => {
                !(host_approval_resume && request.session_id.is_some())
            }
            PendingControlRequestKind::Interaction => request.run_id.is_none(),
        },
    }
}

/// C3：按挂起请求的 kind/run 身份选择**可读失败终态**的错误文案（唯一判据，流式与非流式共用）。
///
/// - `Approval` ⇒ [`build_approval_resume_unavailable_error`]（既有语义不动）；
/// - `Interaction` 且无 run 身份 ⇒ [`build_ask_resume_unavailable_error`]（本次新增收口点）；
/// - 其余（含 `pending_request == None`）⇒ 既有 `control_outcome_pending_without_request`（逐字不变）。
pub(super) fn build_unrecoverable_suspension_error(
    suspension: &ToolTurnSuspension,
) -> String {
    match suspension.pending_request.as_ref() {
        Some(request) => match request.request_kind {
            PendingControlRequestKind::Approval => build_approval_resume_unavailable_error(),
            PendingControlRequestKind::Interaction if request.run_id.is_none() => {
                build_ask_resume_unavailable_error()
            }
            _ => build_control_outcome_pending_without_request_error(),
        },
        None => build_control_outcome_pending_without_request_error(),
    }
}

/// wave-64 R1（BLOCKING，terminal-once 闸门）：尝试占有该轮的**终态派发权**。
///
/// 唯一真值源是 checkpoint `status`（与 C5 宽限监督者共用同一登记表，绝不新建第二套状态）：
/// - `running` ⇒ 在 registry 锁内**原子 CAS** 到 `target`（`"cancelled"` / `"failed"`），
///   抢到者返回 `true`（继续 persist_trace + 派发终态）；
/// - 已是任一终态（`completed` / `failed` / `cancelled`）⇒ 返回 `false`
///   （已有终态派发者，调用方必须静默退出：不再持久化终态、不再派发事件）；
/// - 无 checkpoint（未注册轮次/纯内存子路径）或非终态的非 `running`（如 `suspended`）
///   ⇒ 无并发终态可竞，返回 `true`（保持既有行为不变）。
///
/// 与 [`crate::agent::stop_escalation`] 的 `escalate_now` 互斥：后者同样以
/// "`status == running` 才置 cancelled" 的锁内判定收口，两者抢的是同一状态位，
/// 故宽限期收口与 in-band 收口至多一方派发终态。
pub(crate) fn claim_terminal_status(
    control: &ExecutionControlRegistry,
    turn_id: &str,
    target: &str,
) -> bool {
    let Some(checkpoint) = control.load_checkpoint(Some(turn_id), None) else {
        return true;
    };
    let status = checkpoint.status.trim().to_lowercase();
    if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
        return false;
    }
    if status != "running" {
        return true;
    }
    let mut claimed = false;
    control.update(turn_id, |checkpoint| {
        if checkpoint.status.trim().eq_ignore_ascii_case("running") {
            checkpoint.status = target.to_string();
            checkpoint.phase = target.to_string();
            claimed = true;
        }
    });
    claimed
}

/// 终态已被抢占时的**结构化**留痕（debug 级；不得用裸 eprintln，沿用 nfr logging 约束）。
pub(crate) fn log_terminal_dispatch_suppressed(turn_id: &str, target: &str, source: &str) {
    crate::agent::otel::log::log_json_with_fields(
        "debug",
        "terminal dispatch suppressed: turn already has a terminal status (terminal-once gate)",
        None,
        &[
            ("turn_id", serde_json::json!(turn_id)),
            ("target_status", serde_json::json!(target)),
            ("source", serde_json::json!(source)),
        ],
    );
}

/// 用户已请求停止但仍进入 fallback/失败路径时的结构化留痕（含原始 error，便于对拍）。
pub(crate) fn log_stop_short_circuit(turn_id: &str, error: &str, scope: &str) {
    crate::agent::otel::log::log_json_with_fields(
        "warn",
        "turn stop requested while streaming decision failed; classified cancelled instead of fallback/failed",
        None,
        &[
            ("turn_id", serde_json::json!(turn_id)),
            ("scope", serde_json::json!(scope)),
            ("error", serde_json::json!(error)),
        ],
    );
}

impl AgentRuntime {
    pub(super) fn should_cancel_turn(&self, control: &ExecutionControlRegistry, turn_id: &str) -> bool {
        control.is_stop_requested(turn_id)
    }


    #[allow(clippy::too_many_arguments)]
    pub(super) fn cancel_stream_turn<S: TurnEventSink>(
        &self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        session_id: Option<&str>,
        user_message: &str,
        input_images: &[TurnInputImage],
        provider_meta: Option<&ProviderEventMeta>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        build_context_observation: Option<BuildContextObservation>,
    ) {
        // wave-64 R1（BLOCKING，terminal-once）：C5 宽限监督可能已先行收口并派发了一条
        // `turn:cancelled`；此时 in-band 循环再命中停止位必须**静默退出**，禁止第二条终态、
        // 禁止用 in-band 结果覆盖已落盘的终态 checkpoint/trace。
        if !claim_terminal_status(control, turn_id, "cancelled") {
            log_terminal_dispatch_suppressed(turn_id, "cancelled", "cancel_stream_turn");
            return;
        }
        let error = "stopped_by_user".to_string();
        let attachments = self
            .save_input_attachments_for_session(session_id, input_images)
            .unwrap_or_default();
        // wave-tui-session-resilience C5：把本轮已产生的工具活动投影为检查点步骤。
        let checkpoint_tools: Vec<String> = tool_activities
            .iter()
            .map(|activity| {
                if activity.description.trim().is_empty() {
                    activity.name.clone()
                } else {
                    format!("{} {}", activity.name, activity.description.trim())
                }
            })
            .collect();
        let persisted = self.persist_cancelled_turn_outcome(
            session_id,
            user_message,
            provider_meta,
            attachments,
            None,
            "",
            checkpoint_tools,
        );
        self.update_execution_checkpoint(
            control,
            turn_id,
            "cancelled",
            provider_meta,
            0,
            None,
            &trace_steps,
            &tool_activities,
            None,
            None,
            None,
            Some("cancelled"),
            Some(&error),
        );
        self.persist_turn_trace(
            session_id,
            turn_id,
            user_message,
            "cancelled",
            trace_steps.clone(),
            tool_activities.clone(),
            provider_meta,
            None,
            None,
            build_context_observation.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            first_token_latency_ms,
            turn_duration_ms,
            Some(persisted.session_summary),
            Some(error.clone()),
        );
        emit_stream_cancelled(
            sink,
            turn_id.to_string(),
            provider_meta,
            trace_steps,
            Some(tool_activities),
            first_token_latency_ms,
            turn_duration_ms,
            build_context_observation.clone(),
            Some(build_persisted_trace_timeline(
                user_message,
                "cancelled",
                provider_meta,
                None,
                None,
                build_context_observation.as_ref(),
                &[],
                &[],
                None,
                None,
                None,
                Some(error.as_str()),
                None,
                None,
                None,
                None,
                None,
                first_token_latency_ms,
                turn_duration_ms,
            )),
            None,
            error,
            // PA-095 #3：cancelled 终态事件必须携带 session_id——否则进入空会话
            // 缓冲，flush 被 skip，事件流丢失 turn:cancelled 终态（对拍/重建缺终态）。
            session_id.map(|id| id.to_string()),
            // wave-60 C3：终态携带该轮 checkpoint 记录的停止原因（无记录 ⇒ 下游落 legacy）。
            control
                .load_checkpoint(Some(turn_id), None)
                .and_then(|checkpoint| checkpoint.stop_reason),
        );
    }


    /// Suspend a turn on a persisted control outcome (design.md Decision 5, P1-1 wiring).
    /// Matches the dispatcher's pending request to the originating tool call, binds an Ask wait
    /// to the shared graph run store when a run + store are available (run → `WaitingUser`), and
    /// returns the suspension snapshot. The turn never proceeds to `provider_followup`.
    pub(super) fn suspend_turn_for_control_outcome(
        &self,
        tool_call: &ToolCall,
        _tool_result: &ToolResult,
    ) -> ToolTurnSuspension {
        let Some(dispatcher) = self.governed_dispatcher() else {
            return ToolTurnSuspension {
                pending_request: None,
                bound_run_id: None,
            };
        };
        let requests = dispatcher.pending_requests();
        let Some(pending_request) = match_pending_control_request(&requests, tool_call) else {
            return ToolTurnSuspension {
                pending_request: None,
                bound_run_id: None,
            };
        };
        let pending_request = pending_request.clone();

        let bound_run_id = self.bind_ask_wait_for_request(&pending_request, tool_call);
        ToolTurnSuspension {
            pending_request: Some(pending_request),
            bound_run_id,
        }
    }

    /// W1b：`control_outcome_pending` 已产出，但**治理路径上拿不到可恢复的待处理请求**。
    ///
    /// 判据：本 runtime 挂着治理 dispatcher（执行器就是治理适配器，`control_outcome_pending`
    /// 是它写出的真实挂起标记），却匹配不到任何 `Pending` 的 Approval / Interaction ——
    /// 即 `call_id` 丢失或审批记录已被消费。此时下发裸 `turn:suspended` 会让宿主永远等不到
    /// 恢复信号，必须以可读失败终态收口（`control_outcome_pending_without_request`）。
    ///
    /// 非治理执行器（无 dispatcher，如自定义 `ToolExecutor` 桩）保持既有 suspended 语义：
    /// 这类执行器本来就没有“待处理请求”通道，`pending_request=None` 是唯一可能形态。
    pub(super) fn suspension_without_recoverable_request(
        &self,
        suspension: &ToolTurnSuspension,
    ) -> bool {
        self.governed_dispatcher().is_some()
            && suspension_is_unrecoverable(suspension, self.host_approval_resume)
    }


    /// Bind one persisted Ask `PendingControlRequest` to the shared graph run store
    /// (`GraphRunner::bind_ask_wait`), aligning the pending request's `call_id` with the original
    /// assistant tool-call id. Returns the bound `run_id` or `None` when no store/run is
    /// available or the run is already bound.
    fn bind_ask_wait_for_request(
        &self,
        pending_request: &PendingControlRequest,
        tool_call: &ToolCall,
    ) -> Option<String> {
        // Only `Interaction` (Ask) requests bind to the graph ask-wait suspension; an Approval
        // suspends the turn too, but its resume path re-executes the tool rather than injecting an
        // answer as the Ask terminal result (design.md Decision 5 — Ask is never approval).
        if pending_request.request_kind != PendingControlRequestKind::Interaction {
            return None;
        }
        let store_arc = self.graph_run_store()?;
        let run_id = pending_request.run_id.clone()?;
        let expected_version = pending_request.version;
        let request_id = pending_request.request_id.clone();
        let call_id = pending_request.call_id.clone();
        let turn_id = pending_request.turn_id.clone();
        let session_id = pending_request.session_id.clone();
        let tool_name = tool_call.name.clone();
        let assistant_transcript = json!({
            "toolCalls": [{ "id": call_id, "name": tool_name }],
            "assistantMessage": tool_call.arguments.clone(),
        });
        let binding = GraphAskWaitBinding {
            request_id: request_id.clone(),
            expected_version,
            run_id: run_id.clone(),
            turn_id,
            session_id,
            call_id,
            tool_name,
            assistant_transcript,
            created_at_ms: runtime_now_ms(),
        };
        let mut store = store_arc.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run store poisoned: {e}, recovering");
            e.into_inner()
        });
        match GraphRunner::new()
            .bind_ask_wait(&mut store, &run_id, binding)
        {
            Ok(_) => Some(run_id),
            Err(error) => {
                runtime_log(format!(
                    "turn:ask-bind-failed run={run_id} request={request_id} error={error}"
                ));
                // A failed bind must not silently drop the pending request: the dispatcher still
                // holds it for the host surface; the suspended turn outcome still carries it.
                None
            }
        }
    }


    /// Build the `TurnResult` returned when a turn pauses on a control outcome. `phase` is
    /// `SUSPENDED_TURN_PHASE` ("suspended") so the host control plane can react to the Ask wait
    /// without mistaking the turn for a failure or a completion.
    pub(super) fn build_suspended_turn_result(
        &self,
        provider_meta: Option<&ProviderEventMeta>,
        user_message: String,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        hook_trace_records: Vec<HookTraceRecord>,
        suspension: &ToolTurnSuspension,
    ) -> TurnResult {
        let assistant_message = suspension
            .pending_request
            .as_ref()
            .map(|request| {
                format!(
                    "Ask `{}` awaits a host answer before the run resumes.",
                    request.request_id
                )
            })
            .unwrap_or_else(|| {
                "工具调用进入控制请求状态，等待宿主应答后继续。".to_string()
            });
        let session_summary = suspension
            .pending_request
            .as_ref()
            .map(|request| request.request_id.clone())
            .unwrap_or_else(|| SUSPENDED_TURN_PHASE.to_string());
        TurnResult {
            event_id: None,
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: SUSPENDED_TURN_PHASE.to_string(),
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()).unwrap_or_default(),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()).unwrap_or_default(),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()).unwrap_or_default(),
            provider_model: provider_meta.map(|meta| meta.model.clone()).unwrap_or_default(),
            provider_source: SUSPENDED_TURN_PHASE.to_string(),
            provider_mode: SUSPENDED_TURN_PHASE.to_string(),
            fallback_reason: Some("control_outcome_pending".to_string()),
            build_context_observation: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            user_message,
            assistant_message,
            trace_steps,
            trace_timeline: Vec::new(),
            tool_activities,
            provider_call_records: Vec::new(),
            hook_trace_records,
            session_summary,
        }
    }

}

#[cfg(test)]
mod wave64_classify_tests {
    use super::*;

    /// wave-64 R2：归类判据真值表（实现级单测；验收层另有行为级黑盒用例）。
    #[test]
    fn r2_classify_stream_error_truth_table() {
        let cancel_code = format!(
            "{}: SSE 流已按请求中止 (stop_requested); elapsed=3ms; parsed_bytes=0; endpoint=http://127.0.0.1/v1",
            crate::agent::provider::ProviderStreamGuard::CANCELLED_ERROR_CODE
        );

        // ① 取消码 + 停止位 ⇒ 取消（禁止沦为 failed）。
        assert_eq!(
            classify_stream_error(true, &cancel_code),
            StreamErrorClass::Cancelled,
            "R2：provider 取消码 + 用户停止位必须归类为取消"
        );
        // ② 取消码但无停止位 ⇒ 仍失败（不得把真实错误吞成取消）。
        assert_eq!(
            classify_stream_error(false, &cancel_code),
            StreamErrorClass::Failed,
            "R2：无停止位的取消码不得被归类为取消"
        );
        // ③ 有停止位但错误是真实失败 ⇒ 失败（真实失败行为逐字不变）。
        assert_eq!(
            classify_stream_error(true, "error sending request: connection reset by peer"),
            StreamErrorClass::Failed,
            "R2：非取消码的真实错误不得被归类为取消"
        );
        // ④ 其他 provider 错误码（超时）⇒ 失败，不得因前缀相近而误判。
        assert_eq!(
            classify_stream_error(
                true,
                &format!(
                    "{}: provider 流总时长超限",
                    crate::agent::provider::ProviderStreamGuard::DEADLINE_ERROR_CODE
                )
            ),
            StreamErrorClass::Failed,
            "R2：超时错误码不得被误判为取消"
        );
    }
}


#[cfg(test)]
mod w1_approval_unrecoverable_tests {
    use super::*;
    use crate::agent::tool_runtime::{
        PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
    };

    fn request(kind: PendingControlRequestKind) -> PendingControlRequest {
        request_with_session(kind, Some("sess-w1"))
    }

    fn request_with_session(
        kind: PendingControlRequestKind,
        session_id: Option<&str>,
    ) -> PendingControlRequest {
        PendingControlRequest {
            request_id: "req-w1".to_string(),
            request_kind: kind,
            session_id: session_id.map(str::to_string),
            run_id: Some("run-w1".to_string()),
            turn_id: "turn-w1".to_string(),
            call_id: "call-w1".to_string(),
            descriptor_snapshot_id: "snap".to_string(),
            descriptor_id: "desc".to_string(),
            final_args_digest: "digest".to_string(),
            policy_digest: "policy".to_string(),
            nonce: "nonce".to_string(),
            version: 1,
            expires_at_ms: u64::MAX,
            state: PendingControlRequestState::Pending,
            prompt: Some("approve?".to_string()),
            options: None,
        }
    }

    #[test]
    fn suspension_without_request_is_unrecoverable() {
        // 无待处理请求 ⇒ 无论能力位开关都是不可恢复（既有语义逐字不变）。
        for capability in [false, true] {
            assert!(suspension_is_unrecoverable(
                &ToolTurnSuspension {
                    pending_request: None,
                    bound_run_id: None,
                },
                capability
            ));
        }
    }

    /// wave-65 C1 语义迁移：审批挂起在**能力位关闭**（桌面/CLI 默认）时仍不可恢复。
    #[test]
    fn approval_suspension_unrecoverable_when_capability_disabled() {
        assert!(suspension_is_unrecoverable(
            &ToolTurnSuspension {
                pending_request: Some(request(PendingControlRequestKind::Approval)),
                bound_run_id: None,
            },
            false
        ));
    }

    /// wave-65 C1：能力位开启 **且** 请求带会话身份 ⇒ 可恢复（宿主可列出/批准/拒绝后由
    /// 续轮重放该工具调用）。
    #[test]
    fn approval_suspension_recoverable_when_capability_enabled_with_session_identity() {
        assert!(!suspension_is_unrecoverable(
            &ToolTurnSuspension {
                pending_request: Some(request(PendingControlRequestKind::Approval)),
                bound_run_id: None,
            },
            true
        ));
    }

    /// wave-65 C1：能力位开启但**无会话身份** ⇒ 无法作用域化列出/CAS，仍判不可恢复（fail-closed）。
    #[test]
    fn approval_suspension_unrecoverable_when_capability_enabled_without_session_identity() {
        assert!(suspension_is_unrecoverable(
            &ToolTurnSuspension {
                pending_request: Some(request_with_session(
                    PendingControlRequestKind::Approval,
                    None
                )),
                bound_run_id: None,
            },
            true
        ));
    }

    #[test]
    fn interaction_suspension_stays_recoverable() {
        // Ask 分支不受能力位影响（逐字不变）。
        for capability in [false, true] {
            assert!(!suspension_is_unrecoverable(
                &ToolTurnSuspension {
                    pending_request: Some(request(PendingControlRequestKind::Interaction)),
                    bound_run_id: None,
                },
                capability
            ));
        }
    }
}

#[cfg(test)]
mod w1_unrecoverable_code_tests {
    use super::*;
    use crate::agent::tool_runtime::{
        PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
    };

    fn request(kind: PendingControlRequestKind, run_id: Option<&str>) -> PendingControlRequest {
        PendingControlRequest {
            request_id: "req-w1".to_string(),
            request_kind: kind,
            session_id: Some("sess-w1".to_string()),
            run_id: run_id.map(str::to_string),
            turn_id: "turn-w1".to_string(),
            call_id: "call-w1".to_string(),
            descriptor_snapshot_id: "snap".to_string(),
            descriptor_id: "desc".to_string(),
            final_args_digest: "digest".to_string(),
            policy_digest: "policy".to_string(),
            nonce: "nonce".to_string(),
            version: 1,
            expires_at_ms: u64::MAX,
            state: PendingControlRequestState::Pending,
            prompt: Some("approve?".to_string()),
            options: None,
        }
    }

    /// F3：日志 code 必须等于终态文案的首段 —— 三族各断言一次（真实字符串，不是自证）。
    #[test]
    fn log_code_equals_terminal_code_for_every_unrecoverable_family() {
        let cases = [
            (
                ToolTurnSuspension {
                    pending_request: Some(request(PendingControlRequestKind::Approval, Some("run-w1"))),
                    bound_run_id: None,
                },
                APPROVAL_RESUME_UNAVAILABLE_CODE,
            ),
            (
                ToolTurnSuspension {
                    pending_request: Some(request(PendingControlRequestKind::Interaction, None)),
                    bound_run_id: None,
                },
                ASK_RESUME_UNAVAILABLE_CODE,
            ),
            (
                ToolTurnSuspension {
                    pending_request: None,
                    bound_run_id: None,
                },
                CONTROL_OUTCOME_PENDING_WITHOUT_REQUEST_CODE,
            ),
        ];
        for (suspension, expected) in cases {
            // wave-65 C1：能力位关闭（桌面/CLI 默认）时三族判据逐字不变。
            assert!(
                suspension_is_unrecoverable(&suspension, false),
                "该形态必须判为不可恢复"
            );
            let message = build_unrecoverable_suspension_error(&suspension);
            assert_eq!(
                suspension_error_code(&message),
                expected,
                "日志 code 必须与终态派发的错误码逐字一致，实际文案 = {message}"
            );
            assert!(
                message.starts_with(&format!("{expected}: ")),
                "终态文案必须保持 `<code>: <中文说明>` 形态，实际 = {message}"
            );
        }
    }

    /// F4/C3 回归：`Interaction` **有** run 身份时仍可恢复（不得被误判为失败）。
    #[test]
    fn interaction_with_run_identity_is_still_recoverable() {
        assert!(!suspension_is_unrecoverable(
            &ToolTurnSuspension {
                pending_request: Some(request(PendingControlRequestKind::Interaction, Some("run-w1"))),
                bound_run_id: Some("run-w1".to_string()),
            },
            false
        ));
    }
}
