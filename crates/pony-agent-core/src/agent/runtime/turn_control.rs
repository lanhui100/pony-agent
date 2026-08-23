use super::*;

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
        let error = "stopped_by_user".to_string();
        let attachments = self
            .save_input_attachments_for_session(session_id, input_images)
            .unwrap_or_default();
        let persisted = self.persist_cancelled_turn_outcome(
            session_id,
            user_message,
            provider_meta,
            attachments,
            None,
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
