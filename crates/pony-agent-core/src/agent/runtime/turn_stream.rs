use super::*;

impl AgentRuntime {
    #[allow(clippy::too_many_arguments)]
    fn handle_stream_tool_turn<S: TurnEventSink>(
        &self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        input: &TurnInput,
        user_message: &str,
        display_message: &str,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_call: ToolCall,
        initial_hook_trace_records: Vec<HookTraceRecord>,
        initial_turn_first_token_latency_ms: Option<u64>,
        turn_started_at: &Instant,
        provider_call_records: &mut Vec<ProviderCallCacheRecord>,
    ) {
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
        let first_token_latency = Rc::new(Cell::new(initial_turn_first_token_latency_ms));
        let mut hook_trace_records = initial_hook_trace_records;
        let mut seen_tool_signatures = BTreeSet::from([tool_call_signature(&current_tool_call)]);
        let mut consecutive_failures = ConsecutiveFailureTracker::new();
        let mut accumulated_messages: Vec<Value> = Vec::new();

        loop {
            completed_hops += 1;
            let trace_steps = self.telemetry_builder.trace_tool_active();
            self.update_execution_checkpoint(
                control,
                turn_id,
                "calling_tool",
                Some(provider_meta),
                completed_hops.saturating_sub(1),
                Some(current_tool_call.name.as_str()),
                &trace_steps,
                &tool_activities,
                None,
                None,
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            if self.should_cancel_turn(control, turn_id) {
                self.cancel_stream_turn(
                    sink,
                    control,
                    turn_id,
                    input.session_id.as_deref(),
                    display_message,
                    &input.images,
                    Some(provider_meta),
                    trace_steps,
                    tool_activities.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(context_observation.clone()),
                );
                return;
            }
            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.to_string(),
                "trace",
                Some("executing_tool"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(trace_steps),
                None,
                None,
                None,
                None,
                None,
                None,
            );

            let running_tool_activities = running_tool_activities_with_history(
                &tool_activities,
                self.telemetry_builder
                    .tool_activities_running(&current_tool_call),
            );
            let tool_start_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallStart);
            let tool_start_hook_trace_records = tool_start_hook_outcome.trace_records.clone();
            hook_trace_records.extend(tool_start_hook_trace_records.clone());
            emit_stream_event(
                sink,
                "turn:tool",
                turn_id.to_string(),
                "tool",
                Some("executing_tool"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &running_tool_activities,
                    &model_hop_trace_contents_with_current(
                        &hop_records,
                        current_assistant_output_text.as_str(),
                        current_assistant_reasoning.as_deref(),
                    ),
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_tool",
                )),
                Some(running_tool_activities),
                None,
                Some(tool_start_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = tool_start_hook_outcome.fail_turn_error {
                let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    trace_steps,
                    running_tool_activities_with_history(
                        &tool_activities,
                        self.telemetry_builder
                            .tool_activities_running(&current_tool_call),
                    ),
                    provider_call_records.clone(),
                    tool_start_hook_outcome.trace_records,
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
            }

            let execution_context = ToolExecutionContext {
                workspace_root: self.resolve_session_workspace_root(input.session_id.as_deref()),
                ..Default::default()
            };
            let (tool_result, invocation_record, capability_hook_trace_records) =
                self.execute_registered_tool_call_with_context(&current_tool_call, &execution_context);
            hook_trace_records.extend(capability_hook_trace_records);
            all_tools_ok &= tool_result.status == "ok";
            tool_activities.extend(annotate_capability_tool_activities(
                self.telemetry_builder
                    .tool_activities_after_result(&current_tool_call, &tool_result),
                invocation_record,
            ));
            let return_trace_steps = self.telemetry_builder.trace_return_active(all_tools_ok);
            self.update_execution_checkpoint(
                control,
                turn_id,
                "calling_model",
                Some(provider_meta),
                completed_hops,
                Some(current_tool_call.name.as_str()),
                &return_trace_steps,
                &tool_activities,
                None,
                None,
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            hop_records.push(ToolTurnHopRecord {
                assistant_message: current_assistant_message.clone(),
                assistant_output_text: current_assistant_output_text.clone(),
                assistant_reasoning_content: current_assistant_reasoning.clone(),
                assistant_reasoning_content_value: current_assistant_reasoning_value.clone(),
                tool_call: current_tool_call.clone(),
                tool_result: tool_result.clone(),
            });
            if self.should_cancel_turn(control, turn_id) {
                self.cancel_stream_turn(
                    sink,
                    control,
                    turn_id,
                    input.session_id.as_deref(),
                    display_message,
                    &input.images,
                    Some(provider_meta),
                    return_trace_steps.clone(),
                    tool_activities.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(context_observation.clone()),
                );
                return;
            }

            let tool_end_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallEnd);
            let tool_end_hook_trace_records = tool_end_hook_outcome.trace_records.clone();
            hook_trace_records.extend(tool_end_hook_trace_records.clone());
            emit_stream_event(
                sink,
                "turn:tool",
                turn_id.to_string(),
                "tool",
                Some("tool_result_integrating"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &tool_activities,
                    &model_hop_trace_contents(&hop_records),
                    None,
                    None,
                    first_token_latency.get(),
                    "calling_model",
                )),
                Some(tool_activities.clone()),
                None,
                Some(tool_end_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = tool_end_hook_outcome.fail_turn_error {
                let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    trace_steps,
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    tool_end_hook_outcome.trace_records,
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
            }

            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.to_string(),
                "trace",
                Some("tool_result_integrating"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(return_trace_steps),
                None,
                None,
                None,
                None,
                None,
                None,
            );

            // Design Decision 5 (PA-076 P1-1): a pending control outcome (`control_outcome_pending`,
            // surfaced as a structured `error.code`) means the dispatcher persisted an
            // `Interaction`/`Approval` `PendingControlRequest` instead of producing a
            // provider-consumable result. The run must pause — bind the Ask wait to the graph run
            // when a store + run are available, emit the terminal `turn:suspended` event — and
            // never feed the pending marker back to the provider as an ordinary tool error.
            if tool_result_control_outcome_pending(&tool_result) {
                let suspension = self.suspend_turn_for_control_outcome(
                    &current_tool_call,
                    &tool_result,
                );
                if let Some(request) = &suspension.pending_request {
                    runtime_log(format!(
                        "turn:ask-suspend-stream hop={} request={} kind={:?} call_id={} bound_run={:?}",
                        completed_hops,
                        request.request_id,
                        request.request_kind,
                        request.call_id,
                        suspension.bound_run_id
                    ));
                }
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
                let suspended_trace_steps =
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                emit_stream_event(
                    sink,
                    "turn:suspended",
                    turn_id.to_string(),
                    "suspended",
                    Some(SUSPENDED_TURN_PHASE),
                    Some(assistant_message),
                    None,
                    Some(provider_meta),
                    None,
                    None,
                    Some("control_outcome_pending".to_string()),
                    Some(context_observation.clone()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(suspended_trace_steps),
                    None,
                    Some(tool_activities.clone()),
                    Some(provider_call_records.clone()),
                    Some(hook_trace_records.clone()),
                    Some(session_summary),
                    input.session_id.clone(),
                );
                return;
            }

            // Only abort the turn if the tool was intentionally stopped (e.g. user cancellation).
            // Tool execution errors (status "error") are fed back to the model so it can
            // decide how to respond — e.g. try a different path, use another tool, or
            // explain the problem to the user.
            if tool_result.status == "aborted" {
                let error = build_tool_execution_error(
                    &current_tool_call.name,
                    tool_result.output.as_str(),
                );
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(false),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    hook_trace_records.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
            }

            // Consecutive-failure stop-loss: the same tool failing with the same error code
            // `limit` times in a row means the model is retrying a doomed invocation. Stop the
            // follow-up loop and surface the real error instead of burning the remaining
            // follow-up budget on identical retries.
            let failure_signal = tool_failure_signal(&current_tool_call, &tool_result);
            let failure_count = consecutive_failures.record(failure_signal.clone());
            if failure_count >= max_consecutive_tool_failures_per_turn() {
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(false),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    hook_trace_records.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    build_consecutive_tool_failure_error(
                        max_consecutive_tool_failures_per_turn(),
                        &current_tool_call.name,
                        failure_signal
                            .as_ref()
                            .map(|(_, code)| code.as_str())
                            .unwrap_or("unknown"),
                        tool_result.output.as_str(),
                    ),
                );
                return;
            }

            let followup_model_hook_trace_records = self
                .dispatch_hook_trace_records(TurnHookPoint::ModelCallStart)
                .trace_records;
            // PA-095 #4：followup call 的 step/start——payload.step 携带即将发起
            // 的 hop 索引（completed_hops 在循环头已自增，初始 call=0、首个
            // followup=1），经 build_step_start_event 映射为 StepStart 事件。
            emit_stream_event_with_step(
                sink,
                "turn:trace",
                turn_id.to_string(),
                "trace",
                Some("calling_model"),
                None,
                None,
                Some(provider_meta),
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(self.telemetry_builder.trace_return_active(all_tools_ok)),
                None,
                Some(tool_activities.clone()),
                None,
                Some(followup_model_hook_trace_records),
                None,
                input.session_id.clone(),
                Some(completed_hops as u32),
            );

            let delta_turn_id = turn_id.to_string();
            let first_token_latency_for_emit = Rc::clone(&first_token_latency);
            let turn_started_at_for_latency = *turn_started_at;
            let supports_true_streaming_followup = provider.supports_true_streaming_followup();
            let provider_call_first_token_latency = Rc::new(Cell::new(None));
            let provider_call_first_token_latency_for_emit =
                Rc::clone(&provider_call_first_token_latency);
            let reasoning_batcher = Rc::new(RefCell::new(StreamReasoningBatcher::default()));
            let reasoning_batcher_for_emit = Rc::clone(&reasoning_batcher);
            let last_emit_for_client = Rc::new(Cell::new(0u64));
            let last_emit_for_client_clone = Rc::clone(&last_emit_for_client);
            let delta_turn_id_for_emit = delta_turn_id.clone();
            // PA-095 #4 修复：followup delta 闭包此前 session_id 传 None——事件以
            // 空 session 缓冲且永不 flush（生产事件流丢失全部 followup chunk）。
            let delta_session_id_for_emit = input.session_id.clone();
            let provider_call_started_at = Instant::now();
            // PA-095 #4：本次 followup call 的 hop 索引（Copy 捕获进 delta 闭包）。
            let followup_step = completed_hops as u32;
            let response = match provider_followup_stream(
                provider,
                planning_request,
                tools,
                &mut accumulated_messages,
                current_assistant_message.as_ref(),
                &current_tool_call,
                &tool_result,
                move |delta| {
                    let flush_delta = |text: Option<String>,
                                       reasoning_content: Option<String>,
                                       latency: Option<u64>| {
                        emit_stream_event_with_step(
                            sink,
                            "turn:delta",
                            delta_turn_id_for_emit.clone(),
                            "delta",
                            Some("calling_model"),
                            text,
                            reasoning_content,
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
                            latency,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            delta_session_id_for_emit.clone(),
                            Some(followup_step),
                        );
                    };
                    if supports_true_streaming_followup
                        && provider_call_first_token_latency_for_emit.get().is_none()
                    {
                        let value = provider_call_started_at.elapsed().as_millis() as u64;
                        provider_call_first_token_latency_for_emit.set(Some(value));
                    }
                    let latency = if supports_true_streaming_followup
                        && first_token_latency_for_emit.get().is_none()
                    {
                        let value = turn_started_at_for_latency.elapsed().as_millis() as u64;
                        first_token_latency_for_emit.set(Some(value));
                        Some(value)
                    } else {
                        None
                    };

                    let mut did_emit = false;
                    match delta {
                        ProviderStreamChunk::Text(text) => {
                            if let Some(reasoning) = reasoning_batcher_for_emit.borrow_mut().flush()
                            {
                                flush_delta(None, Some(reasoning), latency);
                            }
                            flush_delta(Some(text), None, latency);
                            did_emit = true;
                        }
                        ProviderStreamChunk::Reasoning(reasoning) => {
                            if let Some(buffered_reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().push(reasoning)
                            {
                                flush_delta(None, Some(buffered_reasoning), latency);
                                did_emit = true;
                            }
                        }
                    }
                    if did_emit {
                        let now = turn_started_at_for_latency.elapsed().as_millis() as u64;
                        let prev = last_emit_for_client_clone.get();
                        let gap = now.saturating_sub(prev);
                        const MIN_CLIENT_GAP_MS: u64 = 16;
                        if prev > 0 && gap < MIN_CLIENT_GAP_MS {
                            std::thread::sleep(std::time::Duration::from_millis(
                                MIN_CLIENT_GAP_MS - gap,
                            ));
                        }
                        last_emit_for_client_clone.set(now);
                    }
                },
            ) {
                Ok(response) => response,
                Err(error) => {
                    self.fail_stream_turn_with_hook_dispatch(
                        sink,
                        control,
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        Some(provider_meta),
                        Some(context_observation.clone()),
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        hook_trace_records.clone(),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        completed_hops,
                        error,
                    );
                    return;
                }
            };
            if let Some(buffered_reasoning) = reasoning_batcher.borrow_mut().flush() {
                emit_stream_event_with_step(
                    sink,
                    "turn:delta",
                    delta_turn_id.clone(),
                    "delta",
                    Some("calling_model"),
                    None,
                    Some(buffered_reasoning.clone()),
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
                    None,
                    None,
                    None,
                    None,
                    None,
                    input.session_id.clone(),
                    Some(followup_step),
                );
                let now = turn_started_at_for_latency.elapsed().as_millis() as u64;
                let prev = last_emit_for_client.get();
                let gap = now.saturating_sub(prev);
                const MIN_CLIENT_GAP_MS: u64 = 16;
                if prev > 0 && gap < MIN_CLIENT_GAP_MS {
                    std::thread::sleep(std::time::Duration::from_millis(MIN_CLIENT_GAP_MS - gap));
                }
            }
            let mut response = response;
            let provider_call_duration_ms = provider_call_started_at.elapsed().as_millis() as u64;
            let provider_call_used_true_stream =
                response.provider_source == "provider_followup_stream";
            provider_call_records.push(build_provider_call_cache_record(
                ProviderRequestKind::ToolFollowup,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                response.token_usage.as_ref(),
                if provider_call_used_true_stream {
                    provider_call_first_token_latency.get()
                } else {
                    None
                },
                Some(provider_call_duration_ms),
                if provider_call_used_true_stream {
                    ProviderLatencyKind::ProviderStream
                } else {
                    ProviderLatencyKind::BufferedResponse
                },
                Some(&context_observation),
            ));
            accumulated_token_usage =
                merge_token_usage(accumulated_token_usage, response.token_usage.as_ref());
            accumulated_fallback_reason = merge_fallback_reason(
                accumulated_fallback_reason,
                response.fallback_reason.clone(),
            );
            let (hop_input_tokens, hop_cache_hit, hop_reasoning, hop_output, hop_total) =
                token_usage_parts(accumulated_token_usage.as_ref());
            emit_stream_event(
                sink,
                "turn:hop_complete",
                turn_id.to_string(),
                "hop_complete",
                Some("streaming_response"),
                None,
                None,
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                None,
                None,
                hop_input_tokens,
                hop_cache_hit,
                hop_reasoning,
                hop_output,
                hop_total,
                first_token_latency.get(),
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                None,
                None,
                None,
                None,
                input.session_id.clone(),
            );
            let return_trace_steps = self.telemetry_builder.trace_return_active(all_tools_ok);
            self.update_execution_checkpoint(
                control,
                turn_id,
                "calling_model",
                Some(provider_meta),
                completed_hops,
                None,
                &return_trace_steps,
                &tool_activities,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            if self.should_cancel_turn(control, turn_id) {
                self.cancel_stream_turn(
                    sink,
                    control,
                    turn_id,
                    input.session_id.as_deref(),
                    display_message,
                    &input.images,
                    Some(provider_meta),
                    return_trace_steps.clone(),
                    tool_activities.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(context_observation.clone()),
                );
                return;
            }

            if let Some(error) = provider_failure_message(
                &response.provider_mode,
                response.fallback_reason.as_deref(),
            ) {
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    hook_trace_records.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
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
                        self.fail_stream_turn_with_hook_dispatch(
                            sink,
                            control,
                            input.session_id.as_deref(),
                            turn_id,
                            display_message,
                            Some(provider_meta),
                            Some(context_observation.clone()),
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                            tool_activities.clone(),
                            provider_call_records.clone(),
                            hook_trace_records.clone(),
                            first_token_latency.get(),
                            Some(turn_started_at.elapsed().as_millis() as u64),
                            completed_hops,
                            error,
                        );
                        return;
                    }
                };
                response.tool_call = Some(normalized.tool_call);
                response.assistant_message = normalized.assistant_message;
            }

            if let Some(next_tool_call) = response.tool_call.clone() {
                if completed_hops >= max_tool_followups_per_turn() {
                    let recovery = recover_tool_followup_completion_stream(
                        sink,
                        provider,
                        planning_request,
                        &user_message,
                        &hop_records,
                        &next_tool_call,
                        response.assistant_message.as_ref(),
                        &build_tool_followup_limit_error(max_tool_followups_per_turn()),
                        &context_observation,
                        turn_id,
                        input.session_id.clone(),
                        &first_token_latency,
                        turn_started_at,
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
                    self.fail_stream_turn_with_hook_dispatch(
                        sink,
                        control,
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        Some(provider_meta),
                        Some(context_observation.clone()),
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        hook_trace_records.clone(),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        completed_hops,
                        build_tool_hop_limit_error(max_tool_hops_per_turn()),
                    );
                    return;
                } else {
                    let next_signature = tool_call_signature(&next_tool_call);
                    if !seen_tool_signatures.insert(next_signature) {
                        let recovery = recover_tool_followup_completion_stream(
                            sink,
                            provider,
                            planning_request,
                            &user_message,
                            &hop_records,
                            &next_tool_call,
                            response.assistant_message.as_ref(),
                            &build_duplicate_tool_call_error(&next_tool_call),
                            &context_observation,
                            turn_id,
                            input.session_id.clone(),
                            &first_token_latency,
                            turn_started_at,
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

            let completed_text = response.output_text.clone();
            let completed_mode = response.provider_mode.clone();
            let attachments = match self.save_input_attachments(input) {
                Ok(attachments) => attachments,
                Err(error) => {
                    self.fail_stream_turn_with_hook_dispatch(
                        sink,
                        control,
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        Some(provider_meta),
                        Some(context_observation.clone()),
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        hook_trace_records.clone(),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        completed_hops,
                        error,
                    );
                    return;
                }
            };
            let persisted = self.persist_turn_outcome(
                input.session_id.as_deref(),
                display_message,
                &completed_text,
                provider.name(),
                &completed_mode,
                accumulated_token_usage.as_ref(),
                native_transcript_for_tool_turn(
                    provider.protocol(),
                    user_message,
                    &hop_records,
                    &response,
                ),
                attachments,
                input.workspace_mode.as_deref(),
            );
            emit_stream_event(
                sink,
                "turn:output_end",
                turn_id.to_string(),
                "output_end",
                Some("streaming_response"),
                Some(response.output_text.clone()),
                response.reasoning_content.clone(),
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason.clone(),
                None,
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                None,
                None,
                None,
                None,
                input.session_id.clone(),
            );
            let trace_steps = self
                .telemetry_builder
                .completed_trace_with_tool(all_tools_ok);
            let turn_duration_ms = Some(turn_started_at.elapsed().as_millis() as u64);
            self.update_execution_checkpoint(
                control,
                turn_id,
                "checkpointing",
                Some(provider_meta),
                completed_hops,
                None,
                &trace_steps,
                &tool_activities,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            emit_stream_event(
                sink,
                "turn:phase_changed",
                turn_id.to_string(),
                "phase",
                Some("checkpointing"),
                None,
                None,
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason.clone(),
                Some(context_observation.clone()),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(trace_steps.clone()),
                None,
                Some(tool_activities.clone()),
                Some(provider_call_records.clone()),
                None,
                None,
                input.session_id.clone(),
            );
            let checkpoint_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::CheckpointPersistEnd);
            let checkpoint_hook_trace_records = checkpoint_hook_outcome.trace_records.clone();
            emit_stream_event(
                sink,
                "turn:checkpoint_persisted",
                turn_id.to_string(),
                "checkpoint",
                Some("checkpointing"),
                None,
                None,
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason.clone(),
                Some(context_observation.clone()),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(trace_steps.clone()),
                None,
                Some(tool_activities.clone()),
                Some(provider_call_records.clone()),
                Some(checkpoint_hook_trace_records.clone()),
                Some(persisted.session_summary.clone()),
                input.session_id.clone(),
            );
            if let Some(error) = checkpoint_hook_outcome.fail_turn_error {
                let mut checkpoint_terminal_hook_trace_records = hook_trace_records.clone();
                checkpoint_terminal_hook_trace_records
                    .extend(checkpoint_hook_trace_records.clone());
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    checkpoint_terminal_hook_trace_records,
                    first_token_latency.get(),
                    turn_duration_ms,
                    completed_hops,
                    error,
                );
                return;
            }
            let finalize_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::TurnFinalizeEnd);
            let finalize_hook_trace_records = finalize_hook_outcome.trace_records.clone();
            let mut terminal_hook_trace_records = hook_trace_records.clone();
            terminal_hook_trace_records.extend(checkpoint_hook_trace_records.clone());
            terminal_hook_trace_records.extend(finalize_hook_trace_records.clone());
            if let Some(error) = finalize_hook_outcome.fail_turn_error {
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    terminal_hook_trace_records,
                    first_token_latency.get(),
                    turn_duration_ms,
                    completed_hops,
                    error,
                );
                return;
            }
            self.update_execution_checkpoint(
                control,
                turn_id,
                "ready",
                Some(provider_meta),
                completed_hops,
                None,
                &trace_steps,
                &tool_activities,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                accumulated_fallback_reason.as_deref(),
                Some("completed"),
                None,
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                turn_id,
                display_message,
                "completed",
                trace_steps.clone(),
                tool_activities.clone(),
                &model_hop_trace_contents(&hop_records),
                provider_call_records.clone(),
                terminal_hook_trace_records.clone(),
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                Some(context_observation.clone()),
                Some(response.output_text.clone()),
                response.reasoning_content.clone(),
                accumulated_fallback_reason.clone(),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(persisted.session_summary.clone()),
                None,
            );

            let completed_timeline = build_persisted_trace_timeline(
                display_message,
                "completed",
                Some(provider_meta),
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                Some(&context_observation),
                &tool_activities,
                &model_hop_trace_contents(&hop_records),
                Some(response.output_text.as_str()),
                response.reasoning_content.as_deref(),
                accumulated_fallback_reason.as_deref(),
                None,
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
            );
            emit_stream_event(
                sink,
                "turn:completed",
                turn_id.to_string(),
                "completed",
                Some("completed"),
                Some(response.output_text.clone()),
                response.reasoning_content.clone(),
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason.clone(),
                Some(context_observation.clone()),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(trace_steps),
                Some(completed_timeline),
                Some(tool_activities),
                Some(provider_call_records.clone()),
                Some(terminal_hook_trace_records),
                Some(persisted.session_summary),
                input.session_id.clone(),
            );
            return;
        }
    }


    #[allow(dead_code)]
    pub fn start_turn_stream<S: TurnEventSink>(
        &mut self,
        sink: &S,
        turn_id: String,
        input: TurnInput,
    ) {
        let control = ExecutionControlRegistry::new();
        control.register_turn(&turn_id, input.session_id.as_deref(), None);
        self.start_turn_stream_with_control(sink, &control, turn_id, input);
    }


    pub fn start_turn_stream_with_control<S: TurnEventSink>(
        &self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: String,
        input: TurnInput,
    ) {
        self.start_turn_stream_with_control_and_facts(
            sink,
            control,
            turn_id,
            input,
            RunTurnFacts::default(),
        );
    }


    /// `start_turn_stream_with_control` with explicit run/turn/workspace facts for governed Ask
    /// control-request persistence (design.md Decision 5, P1-1). The host control plane supplies
    /// the graph run's facts before each graph-run streamed turn so a persisted
    /// `PendingControlRequest` (Ask) is bound to the real run/turn; the plain entry uses `None`
    /// facts and still binds the input's session.
    pub fn start_turn_stream_with_control_and_facts<S: TurnEventSink>(
        &self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: String,
        input: TurnInput,
        facts: RunTurnFacts,
    ) {
        // Bind the streamed turn's session/run/turn to the governed executor so an Ask control
        // request persists against the real invocation (design.md Decision 5, P1-1 wiring).
        self.apply_governed_turn_context(&input, &facts);
        // Consume a pending Ask resume injection (phase-4 P0) so the terminal tool result is
        // seeded into this turn's provider context.
        let ask_injection = self.take_pending_ask_injection(facts.run_id.as_deref());
        let turn_started_at = Instant::now();
        let prepared = match self.prepare_turn(&input, true, ask_injection.as_ref()) {
            Ok(prepared) => prepared,
            Err(error) => {
                emit_turn_failed(
                    sink,
                    turn_id,
                    None,
                    None,
                    None,
                    None,
                    self.telemetry_builder.failed_trace_empty_input(),
                    error,
                    input.session_id.clone(),
                );
                return;
            }
        };
        runtime_log(format!(
            "turn:start id={} requested={} provider={} protocol={} model={} message_preview={}",
            turn_id,
            prepared.provider.requested_name(),
            prepared.provider.name(),
            prepared.provider.protocol_label(),
            prepared.provider.model(),
            preview_text(&prepared.user_message, 120)
        ));
        let prepared_provider_meta = provider_event_meta(&prepared.provider);
        let start_trace_steps = self.telemetry_builder.start_trace_steps();
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "calling_model",
            Some(&prepared_provider_meta),
            0,
            None,
            &start_trace_steps,
            &[],
            None,
            None,
            None,
            Some("running"),
            None,
        );

        emit_stream_event(
            sink,
            "turn:started",
            turn_id.clone(),
            "started",
            Some("calling_model"),
            Some(prepared.user_message.clone()),
            None,
            Some(&prepared_provider_meta),
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
            Some(start_trace_steps.clone()),
            Some(build_stream_started_trace_timeline(
                prepared.user_message.as_str(),
                &prepared_provider_meta,
                &prepared.build_context_observation,
            )),
            None,
            None,
            None,
            None,
            input.session_id.clone(),
        );
        if self.should_cancel_turn(control, &turn_id) {
            self.cancel_stream_turn(
                sink,
                control,
                &turn_id,
                input.session_id.as_deref(),
                prepared.display_message.as_str(),
                &input.images,
                Some(&prepared_provider_meta),
                start_trace_steps,
                Vec::new(),
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(prepared.build_context_observation.clone()),
            );
            return;
        }

        let preflight_decision = self.planner.preflight_decision(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
        );
        let mut preflight_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::TurnPreflight,
            &self.build_planner_preflight_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                preflight_decision.as_ref(),
            ),
            preflight_decision,
            None,
        );
        if let Some(error) = preflight_dispatch.fail_turn_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                &[],
                Vec::new(),
                preflight_dispatch.trace_records,
                Some(&prepared_provider_meta),
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
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        if let Some(error) = preflight_dispatch.blocked_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                &[],
                Vec::new(),
                preflight_dispatch.trace_records,
                Some(&prepared_provider_meta),
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
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        let preflight_decision = preflight_dispatch.decision;
        let mut planner_hook_trace_records = preflight_dispatch.trace_records;
        planner_hook_trace_records.push(self.build_planner_preflight_trace_record(
            &prepared.user_message,
            &prepared.planner_skills,
            preflight_decision.as_ref(),
        ));
        let supports_true_streaming_decision = prepared.provider.supports_true_streaming_decision();
        let turn_id_for_stream = turn_id.clone();
        let stream_session_id = input.session_id.clone();
        let pending_model_call_fail_turn =
            Rc::new(RefCell::new(None::<(Vec<HookTraceRecord>, String)>));
        let pending_model_call_fail_turn_for_stream = Rc::clone(&pending_model_call_fail_turn);
        let stream_initial_decision = || -> Result<
            (
                ProviderDecision,
                Option<u64>,
                Option<u64>,
                Option<u64>,
                ProviderLatencyKind,
            ),
            String,
        > {
            let model_call_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
            let model_call_hook_trace_records = model_call_hook_outcome.trace_records.clone();
            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.clone(),
                "trace",
                Some("calling_model"),
                None,
                None,
                Some(&prepared_provider_meta),
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
                Some(start_trace_steps.clone()),
                None,
                None,
                None,
                Some(model_call_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = model_call_hook_outcome.fail_turn_error {
                *pending_model_call_fail_turn_for_stream.borrow_mut() =
                    Some((model_call_hook_outcome.trace_records, error));
                return Err(HOOK_FAILTURN_HANDLED_SENTINEL.to_string());
            }
            let initial_decision_started_at = Instant::now();
            let initial_turn_first_token_latency = Rc::new(Cell::new(None));
            let initial_call_first_token_latency = Rc::new(Cell::new(None));
            let initial_turn_first_token_latency_for_emit =
                Rc::clone(&initial_turn_first_token_latency);
            let initial_call_first_token_latency_for_emit =
                Rc::clone(&initial_call_first_token_latency);
            let reasoning_batcher = Rc::new(RefCell::new(StreamReasoningBatcher::default()));
            let reasoning_batcher_for_emit = Rc::clone(&reasoning_batcher);
            let last_emit_for_client = Rc::new(Cell::new(0u64));
            let last_emit_for_client_clone = Rc::clone(&last_emit_for_client);
            let turn_id_for_delta_emit = turn_id_for_stream.clone();
            let stream_session_id_for_delta_emit = stream_session_id.clone();

            let decision = provider_decision_stream(
                &prepared.provider,
                &prepared.planning_request,
                &prepared.tools,
                move |delta| {
                    let flush_delta = |text: Option<String>, reasoning_content: Option<String>| {
                        let turn_latency =
                            if initial_turn_first_token_latency_for_emit.get().is_none() {
                                let value = turn_started_at.elapsed().as_millis() as u64;
                                initial_turn_first_token_latency_for_emit.set(Some(value));
                                Some(value)
                            } else {
                                None
                            };
                        emit_stream_event(
                            sink,
                            "turn:delta",
                            turn_id_for_delta_emit.clone(),
                            "delta",
                            Some("calling_model"),
                            text,
                            reasoning_content,
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
                            turn_latency,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            stream_session_id_for_delta_emit.clone(),
                        );
                    };
                    if initial_call_first_token_latency_for_emit.get().is_none() {
                        let value = initial_decision_started_at.elapsed().as_millis() as u64;
                        initial_call_first_token_latency_for_emit.set(Some(value));
                    }
                    let mut did_emit = false;
                    match delta {
                        ProviderStreamChunk::Text(text) => {
                            if let Some(reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().flush()
                            {
                                flush_delta(None, Some(reasoning));
                            }
                            flush_delta(Some(text), None);
                            did_emit = true;
                        }
                        ProviderStreamChunk::Reasoning(reasoning) => {
                            if let Some(buffered_reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().push(reasoning)
                            {
                                flush_delta(None, Some(buffered_reasoning));
                                did_emit = true;
                            }
                        }
                    }
                    if did_emit {
                        let now = turn_started_at.elapsed().as_millis() as u64;
                        let prev = last_emit_for_client_clone.get();
                        let gap = now.saturating_sub(prev);
                        const MIN_CLIENT_GAP_MS: u64 = 16;
                        if prev > 0 && gap < MIN_CLIENT_GAP_MS {
                            std::thread::sleep(std::time::Duration::from_millis(
                                MIN_CLIENT_GAP_MS - gap,
                            ));
                        }
                        last_emit_for_client_clone.set(now);
                    }
                },
            )?;
            if let Some(buffered_reasoning) = reasoning_batcher.borrow_mut().flush() {
                let turn_latency = if initial_turn_first_token_latency.get().is_none() {
                    let value = turn_started_at.elapsed().as_millis() as u64;
                    initial_turn_first_token_latency.set(Some(value));
                    Some(value)
                } else {
                    None
                };
                emit_stream_event(
                    sink,
                    "turn:delta",
                    turn_id_for_stream.clone(),
                    "delta",
                    Some("calling_model"),
                    None,
                    Some(buffered_reasoning.clone()),
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
                    turn_latency,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    input.session_id.clone(),
                );
                let now = turn_started_at.elapsed().as_millis() as u64;
                let prev = last_emit_for_client.get();
                let gap = now.saturating_sub(prev);
                const MIN_CLIENT_GAP_MS: u64 = 16;
                if prev > 0 && gap < MIN_CLIENT_GAP_MS {
                    std::thread::sleep(std::time::Duration::from_millis(
                        MIN_CLIENT_GAP_MS - gap,
                    ));
                }
            }
            Ok((
                decision,
                Some(initial_decision_started_at.elapsed().as_millis() as u64),
                initial_turn_first_token_latency.get(),
                initial_call_first_token_latency.get(),
                ProviderLatencyKind::ProviderStream,
            ))
        };
        let pending_model_call_fail_turn_for_sync = Rc::clone(&pending_model_call_fail_turn);
        let decide_sync = || -> Result<
            (
                ProviderDecision,
                Option<u64>,
                Option<u64>,
                Option<u64>,
                ProviderLatencyKind,
            ),
            String,
        > {
            let model_call_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
            let model_call_hook_trace_records = model_call_hook_outcome.trace_records.clone();
            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.clone(),
                "trace",
                Some("calling_model"),
                None,
                None,
                Some(&prepared_provider_meta),
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
                Some(start_trace_steps.clone()),
                None,
                None,
                None,
                Some(model_call_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = model_call_hook_outcome.fail_turn_error {
                *pending_model_call_fail_turn_for_sync.borrow_mut() =
                    Some((model_call_hook_outcome.trace_records, error));
                return Err(HOOK_FAILTURN_HANDLED_SENTINEL.to_string());
            }
            let started_at = Instant::now();
            let decision = provider_decision(
                &prepared.provider,
                &prepared.planning_request,
                &prepared.tools,
            )?;
            Ok((
                decision,
                Some(started_at.elapsed().as_millis() as u64),
                None,
                None,
                ProviderLatencyKind::BufferedResponse,
            ))
        };
        let planned = (|| -> Result<
            (
                ProviderDecision,
                Option<u64>,
                Option<u64>,
                Option<u64>,
                ProviderLatencyKind,
            ),
            String,
        > {
            if prepared.provider.requires_provider_native_tool_flow() {
                // 与 plan_turn 一致：native tool flow 下必须由 provider 生成决策，
                // 保证 follow-up 回传的 assistant 消息带完整 reasoning_content。
                if supports_true_streaming_decision {
                    stream_initial_decision().or_else(|_| decide_sync())
                } else {
                    decide_sync()
                }
            } else {
                match preflight_decision {
                    Some(decision) => {
                        Ok((decision, None, None, None, ProviderLatencyKind::Unknown))
                    }
                    None => {
                        if supports_true_streaming_decision {
                            stream_initial_decision().or_else(|_| decide_sync())
                        } else {
                            decide_sync()
                        }
                    }
                }
            }
        })();
        let (
            mut first_decision,
            initial_decision_duration_ms,
            initial_turn_first_token_latency_ms,
            initial_call_first_token_latency_ms,
            initial_latency_kind,
        ) = match planned {
            Ok(result) => result,
            Err(error) => {
                if error == HOOK_FAILTURN_HANDLED_SENTINEL {
                    if let Some((hook_trace_records, hook_error)) =
                        pending_model_call_fail_turn.borrow_mut().take()
                    {
                        let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                        self.fail_stream_turn_with_hook_dispatch(
                            sink,
                            control,
                            input.session_id.as_deref(),
                            &turn_id,
                            &prepared.display_message,
                            Some(&prepared_provider_meta),
                            Some(prepared.build_context_observation.clone()),
                            trace_steps,
                            Vec::new(),
                            Vec::new(),
                            hook_trace_records,
                            None,
                            Some(turn_started_at.elapsed().as_millis() as u64),
                            0,
                            hook_error,
                        );
                    }
                    return;
                }
                let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                self.update_execution_checkpoint(
                    control,
                    &turn_id,
                    "failed",
                    Some(&prepared_provider_meta),
                    0,
                    None,
                    &trace_steps,
                    &[],
                    None,
                    None,
                    None,
                    Some("failed"),
                    Some(&error),
                );
                self.persist_turn_trace(
                    input.session_id.as_deref(),
                    &turn_id,
                    &prepared.display_message,
                    "failed",
                    trace_steps.clone(),
                    Vec::new(),
                    Some(&prepared_provider_meta),
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
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    None,
                    Some(error.clone()),
                );
                emit_turn_failed(
                    sink,
                    turn_id,
                    Some(prepared.provider.requested_name().to_string()),
                    Some(prepared.provider.name().to_string()),
                    Some(prepared.provider.protocol_label().to_string()),
                    Some(prepared.provider.model().to_string()),
                    trace_steps,
                    error,
                    input.session_id.clone(),
                );
                return;
            }
        };
        if let Some(tool_call) = first_decision.tool_call.take() {
            let normalized = match normalize_tool_directive(
                tool_call,
                first_decision.assistant_message.take(),
                &first_decision.output_text,
                first_decision.reasoning_content.as_deref(),
                first_decision.reasoning_content_value.as_ref(),
            ) {
                Ok(normalized) => normalized,
                Err(error) => {
                    let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                    self.update_execution_checkpoint(
                        control,
                        &turn_id,
                        "failed",
                        Some(&prepared_provider_meta),
                        0,
                        None,
                        &trace_steps,
                        &[],
                        None,
                        None,
                        None,
                        Some("failed"),
                        Some(&error),
                    );
                    self.persist_turn_trace(
                        input.session_id.as_deref(),
                        &turn_id,
                        &prepared.display_message,
                        "failed",
                        trace_steps.clone(),
                        Vec::new(),
                        Some(&prepared_provider_meta),
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
                        initial_turn_first_token_latency_ms,
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        None,
                        Some(error.clone()),
                    );
                    emit_turn_failed(
                        sink,
                        turn_id,
                        Some(prepared.provider.requested_name().to_string()),
                        Some(prepared.provider.name().to_string()),
                        Some(prepared.provider.protocol_label().to_string()),
                        Some(prepared.provider.model().to_string()),
                        trace_steps,
                        error,
                        input.session_id.clone(),
                    );
                    return;
                }
            };
            first_decision.tool_call = Some(normalized.tool_call);
            first_decision.assistant_message = normalized.assistant_message;
        }
        let resolved_tool_call = self.resolve_tool_call(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
            first_decision.tool_call.clone(),
            !prepared.provider.requires_provider_native_tool_flow(),
        );
        let mut tool_selection_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::ToolSelection,
            &self.build_planner_tool_selection_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                first_decision.tool_call.as_ref(),
                resolved_tool_call.as_ref(),
            ),
            None,
            resolved_tool_call,
        );
        if let Some(error) = tool_selection_dispatch.fail_turn_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                &[],
                Vec::new(),
                {
                    let mut records = planner_hook_trace_records.clone();
                    records.extend(tool_selection_dispatch.trace_records);
                    records
                },
                Some(&prepared_provider_meta),
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
                initial_turn_first_token_latency_ms,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        if let Some(error) = tool_selection_dispatch.blocked_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                &[],
                Vec::new(),
                {
                    let mut records = planner_hook_trace_records.clone();
                    records.extend(tool_selection_dispatch.trace_records);
                    records
                },
                Some(&prepared_provider_meta),
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
                initial_turn_first_token_latency_ms,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        let resolved_tool_call = tool_selection_dispatch.selected_tool_call;
        planner_hook_trace_records.extend(tool_selection_dispatch.trace_records);
        planner_hook_trace_records.push(self.build_planner_tool_selection_trace_record(
            &prepared.user_message,
            first_decision.tool_call.as_ref(),
            resolved_tool_call.as_ref(),
        ));
        let PreparedTurn {
            user_message,
            display_message,
            provider,
            tools,
            planning_request,
            build_context_observation,
            ..
        } = prepared;
        let provider_meta = provider_event_meta(&provider);
        let mut provider_call_records = vec![build_provider_call_cache_record(
            ProviderRequestKind::InitialRequest,
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.token_usage.as_ref(),
            if initial_latency_kind == ProviderLatencyKind::ProviderStream {
                initial_call_first_token_latency_ms
            } else {
                None
            },
            initial_decision_duration_ms,
            initial_latency_kind.clone(),
            Some(&build_context_observation),
        )];

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                Some(first_decision.provider_source.as_str()),
                Some(first_decision.provider_mode.as_str()),
                first_decision.fallback_reason.as_deref(),
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls(
                input.session_id.as_deref(),
                &turn_id,
                &display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                provider_call_records.clone(),
                Some(&provider_meta),
                Some(first_decision.provider_source.clone()),
                Some(first_decision.provider_mode.clone()),
                Some(build_context_observation.clone()),
                None,
                None,
                first_decision.fallback_reason.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                Some(error.clone()),
            );
            emit_stream_failed(
                sink,
                turn_id,
                Some(&provider_meta),
                trace_steps,
                None,
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(build_context_observation.clone()),
                None,
                Some(provider_call_records.clone()),
                None,
                error,
                input.session_id.clone(),
            );
            return;
        }

        if self.should_cancel_turn(control, &turn_id) {
            self.cancel_stream_turn(
                sink,
                control,
                &turn_id,
                input.session_id.as_deref(),
                &display_message,
                &input.images,
                Some(&provider_meta),
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(build_context_observation.clone()),
            );
            return;
        }

        if let Some(tool_call) = resolved_tool_call {
            self.handle_stream_tool_turn(
                sink,
                control,
                &turn_id,
                &input,
                &user_message,
                &display_message,
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                &first_decision,
                tool_call,
                planner_hook_trace_records.clone(),
                initial_turn_first_token_latency_ms,
                &turn_started_at,
                &mut provider_call_records,
            );
            return;
        }

        emit_stream_event(
            sink,
            "turn:trace",
            turn_id.clone(),
            "trace",
            Some("response_ready"),
            None,
            None,
            None,
            Some(first_decision.provider_source.clone()),
            None,
            None,
            Some(build_context_observation.clone()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(self.telemetry_builder.trace_return_active_without_tool()),
            None,
            None,
            None,
            None,
            None,
            input.session_id.clone(),
        );
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "calling_model",
            Some(&provider_meta),
            0,
            None,
            &self.telemetry_builder.trace_return_active_without_tool(),
            &[],
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.fallback_reason.as_deref(),
            Some("running"),
            None,
        );
        if self.should_cancel_turn(control, &turn_id) {
            self.cancel_stream_turn(
                sink,
                control,
                &turn_id,
                input.session_id.as_deref(),
                &display_message,
                &input.images,
                Some(&provider_meta),
                self.telemetry_builder.trace_return_active_without_tool(),
                Vec::new(),
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(build_context_observation.clone()),
            );
            return;
        }

        let first_token_latency_ms = if initial_latency_kind == ProviderLatencyKind::ProviderStream
        {
            initial_turn_first_token_latency_ms
        } else {
            let first_visible_first_token_latency_ms =
                if let Some(reasoning_content) = first_decision.reasoning_content.as_deref() {
                    stream_reasoning_chunks(
                        sink,
                        &turn_id,
                        "calling_model",
                        reasoning_content,
                        &turn_started_at,
                        None,
                        false,
                    )
                } else {
                    None
                };
            stream_text_chunks(
                sink,
                &turn_id,
                "calling_model",
                &first_decision.output_text,
                &turn_started_at,
                first_visible_first_token_latency_ms,
                false,
            )
        };
        let completed_text = first_decision.output_text.clone();
        let completed_mode = first_decision.provider_mode.clone();
        let attachments = match self.save_input_attachments(&input) {
            Ok(attachments) => attachments,
            Err(error) => {
                let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                self.update_execution_checkpoint(
                    control,
                    &turn_id,
                    "failed",
                    Some(&provider_meta),
                    0,
                    None,
                    &trace_steps,
                    &[],
                    Some(first_decision.provider_source.as_str()),
                    Some(first_decision.provider_mode.as_str()),
                    first_decision.fallback_reason.as_deref(),
                    Some("failed"),
                    Some(&error),
                );
                self.persist_turn_trace_with_provider_calls(
                    input.session_id.as_deref(),
                    &turn_id,
                    &display_message,
                    "failed",
                    trace_steps.clone(),
                    Vec::new(),
                    provider_call_records.clone(),
                    Some(&provider_meta),
                    Some(first_decision.provider_source.clone()),
                    Some(first_decision.provider_mode.clone()),
                    Some(build_context_observation.clone()),
                    None,
                    None,
                    first_decision.fallback_reason.clone(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    first_token_latency_ms,
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    None,
                    Some(error.clone()),
                );
                emit_stream_failed(
                    sink,
                    turn_id,
                    Some(&provider_meta),
                    trace_steps,
                    None,
                    first_token_latency_ms,
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(build_context_observation.clone()),
                    None,
                    Some(provider_call_records.clone()),
                    None,
                    error,
                    input.session_id.clone(),
                );
                return;
            }
        };
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            &display_message,
            &completed_text,
            provider.name(),
            &completed_mode,
            first_decision.token_usage.as_ref(),
            native_transcript_for_completed_turn(
                &user_message,
                &first_decision,
                provider.requires_provider_native_tool_flow(),
            ),
            attachments,
            input.workspace_mode.as_deref(),
        );
        emit_stream_event(
            sink,
            "turn:output_end",
            turn_id.clone(),
            "output_end",
            Some("streaming_response"),
            Some(first_decision.output_text.clone()),
            first_decision.reasoning_content.clone(),
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
            None,
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            Some(turn_started_at.elapsed().as_millis() as u64),
            None,
            None,
            None,
            None,
            None,
            None,
            input.session_id.clone(),
        );
        let trace_steps = self.telemetry_builder.completed_trace_without_tool();
        let turn_duration_ms = Some(turn_started_at.elapsed().as_millis() as u64);
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "checkpointing",
            Some(&provider_meta),
            0,
            None,
            &trace_steps,
            &[],
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.fallback_reason.as_deref(),
            Some("running"),
            None,
        );
        emit_stream_event(
            sink,
            "turn:phase_changed",
            turn_id.clone(),
            "phase",
            Some("checkpointing"),
            None,
            None,
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
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
            Some(Vec::new()),
            Some(provider_call_records.clone()),
            None,
            None,
            input.session_id.clone(),
        );
        let checkpoint_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::CheckpointPersistEnd);
        let checkpoint_hook_trace_records = checkpoint_hook_outcome.trace_records.clone();
        emit_stream_event(
            sink,
            "turn:checkpoint_persisted",
            turn_id.clone(),
            "checkpoint",
            Some("checkpointing"),
            None,
            None,
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
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
            Some(Vec::new()),
            Some(provider_call_records.clone()),
            Some(checkpoint_hook_trace_records.clone()),
            Some(persisted.session_summary.clone()),
            input.session_id.clone(),
        );
        if let Some(error) = checkpoint_hook_outcome.fail_turn_error {
            let mut checkpoint_terminal_hook_trace_records = planner_hook_trace_records.clone();
            checkpoint_terminal_hook_trace_records.extend(checkpoint_hook_trace_records.clone());
            self.fail_stream_turn_with_hook_dispatch(
                sink,
                control,
                input.session_id.as_deref(),
                &turn_id,
                &display_message,
                Some(&provider_meta),
                Some(build_context_observation.clone()),
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                provider_call_records.clone(),
                checkpoint_terminal_hook_trace_records,
                first_token_latency_ms,
                turn_duration_ms,
                0,
                error,
            );
            return;
        }
        let finalize_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::TurnFinalizeEnd);
        let finalize_hook_trace_records = finalize_hook_outcome.trace_records.clone();
        let mut terminal_hook_trace_records = planner_hook_trace_records.clone();
        terminal_hook_trace_records.extend(checkpoint_hook_trace_records.clone());
        terminal_hook_trace_records.extend(finalize_hook_trace_records.clone());
        if let Some(error) = finalize_hook_outcome.fail_turn_error {
            self.fail_stream_turn_with_hook_dispatch(
                sink,
                control,
                input.session_id.as_deref(),
                &turn_id,
                &display_message,
                Some(&provider_meta),
                Some(build_context_observation.clone()),
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                provider_call_records.clone(),
                terminal_hook_trace_records,
                first_token_latency_ms,
                turn_duration_ms,
                0,
                error,
            );
            return;
        }
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "ready",
            Some(&provider_meta),
            0,
            None,
            &trace_steps,
            &[],
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.fallback_reason.as_deref(),
            Some("completed"),
            None,
        );
        self.persist_turn_trace_with_provider_calls_and_hooks(
            input.session_id.as_deref(),
            &turn_id,
            &display_message,
            "completed",
            trace_steps.clone(),
            Vec::new(),
            &[],
            provider_call_records.clone(),
            terminal_hook_trace_records.clone(),
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            Some(build_context_observation.clone()),
            Some(first_decision.output_text.clone()),
            first_decision.reasoning_content.clone(),
            first_decision.fallback_reason.clone(),
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

        let completed_timeline = build_persisted_trace_timeline(
            display_message.as_str(),
            "completed",
            Some(&provider_meta),
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            Some(&build_context_observation),
            &[],
            &[],
            Some(first_decision.output_text.as_str()),
            first_decision.reasoning_content.as_deref(),
            first_decision.fallback_reason.as_deref(),
            None,
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
        );
        emit_stream_event(
            sink,
            "turn:completed",
            turn_id,
            "completed",
            Some("completed"),
            Some(first_decision.output_text.clone()),
            first_decision.reasoning_content.clone(),
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
            Some(build_context_observation.clone()),
            Some(persisted.input_tokens).flatten(),
            Some(persisted.cache_hit_input_tokens).flatten(),
            Some(persisted.reasoning_tokens).flatten(),
            Some(persisted.output_tokens).flatten(),
            Some(persisted.total_tokens).flatten(),
            first_token_latency_ms,
            turn_duration_ms,
            Some(trace_steps),
            Some(completed_timeline),
            Some(Vec::new()),
            Some(provider_call_records),
            Some(terminal_hook_trace_records),
            Some(persisted.session_summary),
            input.session_id.clone(),
        );
    }
}
