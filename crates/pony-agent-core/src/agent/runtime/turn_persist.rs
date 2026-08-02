// turn_persist: 轮次结果落库、附件保存、失败 trace 构建与 checkpoint 更新。
// 由 runtime/mod.rs 的 AgentRuntime 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl AgentRuntime {
    pub(crate) fn persist_turn_outcome(
        &self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_name: &str,
        provider_mode: &str,
        token_usage: Option<&TokenUsage>,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
        workspace_mode: Option<&str>,
    ) -> PersistedTurnOutcome {
        let updated_session = self
            .sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .append_turn(
                session_id,
                user_message,
                assistant_message,
                provider_native_transcript,
                attachments,
            );
        let retrieved = self.context_builder.retrieve_context_state(
            user_message,
            &[],
            workspace_mode,
            &updated_session,
            None,
            None,
        );
        let session_summary = self.context_builder.build_session_summary(
            self.graph.name(),
            &retrieved,
            provider_name,
            Some(provider_mode),
        );
        let (input_tokens, cache_hit_input_tokens, reasoning_tokens, output_tokens, total_tokens) =
            token_usage_parts(token_usage);

        PersistedTurnOutcome {
            session_summary,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn persist_turn_trace(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        phase: &str,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_meta: Option<&ProviderEventMeta>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        return_text: Option<String>,
        return_reasoning_content: Option<String>,
        fallback_reason: Option<String>,
        input_tokens: Option<u64>,
        cache_hit_input_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        session_summary: Option<String>,
        error: Option<String>,
    ) {
        self.persist_turn_trace_with_provider_calls(
            session_id,
            turn_id,
            user_message,
            phase,
            trace_steps,
            tool_activities,
            Vec::new(),
            provider_meta,
            provider_source,
            provider_mode,
            build_context_observation,
            return_text,
            return_reasoning_content,
            fallback_reason,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            session_summary,
            error,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn persist_turn_trace_with_provider_calls_and_hooks(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        phase: &str,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        model_hop_trace_contents: &[ModelHopTraceContent],
        provider_call_records: Vec<ProviderCallCacheRecord>,
        hook_trace_records: Vec<HookTraceRecord>,
        provider_meta: Option<&ProviderEventMeta>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        return_text: Option<String>,
        return_reasoning_content: Option<String>,
        fallback_reason: Option<String>,
        input_tokens: Option<u64>,
        cache_hit_input_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        session_summary: Option<String>,
        error: Option<String>,
    ) {
        let trace_timeline = build_persisted_trace_timeline(
            user_message,
            phase,
            provider_meta,
            provider_source.as_deref(),
            provider_mode.as_deref(),
            build_context_observation.as_ref(),
            &tool_activities,
            model_hop_trace_contents,
            return_text.as_deref(),
            return_reasoning_content.as_deref(),
            fallback_reason.as_deref(),
            error.as_deref(),
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
        );
        // 内存更新同步（快），SQLite 落库通过后台队列异步执行，
        // 避免 trace（可观测性）写盘阻塞主对话执行路径。
        let (session_key, mutation) = self
            .sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .record_turn_trace_in_memory(
                session_id,
                TurnTraceRecord {
                    turn_id: turn_id.to_string(),
                    session_id: session_id.map(str::to_string),
                    event_id: None,
                    event_type: None,
                    event_version: None,
                    sequence: None,
                    emitted_at_ms: None,
                    title: build_turn_trace_title(user_message),
                    phase: phase.to_string(),
                    trace_steps,
                    trace_timeline,
                    tool_activities,
                    provider_call_records,
                    hook_trace_records,
                    provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
                    provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
                    provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
                    provider_model: provider_meta.map(|meta| meta.model.clone()),
                    provider_source,
                    provider_mode,
                    build_context_observation,
                    session_summary,
                    fallback_reason,
                    error,
                    input_tokens,
                    cache_hit_input_tokens,
                    reasoning_tokens,
                    output_tokens,
                    total_tokens,
                    first_token_latency_ms,
                    turn_duration_ms,
                    updated_at: 0,
                },
            );
        self.enqueue_trace_persistence(session_key, mutation);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn persist_turn_trace_with_provider_calls(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        phase: &str,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_call_records: Vec<ProviderCallCacheRecord>,
        provider_meta: Option<&ProviderEventMeta>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        return_text: Option<String>,
        return_reasoning_content: Option<String>,
        fallback_reason: Option<String>,
        input_tokens: Option<u64>,
        cache_hit_input_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        session_summary: Option<String>,
        error: Option<String>,
    ) {
        self.persist_turn_trace_with_provider_calls_and_hooks(
            session_id,
            turn_id,
            user_message,
            phase,
            trace_steps,
            tool_activities,
            &[],
            provider_call_records,
            Vec::new(),
            provider_meta,
            provider_source,
            provider_mode,
            build_context_observation,
            return_text,
            return_reasoning_content,
            fallback_reason,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            session_summary,
            error,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fail_stream_turn_with_hook_dispatch(
        &self,
        sink: &impl TurnEventSink,
        control: &ExecutionControlRegistry,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        build_context_observation: Option<BuildContextObservation>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_call_records: Vec<ProviderCallCacheRecord>,
        hook_trace_records: Vec<HookTraceRecord>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        completed_hops: usize,
        error: String,
    ) {
        self.update_execution_checkpoint(
            control,
            turn_id,
            "failed",
            provider_meta,
            completed_hops,
            None,
            &trace_steps,
            &tool_activities,
            None,
            None,
            None,
            Some("failed"),
            Some(&error),
        );
        self.persist_turn_trace_with_provider_calls_and_hooks(
            session_id,
            turn_id,
            user_message,
            "failed",
            trace_steps.clone(),
            tool_activities.clone(),
            &[],
            provider_call_records.clone(),
            hook_trace_records.clone(),
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
            None,
            turn_duration_ms,
            None,
            Some(error.clone()),
        );
        emit_stream_failed(
            sink,
            turn_id.to_string(),
            provider_meta,
            trace_steps,
            Some(tool_activities),
            first_token_latency_ms,
            turn_duration_ms,
            build_context_observation,
            None,
            Some(provider_call_records),
            Some(hook_trace_records),
            error,
            session_id.map(str::to_string),
        );
    }

    pub(crate) fn save_input_attachments(
        &self,
        input: &TurnInput,
    ) -> Result<Vec<SessionAttachment>, String> {
        self.save_input_attachments_for_session(input.session_id.as_deref(), &input.images)
    }

    pub(crate) fn save_input_attachments_for_session(
        &self,
        session_id: Option<&str>,
        images: &[TurnInputImage],
    ) -> Result<Vec<SessionAttachment>, String> {
        let Some(session_id) = session_id else {
            return Ok(Vec::new());
        };
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .save_input_attachments(session_id, images)
    }

    pub(crate) fn persist_cancelled_turn_outcome(
        &self,
        session_id: Option<&str>,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        attachments: Vec<SessionAttachment>,
        workspace_mode: Option<&str>,
    ) -> PersistedTurnOutcome {
        self.persist_turn_outcome(
            session_id,
            user_message,
            CANCELLED_TURN_MESSAGE,
            provider_meta
                .map(|meta| meta.provider_name.as_str())
                .unwrap_or("runtime"),
            "cancelled",
            None,
            None,
            attachments,
            workspace_mode,
        )
    }

    pub(crate) fn fail_sync_turn_result(
        &self,
        provider_meta: Option<&ProviderEventMeta>,
        user_message: String,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        hook_trace_records: Vec<HookTraceRecord>,
        error: String,
    ) -> TurnResult {
        build_failed_turn_result_with_hooks(
            provider_meta,
            user_message,
            error,
            trace_steps,
            tool_activities,
            hook_trace_records,
        )
    }

    pub(crate) fn failed_trace_steps_for_tool_activities(
        &self,
        tool_activities: &[TurnToolActivity],
    ) -> Vec<TurnTraceStep> {
        if tool_activities.is_empty() {
            return self.telemetry_builder.failed_trace_before_tool();
        }
        let all_tools_ok = tool_activities
            .iter()
            .all(|activity| activity.status != "error");
        self.telemetry_builder.failed_trace_after_tool(all_tools_ok)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn persist_failed_sync_turn_trace_with_hooks(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_call_records: Vec<ProviderCallCacheRecord>,
        hook_trace_records: Vec<HookTraceRecord>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        fallback_reason: Option<String>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        error: String,
    ) {
        let error_message = error;
        let assistant_message = error_message.clone();
        let trace_timeline = build_persisted_trace_timeline(
            user_message,
            "failed",
            provider_meta,
            provider_source.as_deref(),
            provider_mode.as_deref(),
            build_context_observation.as_ref(),
            &tool_activities,
            &[],
            None,
            None,
            fallback_reason.as_deref(),
            Some(error_message.as_str()),
            None,
            None,
            None,
            None,
            None,
            first_token_latency_ms,
            turn_duration_ms,
        );
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .append_failed_turn(
                session_id,
                user_message,
                &assistant_message,
                TurnTraceRecord {
                    turn_id: turn_id.to_string(),
                    session_id: session_id.map(str::to_string),
                    event_id: None,
                    event_type: None,
                    event_version: None,
                    sequence: None,
                    emitted_at_ms: None,
                    title: build_turn_trace_title(user_message),
                    phase: "failed".to_string(),
                    trace_steps,
                    trace_timeline,
                    tool_activities,
                    provider_call_records,
                    hook_trace_records,
                    provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
                    provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
                    provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
                    provider_model: provider_meta.map(|meta| meta.model.clone()),
                    provider_source,
                    provider_mode,
                    build_context_observation,
                    session_summary: Some(error_message.clone()),
                    fallback_reason,
                    error: Some(error_message),
                    input_tokens: None,
                    cache_hit_input_tokens: None,
                    reasoning_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms,
                    turn_duration_ms,
                    updated_at: 0,
                },
            );
    }

    pub(crate) fn annotate_sync_terminal_trace_with_envelope(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        envelope: &TurnEventEnvelope,
    ) {
        let _ = self.annotate_turn_trace_terminal_event(
            session_id,
            turn_id,
            Some(envelope.event_id.clone()),
            Some(envelope.event_type.clone()),
            Some(envelope.event_version.clone()),
            Some(envelope.sequence),
            Some(envelope.emitted_at_ms),
        );
    }

    pub(crate) fn apply_terminal_envelope_to_turn_result(
        &self,
        result: &mut TurnResult,
        envelope: &TurnEventEnvelope,
    ) {
        result.event_id = Some(envelope.event_id.clone());
        result.event_type = Some(envelope.event_type.clone());
        result.event_version = Some(envelope.event_version.clone());
        result.sequence = Some(envelope.sequence);
        result.emitted_at_ms = Some(envelope.emitted_at_ms);
    }

    pub(crate) fn update_execution_checkpoint(
        &self,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        phase: &str,
        provider_meta: Option<&ProviderEventMeta>,
        completed_hops: usize,
        active_tool_name: Option<&str>,
        trace_steps: &[TurnTraceStep],
        tool_activities: &[TurnToolActivity],
        provider_source: Option<&str>,
        provider_mode: Option<&str>,
        fallback_reason: Option<&str>,
        status: Option<&str>,
        error: Option<&str>,
    ) {
        control.update(turn_id, |checkpoint| {
            checkpoint.phase = phase.to_string();
            checkpoint.completed_hops = completed_hops;
            checkpoint.max_hops = max_tool_hops_per_turn();
            checkpoint.active_tool_name = active_tool_name.map(str::to_string);
            checkpoint.trace_steps = trace_steps.to_vec();
            checkpoint.tool_activities = tool_activities.to_vec();
            checkpoint.provider_requested_name =
                provider_meta.map(|meta| meta.requested_name.clone());
            checkpoint.provider_name = provider_meta.map(|meta| meta.provider_name.clone());
            checkpoint.provider_protocol = provider_meta.map(|meta| meta.protocol.clone());
            checkpoint.provider_model = provider_meta.map(|meta| meta.model.clone());
            checkpoint.provider_source = provider_source.map(str::to_string);
            checkpoint.provider_mode = provider_mode.map(str::to_string);
            checkpoint.fallback_reason = fallback_reason.map(str::to_string);
            checkpoint.error = error.map(str::to_string);
            if let Some(status) = status {
                checkpoint.status = status.to_string();
            }
        });
    }
}
