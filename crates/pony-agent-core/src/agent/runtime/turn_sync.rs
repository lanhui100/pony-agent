use super::*;

impl AgentRuntime {
    fn handle_sync_tool_turn(
        &self,
        user_message: String,
        display_message: String,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_call: ToolCall,
        initial_model_hook_trace_records: Vec<HookTraceRecord>,
        first_token_latency_ms: Option<u64>,
        _turn_started_at: &Instant,
        provider_call_records: &mut Vec<ProviderCallCacheRecord>,
    ) -> Result<SyncToolTurnOutcome, TurnResult> {
        let context_observation = build_context_observation(planning_request, tools);
        let mut hop_records = Vec::new();
        let mut tool_activities = Vec::new();
        let mut current_tool_call = tool_call;
        let mut current_assistant_message = first_decision.assistant_message.clone();
        let mut current_assistant_output_text = first_decision.output_text.clone();
        let mut current_assistant_reasoning = first_decision.reasoning_content.clone();
        let mut current_assistant_reasoning_value = first_decision.reasoning_content_value.clone();
        let mut all_tools_ok = true;
        let mut completed_hops = 0usize;
        let mut accumulated_fallback_reason = first_decision.fallback_reason.clone();
        let mut accumulated_token_usage = first_decision.token_usage.clone();
        let mut hook_trace_records = initial_model_hook_trace_records;
        let mut seen_tool_signatures = BTreeSet::from([tool_call_signature(&current_tool_call)]);
        let mut consecutive_failures = ConsecutiveFailureTracker::new();
        let mut accumulated_messages: Vec<Value> = Vec::new();

        loop {
            completed_hops += 1;
            runtime_log(format!(
                "turn:tool-execute hop={} name={} args={}",
                completed_hops, current_tool_call.name, current_tool_call.arguments
            ));
            let tool_start_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallStart);
            hook_trace_records.extend(tool_start_hook_outcome.trace_records.clone());
            if let Some(error) = tool_start_hook_outcome.fail_turn_error {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    self.telemetry_builder
                        .tool_activities_running(&current_tool_call),
                    hook_trace_records,
                    error,
                ));
            }

            let (tool_result, invocation_record, capability_hook_trace_records) =
                self.execute_registered_tool_call(&current_tool_call);
            hook_trace_records.extend(capability_hook_trace_records);
            runtime_log(format!(
                "turn:tool-result hop={} name={} status={} output_preview={}",
                completed_hops,
                tool_result.tool_name,
                tool_result.status,
                preview_text(&tool_result.output, 160)
            ));
            all_tools_ok &= tool_result.status == "ok";
            tool_activities.extend(annotate_capability_tool_activities(
                self.telemetry_builder
                    .tool_activities_after_result(&current_tool_call, &tool_result),
                invocation_record,
            ));
            hop_records.push(ToolTurnHopRecord {
                assistant_message: current_assistant_message.clone(),
                assistant_output_text: current_assistant_output_text.clone(),
                assistant_reasoning_content: current_assistant_reasoning.clone(),
                assistant_reasoning_content_value: current_assistant_reasoning_value.clone(),
                tool_call: current_tool_call.clone(),
                tool_result: tool_result.clone(),
            });

            // Design Decision 5 (PA-076 P1-1): a pending control outcome (`control_outcome_pending`,
            // surfaced as a structured `error.code`) means the dispatcher persisted an
            // `Interaction`/`Approval` `PendingControlRequest` instead of producing a
            // provider-consumable result. The run must pause — bind the Ask wait to the graph run
            // when a store + run are available, and return a suspended outcome — never feed the
            // pending marker back to the provider as an ordinary tool error.
            if tool_result_control_outcome_pending(&tool_result) {
                let suspension = self.suspend_turn_for_control_outcome(
                    &current_tool_call,
                    &tool_result,
                );
                if let Some(request) = &suspension.pending_request {
                    runtime_log(format!(
                        "turn:ask-suspend hop={} request={} kind={:?} call_id={} bound_run={:?}",
                        completed_hops,
                        request.request_id,
                        request.request_kind,
                        request.call_id,
                        suspension.bound_run_id
                    ));
                }
                return Err(self.build_suspended_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities,
                    hook_trace_records,
                    &suspension,
                ));
            }

            // Only abort the turn if the tool was intentionally stopped (e.g. user cancellation).
            // Tool execution errors (status "error") are fed back to the model so it can
            // decide how to respond — e.g. try a different path, use another tool, or
            // explain the problem to the user.
            if tool_result.status == "aborted" {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(false),
                    tool_activities,
                    hook_trace_records,
                    build_tool_execution_error(
                        &current_tool_call.name,
                        tool_result.output.as_str(),
                    ),
                ));
            }

            // Consecutive-failure stop-loss: the same tool failing with the same error code
            // `limit` times in a row means the model is retrying a doomed invocation. Stop the
            // follow-up loop and surface the real error instead of burning the remaining
            // follow-up budget on identical retries.
            let failure_signal = tool_failure_signal(&current_tool_call, &tool_result);
            let failure_count = consecutive_failures.record(failure_signal.clone());
            if failure_count >= max_consecutive_tool_failures_per_turn() {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(false),
                    tool_activities,
                    hook_trace_records,
                    build_consecutive_tool_failure_error(
                        max_consecutive_tool_failures_per_turn(),
                        &current_tool_call.name,
                        failure_signal
                            .as_ref()
                            .map(|(_, code)| code.as_str())
                            .unwrap_or("unknown"),
                        tool_result.output.as_str(),
                    ),
                ));
            }

            let tool_end_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallEnd);
            hook_trace_records.extend(tool_end_hook_outcome.trace_records.clone());
            if let Some(error) = tool_end_hook_outcome.fail_turn_error {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities,
                    hook_trace_records,
                    error,
                ));
            }

            let provider_call_started_at = Instant::now();
            let response = match provider_followup(
                provider,
                planning_request,
                tools,
                &mut accumulated_messages,
                current_assistant_message.as_ref(),
                &current_tool_call,
                &tool_result,
            ) {
                Ok(response) => response,
                Err(error) => {
                    return Err(build_failed_turn_result(
                        Some(provider_meta),
                        display_message,
                        error,
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities,
                    ));
                }
            };
            let mut response = response;
            let provider_call_duration_ms = provider_call_started_at.elapsed().as_millis() as u64;
            provider_call_records.push(build_provider_call_cache_record(
                ProviderRequestKind::ToolFollowup,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                response.token_usage.as_ref(),
                None,
                Some(provider_call_duration_ms),
                ProviderLatencyKind::BufferedResponse,
                Some(&context_observation),
            ));
            accumulated_token_usage =
                merge_token_usage(accumulated_token_usage, response.token_usage.as_ref());
            accumulated_fallback_reason = merge_fallback_reason(
                accumulated_fallback_reason,
                response.fallback_reason.clone(),
            );

            if let Some(error) = provider_failure_message(
                &response.provider_mode,
                response.fallback_reason.as_deref(),
            ) {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities,
                    hook_trace_records,
                    error,
                ));
            }

            if let Some(next_tool_call) = response.tool_call.take() {
                let normalized = match normalize_tool_directive(
                    next_tool_call,
                    response.assistant_message.take(),
                    &response.output_text,
                    response.reasoning_content.as_deref(),
                    response.reasoning_content_value.as_ref(),
                ) {
                    Ok(normalized) => normalized,
                    Err(error) => {
                        return Err(self.fail_sync_turn_result(
                            Some(provider_meta),
                            display_message,
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                            tool_activities,
                            hook_trace_records,
                            error,
                        ));
                    }
                };
                response.tool_call = Some(normalized.tool_call);
                response.assistant_message = normalized.assistant_message;
            }

            if let Some(next_tool_call) = response.tool_call.clone() {
                if completed_hops >= max_tool_followups_per_turn() {
                    let recovery = recover_tool_followup_completion(
                        provider,
                        planning_request,
                        &user_message,
                        &hop_records,
                        &next_tool_call,
                        response.assistant_message.as_ref(),
                        &build_tool_followup_limit_error(max_tool_followups_per_turn()),
                        &context_observation,
                    );
                    provider_call_records.push(recovery.provider_call_record);
                    accumulated_token_usage = merge_token_usage(
                        accumulated_token_usage,
                        recovery.response.token_usage.as_ref(),
                    );
                    accumulated_fallback_reason = merge_fallback_reason(
                        accumulated_fallback_reason,
                        recovery.response.fallback_reason.clone(),
                    );
                    response = recovery.response;
                } else if completed_hops >= max_tool_hops_per_turn() {
                    return Err(self.fail_sync_turn_result(
                        Some(provider_meta),
                        display_message,
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities,
                        hook_trace_records,
                        build_tool_hop_limit_error(max_tool_hops_per_turn()),
                    ));
                } else {
                    let next_signature = tool_call_signature(&next_tool_call);
                    if !seen_tool_signatures.insert(next_signature) {
                        let recovery = recover_tool_followup_completion(
                            provider,
                            planning_request,
                            &user_message,
                            &hop_records,
                            &next_tool_call,
                            response.assistant_message.as_ref(),
                            &build_duplicate_tool_call_error(&next_tool_call),
                            &context_observation,
                        );
                        provider_call_records.push(recovery.provider_call_record);
                        accumulated_token_usage = merge_token_usage(
                            accumulated_token_usage,
                            recovery.response.token_usage.as_ref(),
                        );
                        accumulated_fallback_reason = merge_fallback_reason(
                            accumulated_fallback_reason,
                            recovery.response.fallback_reason.clone(),
                        );
                        response = recovery.response;
                    } else {
                        current_assistant_message = response.assistant_message.clone();
                        current_assistant_output_text = response.output_text.clone();
                        current_assistant_reasoning = response.reasoning_content.clone();
                        current_assistant_reasoning_value =
                            response.reasoning_content_value.clone();
                        current_tool_call = next_tool_call;
                        continue;
                    }
                }
            }

            return Ok(SyncToolTurnOutcome {
                assistant_message: response.output_text.clone(),
                assistant_reasoning_content: response.reasoning_content.clone(),
                provider_native_transcript: native_transcript_for_tool_turn(
                    provider.protocol(),
                    &user_message,
                    &hop_records,
                    &response,
                ),
                provider_source: response.provider_source,
                provider_mode: response.provider_mode,
                fallback_reason: accumulated_fallback_reason,
                token_usage: accumulated_token_usage,
                trace_steps: self
                    .telemetry_builder
                    .completed_trace_with_tool(all_tools_ok),
                model_hop_trace_contents: model_hop_trace_contents(&hop_records),
                tool_activities,
                hook_trace_records,
                first_token_latency_ms,
            });
        }
    }


    pub fn run_turn(&self, input: TurnInput) -> TurnResult {
        self.run_turn_with_facts(input, RunTurnFacts::default())
    }


    /// PA-095：同步入口的事件 turn_id（与 sync trace 的 turn_id 同构，
    /// 保证事件流与 trace 记录可关联）。nanos 提供跨进程区分度；进程内
    /// 原子计数器保证唯一——turn 开始时 elapsed 仅数百 ns，低于 Windows
    /// 计时器精度，纯 nanos 会使同会话连续两轮产生相同 id（trace 互相
    /// 覆盖 + 事件 (turn_id, seq) 冲突 flush 失败）。
    fn sync_turn_event_id(input: &TurnInput) -> String {
        static SYNC_TURN_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        format!(
            "sync:{}:{}{:04x}",
            input.session_id.as_deref().unwrap_or("local-dev-session"),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
            SYNC_TURN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) % 0x10000,
        )
    }


    /// PA-095：同步入口失败终态事件——发射 turn:failed（触发 turn/end Error；
    /// 无 token 字段故不触发 provider/usage，早期失败无 usage 数据可结算）。
    /// turn_id 由调用方传入（与 turn:started 同源，保证配对不变式）。
    /// 返回本次发射的信封（调用方复用为 TurnResult 终态信封，避免二次分配）。
    #[allow(clippy::too_many_arguments)]
    fn emit_sync_turn_failed(
        &self,
        turn_id: &str,
        input: &TurnInput,
        started_at: &Instant,
        provider_meta: Option<&ProviderEventMeta>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        first_token_latency_ms: Option<u64>,
        error: String,
    ) -> crate::agent::turn_flow::TurnEventEnvelope {
        let sink = crate::agent::turn_flow::NoopTurnEventSink;
        crate::agent::turn_flow::emit_stream_failed(
            &sink,
            turn_id.to_string(),
            provider_meta,
            trace_steps,
            Some(tool_activities),
            first_token_latency_ms,
            Some(started_at.elapsed().as_millis() as u64),
            None,
            None,
            None,
            None,
            error,
            input.session_id.clone(),
        )
    }


    /// `run_turn` with explicit session/run/turn/workspace facts for governed Ask
    /// control-request persistence (design.md Decision 5). The host control plane supplies the
    /// graph run's facts before each graph-run turn; the plain `run_turn` entry uses `None` facts
    /// and still binds the input's session. A pending Ask resume injection (phase-4 P0) is
    /// consumed here and seeded into the turn's provider context.
    pub(crate) fn run_turn_with_facts(&self, input: TurnInput, facts: RunTurnFacts) -> TurnResult {
        self.apply_governed_turn_context(&input, &facts);
        let ask_injection = self.take_pending_ask_injection(facts.run_id.as_deref());
        self.run_turn_inner(input, ask_injection)
    }


    fn run_turn_inner(
        &self,
        input: TurnInput,
        ask_injection: Option<GraphAskResumeInjection>,
    ) -> TurnResult {
        let turn_started_at = Instant::now();
        // PA-095：同步入口事件 turn_id 一次性计算——started/failed/completed 全路径
        // 复用同一 id，保证"每个 turn:started 必有配对 turn:end"的配对不变式。
        let sync_turn_id = Self::sync_turn_event_id(&input);
        let prepared = match self.prepare_turn(&input, false, ask_injection.as_ref()) {
            Ok(prepared) => prepared,
            Err(error) => {
                return build_failed_turn_result(
                    None,
                    String::new(),
                    error,
                    self.telemetry_builder.failed_trace_empty_input(),
                    Vec::new(),
                );
            }
        };

        runtime_log(format!(
            "turn:run requested={} provider={} protocol={} model={} message_preview={}",
            prepared.provider.requested_name(),
            prepared.provider.name(),
            prepared.provider.protocol_label(),
            prepared.provider.model(),
            preview_text(&prepared.user_message, 120)
        ));
        let provider_meta = provider_event_meta(&prepared.provider);
        // PA-095：同步入口事件化——prepare 成功即发射 turn:started（payload.text
        // 携带用户消息 → 触发 user/message；build_context_observation → 触发
        // context/observation 外置）。事件持久化走全局注册表，NoopSink 不做推送。
        {
            let sync_sink = crate::agent::turn_flow::NoopTurnEventSink;
            crate::agent::turn_flow::emit_stream_event(
                &sync_sink,
                "turn:started",
                sync_turn_id.clone(),
                "started",
                Some("calling_model"),
                Some(prepared.user_message.clone()),
                None,
                Some(&provider_meta),
                None,
                None,
                None,
                Some(prepared.build_context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                input.session_id.clone(),
            );
        }
        let model_call_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
        let initial_model_hook_trace_records = model_call_hook_outcome.trace_records.clone();
        if let Some(error) = model_call_hook_outcome.fail_turn_error {
            // PA-095：终态信封取自发射（TurnResult 携带事件流同一身份）。
            let envelope = self.emit_sync_turn_failed(
                &sync_turn_id,
                &input,
                &turn_started_at,
                Some(&provider_meta),
                Vec::new(),
                Vec::new(),
                None,
                error.clone(),
            );
            let mut result = self.fail_sync_turn_result(
                Some(&provider_meta),
                prepared.display_message,
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                model_call_hook_outcome.trace_records,
                error,
            );
            self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
            return result;
        }

        let planned = match self.plan_turn(&prepared) {
            Ok(planned) => planned,
            Err(error) => {
                let envelope = self.emit_sync_turn_failed(
                    &sync_turn_id,
                    &input,
                    &turn_started_at,
                    Some(&provider_meta),
                    Vec::new(),
                    Vec::new(),
                    None,
                    error.clone(),
                );
                let mut result = self.fail_sync_turn_result(
                    Some(&provider_meta),
                    prepared.display_message,
                    self.telemetry_builder.failed_trace_before_tool(),
                    Vec::new(),
                    initial_model_hook_trace_records.clone(),
                    error,
                );
                self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
                return result;
            }
        };

        let PlannedTurn {
            first_decision,
            resolved_tool_call,
            initial_decision_duration_ms,
            planner_hook_trace_records,
        } = planned;
        let PreparedTurn {
            user_message,
            display_message,
            provider,
            tools,
            planning_request,
            build_context_observation,
            ..
        } = prepared;
        let mut provider_call_records = vec![build_provider_call_cache_record(
            ProviderRequestKind::InitialRequest,
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.token_usage.as_ref(),
            None,
            initial_decision_duration_ms,
            ProviderLatencyKind::BufferedResponse,
            Some(&build_context_observation),
        )];

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            let mut planning_hook_trace_records = initial_model_hook_trace_records.clone();
            planning_hook_trace_records.extend(planner_hook_trace_records.clone());
            let envelope = self.emit_sync_turn_failed(
                &sync_turn_id,
                &input,
                &turn_started_at,
                Some(&provider_meta),
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                None,
                error.clone(),
            );
            let mut result = self.fail_sync_turn_result(
                Some(&provider_meta),
                display_message,
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                planning_hook_trace_records,
                error,
            );
            self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
            return result;
        }
        let mut planning_hook_trace_records = initial_model_hook_trace_records.clone();
        planning_hook_trace_records.extend(planner_hook_trace_records.clone());
        let (
            assistant_message,
            assistant_reasoning_content,
            provider_native_transcript,
            provider_source,
            provider_mode,
            fallback_reason,
            token_usage,
            trace_steps,
            tool_activities,
            model_hop_trace_contents,
            mut hook_trace_records,
            first_token_latency_ms,
        ) = if let Some(tool_call) = resolved_tool_call {
            let initial_visible_first_token_latency_ms = None;
            match self.handle_sync_tool_turn(
                user_message.clone(),
                display_message.clone(),
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                &first_decision,
                tool_call,
                planning_hook_trace_records.clone(),
                initial_visible_first_token_latency_ms,
                &turn_started_at,
                &mut provider_call_records,
            ) {
                Ok(outcome) => (
                    outcome.assistant_message,
                    outcome.assistant_reasoning_content,
                    outcome.provider_native_transcript,
                    outcome.provider_source,
                    outcome.provider_mode,
                    outcome.fallback_reason,
                    outcome.token_usage,
                    outcome.trace_steps,
                    outcome.tool_activities,
                    outcome.model_hop_trace_contents,
                    outcome.hook_trace_records,
                    outcome.first_token_latency_ms,
                ),
                Err(mut failed_result) => {
                    // PA-095：工具轮失败——发射 turn:failed 保持 start/end 配对
                    // （错误文本取 session_summary，与 failed result 呈现一致）；
                    // 终态信封取自发射并应用到结果（事件流身份一致）。
                    let envelope = self.emit_sync_turn_failed(
                        &sync_turn_id,
                        &input,
                        &turn_started_at,
                        Some(&provider_meta),
                        Vec::new(),
                        Vec::new(),
                        None,
                        failed_result.session_summary.clone(),
                    );
                    self.apply_terminal_envelope_to_turn_result(&mut failed_result, &envelope);
                    return failed_result;
                }
            }
        } else {
            (
                first_decision.output_text.clone(),
                first_decision.reasoning_content.clone(),
                native_transcript_for_completed_turn(
                    &user_message,
                    &first_decision,
                    provider.requires_provider_native_tool_flow(),
                ),
                first_decision.provider_source.clone(),
                first_decision.provider_mode.clone(),
                first_decision.fallback_reason.clone(),
                first_decision.token_usage.clone(),
                self.telemetry_builder.completed_trace_without_tool(),
                Vec::new(),
                Vec::new(),
                planning_hook_trace_records.clone(),
                None,
            )
        };
        let attachments = match self.save_input_attachments(&input) {
            Ok(attachments) => attachments,
            Err(error) => {
                let failed_trace_steps =
                    self.failed_trace_steps_for_tool_activities(&tool_activities);
                self.persist_failed_sync_turn_trace_with_hooks(
                    input.session_id.as_deref(),
                    &sync_turn_id,
                    &display_message,
                    Some(&provider_meta),
                    failed_trace_steps.clone(),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    hook_trace_records.clone(),
                    Some(provider_source.clone()),
                    Some(provider_mode.clone()),
                    Some(build_context_observation.clone()),
                    fallback_reason.clone(),
                    first_token_latency_ms,
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    error.clone(),
                );
                // PA-095：附件保存失败——事件流 turn/end 配对；终态信封取自
                // 发射本身（同源序列号，避免二次分配错位）。
                let envelope = self.emit_sync_turn_failed(
                    &sync_turn_id,
                    &input,
                    &turn_started_at,
                    Some(&provider_meta),
                    failed_trace_steps.clone(),
                    tool_activities.clone(),
                    first_token_latency_ms,
                    error.clone(),
                );
                self.annotate_sync_terminal_trace_with_envelope(
                    input.session_id.as_deref(),
                    &sync_turn_id,
                    &envelope,
                );
                let mut result = build_failed_turn_result_with_hooks(
                    Some(&provider_meta),
                    display_message,
                    error,
                    failed_trace_steps,
                    tool_activities,
                    hook_trace_records,
                );
                self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
                return result;
            }
        };
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            &display_message,
            &assistant_message,
            provider.name(),
            &provider_mode,
            token_usage.as_ref(),
            provider_native_transcript,
            attachments,
            input.workspace_mode.as_deref(),
        );
        let turn_duration_ms = Some(turn_started_at.elapsed().as_millis() as u64);
        let checkpoint_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::CheckpointPersistEnd);
        hook_trace_records.extend(checkpoint_hook_outcome.trace_records.clone());
        if let Some(error) = checkpoint_hook_outcome.fail_turn_error {
            let failed_trace_steps = self.failed_trace_steps_for_tool_activities(&tool_activities);
            self.persist_failed_sync_turn_trace_with_hooks(
                input.session_id.as_deref(),
                &sync_turn_id,
                &display_message,
                Some(&provider_meta),
                failed_trace_steps.clone(),
                tool_activities.clone(),
                provider_call_records.clone(),
                hook_trace_records.clone(),
                Some(provider_source.clone()),
                Some(provider_mode.clone()),
                Some(build_context_observation.clone()),
                fallback_reason.clone(),
                first_token_latency_ms,
                turn_duration_ms,
                error.clone(),
            );
            // PA-095：checkpoint hook 失败——事件流 turn/end 配对；终态信封取自发射。
            let envelope = self.emit_sync_turn_failed(
                &sync_turn_id,
                &input,
                &turn_started_at,
                Some(&provider_meta),
                failed_trace_steps.clone(),
                tool_activities.clone(),
                first_token_latency_ms,
                error.clone(),
            );
            self.annotate_sync_terminal_trace_with_envelope(
                input.session_id.as_deref(),
                &sync_turn_id,
                &envelope,
            );
            let mut result = self.fail_sync_turn_result(
                Some(&provider_meta),
                display_message,
                failed_trace_steps,
                tool_activities,
                hook_trace_records,
                error,
            );
            self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
            return result;
        }
        let finalize_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::TurnFinalizeEnd);
        hook_trace_records.extend(finalize_hook_outcome.trace_records.clone());
        if let Some(error) = finalize_hook_outcome.fail_turn_error {
            let failed_trace_steps = self.failed_trace_steps_for_tool_activities(&tool_activities);
            self.persist_failed_sync_turn_trace_with_hooks(
                input.session_id.as_deref(),
                &sync_turn_id,
                &display_message,
                Some(&provider_meta),
                failed_trace_steps.clone(),
                tool_activities.clone(),
                provider_call_records.clone(),
                hook_trace_records.clone(),
                Some(provider_source.clone()),
                Some(provider_mode.clone()),
                Some(build_context_observation.clone()),
                fallback_reason.clone(),
                first_token_latency_ms,
                turn_duration_ms,
                error.clone(),
            );
            // PA-095：finalize hook 失败——事件流 turn/end 配对；终态信封取自发射。
            let envelope = self.emit_sync_turn_failed(
                &sync_turn_id,
                &input,
                &turn_started_at,
                Some(&provider_meta),
                failed_trace_steps.clone(),
                tool_activities.clone(),
                first_token_latency_ms,
                error.clone(),
            );
            self.annotate_sync_terminal_trace_with_envelope(
                input.session_id.as_deref(),
                &sync_turn_id,
                &envelope,
            );
            let mut result = self.fail_sync_turn_result(
                Some(&provider_meta),
                display_message,
                failed_trace_steps,
                tool_activities,
                hook_trace_records,
                error,
            );
            self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
            return result;
        }
        // PA-095：trace turn_id 与事件 turn_id 同源（sync_turn_id），保证事件流
        // 与 trace 记录可关联。
        let trace_turn_id = sync_turn_id.clone();
        self.persist_turn_trace_with_provider_calls_and_hooks(
            input.session_id.as_deref(),
            &trace_turn_id,
            &display_message,
            "completed",
            trace_steps.clone(),
            tool_activities.clone(),
            &model_hop_trace_contents,
            provider_call_records.clone(),
            hook_trace_records.clone(),
            Some(&provider_meta),
            Some(provider_source.clone()),
            Some(provider_mode.clone()),
            Some(build_context_observation.clone()),
            Some(assistant_message.clone()),
            assistant_reasoning_content.clone(),
            fallback_reason.clone(),
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            Some(persisted.session_summary.clone()),
            None,
        );
        // PA-095：同步入口成功终态——发射 turn:completed（携带 token 结算与
        // per-call records → 触发 assistant/message + provider/usage×N + turn/end
        // 三连事件，与 streaming 入口等价）；终态信封取自发射本身（同源序列号）。
        let terminal_envelope = {
            let sync_sink = crate::agent::turn_flow::NoopTurnEventSink;
            crate::agent::turn_flow::emit_stream_event(
                &sync_sink,
                "turn:completed",
                trace_turn_id.clone(),
                "completed",
                Some("completed"),
                Some(assistant_message.clone()),
                assistant_reasoning_content.clone(),
                Some(&provider_meta),
                Some(provider_source.clone()),
                Some(provider_mode.clone()),
                fallback_reason.clone(),
                Some(build_context_observation.clone()),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency_ms,
                turn_duration_ms,
                Some(trace_steps.clone()),
                None,
                Some(tool_activities.clone()),
                Some(provider_call_records.clone()),
                Some(hook_trace_records.clone()),
                Some(persisted.session_summary.clone()),
                input.session_id.clone(),
            )
        };
        self.annotate_sync_terminal_trace_with_envelope(
            input.session_id.as_deref(),
            &trace_turn_id,
            &terminal_envelope,
        );

        let trace_timeline = build_persisted_trace_timeline(
            display_message.as_str(),
            "completed",
            Some(&provider_meta),
            Some(provider_source.as_str()),
            Some(provider_mode.as_str()),
            Some(&build_context_observation),
            &tool_activities,
            &model_hop_trace_contents,
            Some(assistant_message.as_str()),
            assistant_reasoning_content.as_deref(),
            fallback_reason.as_deref(),
            None,
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
        );

        let mut result = TurnResult {
            event_id: None,
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: "ready".to_string(),
            provider_requested_name: provider.requested_name().to_string(),
            provider_name: provider.name().to_string(),
            provider_protocol: provider.protocol_label().to_string(),
            provider_model: provider.model().to_string(),
            provider_source,
            provider_mode,
            fallback_reason,
            build_context_observation: Some(build_context_observation),
            input_tokens: persisted.input_tokens,
            cache_hit_input_tokens: persisted.cache_hit_input_tokens,
            reasoning_tokens: persisted.reasoning_tokens,
            output_tokens: persisted.output_tokens,
            total_tokens: persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms: Some(turn_started_at.elapsed().as_millis() as u64),
            user_message: display_message,
            assistant_message,
            trace_steps,
            trace_timeline,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            session_summary: persisted.session_summary,
        };
        self.apply_terminal_envelope_to_turn_result(&mut result, &terminal_envelope);
        result
    }

}
