use super::*;

impl AgentRuntime {
    /// D3a（P1-1 / B018）：同步工具轮驱动 —— 一次上游响应里的**整批**工具调用都在本函数内执行。
    ///
    /// 批语义（`.dev-team/designs/multi-tool-call-execution.md` §3）：
    /// - 批内**顺序**执行，逐调用复用 `execute_registered_tool_call_with_context`（治理/沙箱/预算
    ///   路径与单调用完全一致），逐调用一条 [`ToolTurnHopRecord`]、逐调用计 hop 与熔断
    ///   （per-call 计数语义，不因批而放宽）；
    /// - 批执行完**一次** followup 回填整批结果（N>1 走 provider 的批量入口）；
    /// - 交互类工具（`ToolKind::Interactive`：Ask / Permission 等）是**批终止符**：执行到它即停，
    ///   其后调用不执行（Ask 的 `control_outcome_pending` 直接挂起本轮，剩余调用留给恢复后的
    ///   下一 hop 重新规划）；
    /// - **批长 1**（绝大多数场景）逐字走既有 1:1 路径：同一执行入口 + 同一 `provider_followup_with_mode`。
    fn handle_sync_tool_turn(
        &self,
        session_id: Option<&str>,
        user_message: String,
        display_message: String,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_calls: Vec<ToolCall>,
        initial_model_hook_trace_records: Vec<HookTraceRecord>,
        first_token_latency_ms: Option<u64>,
        provider_call_records: &mut Vec<ProviderCallCacheRecord>,
        governed_context: Option<&DispatchContext>,
    ) -> Result<SyncToolTurnOutcome, TurnResult> {
        let context_observation = build_context_observation(planning_request, tools);
        let mut hop_records = Vec::new();
        let mut tool_activities = Vec::new();
        let mut current_tool_calls = tool_calls;
        let mut current_assistant_message = first_decision.assistant_message.clone();
        let mut current_assistant_output_text = first_decision.output_text.clone();
        let mut current_assistant_reasoning = first_decision.reasoning_content.clone();
        let mut current_assistant_reasoning_value = first_decision.reasoning_content_value.clone();
        let mut all_tools_ok = true;
        let mut completed_hops = 0usize;
        let mut accumulated_fallback_reason = first_decision.fallback_reason.clone();
        let mut accumulated_token_usage = first_decision.token_usage.clone();
        let mut hook_trace_records = initial_model_hook_trace_records;
        // 同签名重复调用闸（契约矩阵 C2/C3）：同一工具 + 同一归一化参数允许**有限次**执行，
        // 第 `limit + 1` 次命中闸后走恢复式收尾。**本 turn 起始一律建空计数（`new()`）**——
        // 若按首批调用预置计数（note 最初的 `from_batch`），预置 1 次 + 执行点再记 1 次构成
        // 双重计数，默认 limit=2 时第 2 次即熔断，直接违反「前 2 次放行」契约。
        let mut same_signatures = SameSignatureTracker::new();
        let mut consecutive_failures = ConsecutiveFailureTracker::new();
        let mut accumulated_messages: Vec<Value> = Vec::new();
        // D4（W28b / B018 后续）：批内执行段规划见 `batch_plan`；开关 `PONY_AGENT_PARALLEL_TOOLS`
        // 默认关 ⇒ 每段都是「预阶段 → 执行 → 后阶段」的单调用段，逐调用交织语义与本波之前逐字一致。
        let mut executed_calls: Vec<ToolCall> = Vec::new();
        let mut executed_results: Vec<ToolResult> = Vec::new();
        // 交互类工具（批终止符）命中标记：后阶段置位，段循环据此停批。
        let mut sync_batch_terminated: bool;

        // 预阶段（逐调用、保序）：hop 限额/计数、ToolCallStart hook。
        //
        // 为什么用函数内宏：本阶段与后阶段共享 10+ 个可变局部状态（hop_records / tool_activities /
        // hook_trace_records / 熔断跟踪器 / …），提取成方法会把「逐调用语义等价」藏进重构本身；
        // 宏展开的是**同一段代码文本**，`return` 也直接结束本函数，行为与内联逐字等价。
        macro_rules! sync_tool_call_pre {
            ($tool_call:expr) => {{
                let tool_call: &ToolCall = $tool_call;
                // hop 限额仍按**调用**计数：批内任一调用将越过上限即硬停，
                // 与批间同口径（不因批而放宽；批长 1 时该闸恒被批间闸先命中）。
                if completed_hops >= max_tool_hops_per_turn() {
                    return Err(self.fail_sync_turn_result(
                        Some(provider_meta),
                        display_message,
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities,
                        hook_trace_records,
                        build_tool_hop_limit_error(max_tool_hops_per_turn()),
                    ));
                }
                completed_hops += 1;
                runtime_log(format!(
                    "turn:tool-execute hop={} name={} args={}",
                    completed_hops, tool_call.name, tool_call.arguments
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
                            .tool_activities_running(tool_call),
                        hook_trace_records,
                        error,
                    ));
                }

            }};
        }

        // 后阶段（逐调用、保序）：记账、tool span、熔断闸、取消 / pending / aborted 判定、批终止符。
        macro_rules! sync_tool_call_post {
            ($tool_call:expr, $outcome:expr) => {{
                let tool_call: &ToolCall = $tool_call;
                let tool_outcome = $outcome;
                let tool_result = tool_outcome.result;
                let invocation_record = tool_outcome.invocation_record;
                hook_trace_records.extend(tool_outcome.capability_hook_trace_records);
                runtime_log(format!(
                    "turn:tool-result hop={} name={} status={} output_preview={}",
                    completed_hops,
                    tool_result.tool_name,
                    tool_result.status,
                    preview_text(&tool_result.output, 160)
                ));
                all_tools_ok &= tool_result.status == "ok";
                // Wave43 task-8 挂载切片：实时路径「工具执行结束 → 产生 TurnToolActivity」单点。
                // 一层工具执行 = 一个 tool.call span：只对顶层活动（parent_activity_id 为 None）记录，
                // 子调用活动从属顶层活动，避免 span 基数爆炸。
                let otel_activity_batch = self
                    .telemetry_builder
                    .tool_activities_after_result(tool_call, &tool_result);
                if let Some(otel_top_activity) = otel_activity_batch
                    .iter()
                    .find(|activity| activity.parent_activity_id.is_none())
                {
                    let otel_tool_error_kind = turn_tool_activity_error_kind(otel_top_activity);
                    crate::agent::otel::record_tool_call_span_with_call_id_and_kind(
                        crate::agent::otel::ToolSpanInput {
                            tool_name: otel_top_activity
                                .canonical_tool_name
                                .clone()
                                .unwrap_or_else(|| otel_top_activity.name.clone()),
                            status: if otel_top_activity.status == "done" {
                                "success".to_string()
                            } else {
                                "failed".to_string()
                            },
                            duration_ms: otel_top_activity
                                .duration_seconds
                                .map(|seconds| (seconds * 1000.0).round() as u64),
                            error: turn_tool_activity_error_text(otel_top_activity),
                            // P0-1：工具 span 的入参/结果预览（trace 可复盘的最小切片）。
                            // 此处传原始串即可 —— 脱敏与 2KB 截断统一在 `tool_span_attributes` 内完成
                            // （`None` ⇒ 不写键；序列化失败也不写，绝不用空串占位）。
                            args_preview: serde_json::to_string(&tool_call.arguments).ok(),
                            result_preview: Some(tool_result.output.clone()),
                        },
                        tool_call.call_id.clone(),
                        otel_tool_error_kind.as_deref(),
                    );
                }
                tool_activities.extend(annotate_capability_tool_activities(
                    otel_activity_batch,
                    invocation_record,
                ));
                hop_records.push(ToolTurnHopRecord {
                    assistant_message: current_assistant_message.clone(),
                    assistant_output_text: current_assistant_output_text.clone(),
                    assistant_reasoning_content: current_assistant_reasoning.clone(),
                    assistant_reasoning_content_value: current_assistant_reasoning_value.clone(),
                    tool_call: tool_call.clone(),
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
                        tool_call,
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
                    // W1b：治理路径已产出挂起标记，却匹配不到可恢复的待处理请求 ⇒
                    // 不得只发一条裸 `suspended`（用户永远等不到恢复），以可读失败终态收口。
                    if self.suspension_without_recoverable_request(&suspension) {
                        // F3（NFR）：日志 code 必须与终态错误码一致（首段），见 turn_control。
                        let unrecoverable_error =
                            super::turn_control::build_unrecoverable_suspension_error(&suspension);
                        runtime_log(format!(
                            "turn:control-suspend-unrecoverable hop={} name={} code={}",
                            completed_hops,
                            tool_call.name,
                            super::turn_control::suspension_error_code(&unrecoverable_error)
                        ));
                        return Err(self.fail_sync_turn_result(
                            Some(provider_meta),
                            display_message,
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                            tool_activities,
                            hook_trace_records,
                            unrecoverable_error,
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
                            &tool_call.name,
                            tool_result.output.as_str(),
                        ),
                    ));
                }

                // Consecutive-failure stop-loss: the same tool failing with the same error code
                // `limit` times in a row means the model is retrying a doomed invocation. Stop the
                // follow-up loop and surface the real error instead of burning the remaining
                // follow-up budget on identical retries.
                let failure_signal = tool_failure_signal(tool_call, &tool_result);
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
                            &tool_call.name,
                            failure_signal
                                .as_ref()
                                .map(|(_, _, code)| code.as_str())
                                .unwrap_or("unknown"),
                            tool_result.output.as_str(),
                        ),
                    ));
                }
                // P1-3 第二道闸：同一工具连续失败（忽略入参/错误码差异），与流式路径同口径。
                if consecutive_failures.same_tool_count()
                    >= max_consecutive_same_tool_failures_per_turn()
                {
                    return Err(self.fail_sync_turn_result(
                        Some(provider_meta),
                        display_message,
                        self.telemetry_builder.failed_trace_after_tool(false),
                        tool_activities,
                        hook_trace_records,
                        build_same_tool_consecutive_failure_error(
                            max_consecutive_same_tool_failures_per_turn(),
                            &tool_call.name,
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
                // 逐调用记录（供批后一次性回填）+ 同签名计数在**执行点**记账（闸点只读；
                // 被拦下的调用不记账，否则收尾后的重试会被自己推高计数）。
                same_signatures.record_executed(
                    tool_call_signature(tool_call),
                    tool_execution_changed_world(tool_call, &tool_result),
                );
                executed_results.push(tool_result.clone());
                executed_calls.push(tool_call.clone());
                // 批终止符：交互类工具（Ask / Permission…）执行完即停批，后续调用不执行。
                if sync_tool_call_ends_batch(tool_call) {
                    runtime_log(format!(
                        "turn:tool-batch-terminator hop={} name={} executed={} skipped={}",
                        completed_hops,
                        tool_call.name,
                        executed_calls.len(),
                        current_tool_calls.len().saturating_sub(executed_calls.len())
                    ));
                    sync_batch_terminated = true;
                }
            }};
        }

        loop {
            // ── 1) 执行本 hop 的**整批**调用（段内可并发；规划与引擎接线见 `batch_plan`）──
            executed_calls.clear();
            executed_results.clear();
            sync_batch_terminated = false;
            for segment in plan_batch_segments(&current_tool_calls) {
                // 1a) 预阶段：逐调用（保序）计 hop、跑 ToolCallStart hook。
                for tool_call in current_tool_calls[segment.calls.clone()].iter() {
                    sync_tool_call_pre!(tool_call);
                }
                // 1b) 执行阶段：并发段整段交 `parallel_tools` 引擎（结果保序）；其余逐调用顺序执行。
                let outcomes: Vec<ToolCallOutcome> = if segment.parallel {
                    let segment_calls: Vec<ToolCall> =
                        current_tool_calls[segment.calls.clone()].to_vec();
                    // W2：宿主当前 provider 选择随每次调用下传（子代理/队友继承源）。
                    let host_selection = Some(provider.selection());
                    match execute_segment_in_parallel(&segment_calls, |call| {
                        self.execute_tool_call_with_turn_context(
                            session_id,
                            governed_context,
                            call,
                            host_selection,
                        )
                    }) {
                        Ok(outcomes) => outcomes,
                        Err(error) => {
                            // fail-safe：引擎入参校验失败（段长恒一致 ⇒ 生产不可达）退回逐调用执行。
                            runtime_log(format!("turn:tool-parallel-fallback error={error}"));
                            segment_calls
                                .iter()
                                .map(|call| {
                                    self.execute_tool_call_with_turn_context(
                                        session_id,
                                        governed_context,
                                        call,
                                        host_selection,
                                    )
                                })
                                .collect()
                        }
                    }
                } else {
                    vec![self.execute_tool_call_with_turn_context(
                        session_id,
                        governed_context,
                        &current_tool_calls[segment.calls.start],
                        Some(provider.selection()),
                    )]
                };
                // 1c) 后阶段：逐调用（保序）回填；交互类工具命中即停批（后续段不执行）。
                for (tool_call, outcome) in current_tool_calls[segment.calls.clone()]
                    .iter()
                    .zip(outcomes.into_iter())
                {
                    sync_tool_call_post!(tool_call, outcome);
                }
                if sync_batch_terminated {
                    break;
                }
            }

            // ── 2) 批后**一次** followup：N==1 走既有 1:1 入口（逐字不变），N>1 走批量入口 ──
            let provider_call_started_at = Instant::now();
            let followup_result = if executed_calls.len() == 1 {
                provider_followup_with_mode(
                    provider,
                    planning_request,
                    tools,
                    &mut accumulated_messages,
                    current_assistant_message.as_ref(),
                    &executed_calls[0],
                    &executed_results[0],
                    crate::agent::turn_flow::LlmRequestMode::ToolFollowup,
                )
            } else {
                provider_followup_with_calls(
                    provider,
                    planning_request,
                    tools,
                    &mut accumulated_messages,
                    current_assistant_message.as_ref(),
                    &executed_calls,
                    &executed_results,
                    crate::agent::turn_flow::LlmRequestMode::ToolFollowup,
                )
            };
            let response = match followup_result {
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
                // wave-54 C4：协议归一上下文输入口径（anthropic ⇒ input + cache_read）。
                Some(provider.protocol_label()),
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

            // 本 hop 之后是否还有调用：单值字段是「还有调用」的总开关（恢复路径会把它清空），
            // 复数位给出整批（D3a 起 OpenAI 非流式入口全量上报；测试替身只填单值时回落单批）。
            let mut next_tool_calls: Vec<ToolCall> = match response.tool_call.take() {
                Some(single) => {
                    let mut calls = std::mem::take(&mut response.tool_calls);
                    if calls.is_empty() {
                        calls.push(single);
                    }
                    calls
                }
                None => Vec::new(),
            };

            if let Some(next_tool_call) = next_tool_calls.first().cloned() {
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
                response.tool_call = normalized.tool_call.clone();
                response.assistant_message = normalized.assistant_message;
                match normalized.tool_call {
                    Some(normalized_call) => next_tool_calls[0] = normalized_call,
                    None => {
                        // 空洞 tool call 已丢弃 ⇒ 整批作废：余下调用没有可回填的 assistant
                        // 锚点，不能单独执行；用兜底文本继续本轮（不进工具执行）。
                        next_tool_calls.clear();
                        if response.output_text.trim().is_empty()
                            && !normalized.output_text.trim().is_empty()
                        {
                            response.output_text = normalized.output_text;
                        }
                    }
                }
            }

            if let Some(next_tool_call) = next_tool_calls.first().cloned() {
                if completed_hops >= max_tool_followups_per_turn() {
                    let recovery = recover_tool_followup_completion(
                        provider,
                        planning_request,
                        &mut accumulated_messages,
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
                    // 重复调用闸：按**调用**判定（批内首个）、**只读**（不记账、不改状态）；
                    // 命中则恢复式收尾，不整批重放。
                    let next_signature = tool_call_signature(&next_tool_call);
                    let same_signature_limit = max_same_signature_occurrences_per_turn();
                    if same_signatures.is_exhausted(&next_signature, same_signature_limit) {
                        let occurrences = same_signatures.occurrence_count(&next_signature) + 1;
                        let recovery = recover_tool_followup_completion(
                            provider,
                            planning_request,
                            &mut accumulated_messages,
                            &hop_records,
                            &next_tool_call,
                            response.assistant_message.as_ref(),
                            &build_duplicate_tool_call_error(
                                &next_tool_call,
                                occurrences,
                                same_signature_limit,
                            ),
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
                        current_tool_calls = next_tool_calls;
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
            input.session_id.as_deref().unwrap_or(""),
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
            // P0-3：同步/CLI 非流式路径失败时**未向 UI 分片渲染**任何 assistant 正文
            // （`TurnInput` 只有 user message 与图片），故无已产出正文可保留。
            None,
        )
    }


    /// `run_turn` with explicit session/run/turn/workspace facts for governed Ask
    /// control-request persistence (design.md Decision 5). The host control plane supplies the
    /// graph run's facts before each graph-run turn; the plain `run_turn` entry uses `None` facts
    /// and still binds the input's session. A pending Ask resume injection (phase-4 P0) is
    /// consumed here and seeded into the turn's provider context.
    pub(crate) fn run_turn_with_facts(&self, input: TurnInput, facts: RunTurnFacts) -> TurnResult {
        // T2-1：turn 事实在**本 turn 内部**解析并随每次工具调用下传（不再写进 executor 共享槽位）。
        let governed_context = self.resolve_governed_turn_context(&input, &facts);
        let injection_lease = self.peek_pending_ask_injection(
            facts.run_id.as_deref(),
            input.session_id.as_deref(),
        );
        let ask_injection = injection_lease
            .as_ref()
            .map(|lease| lease.injection.clone());
        let result = self.run_turn_inner(input, ask_injection, governed_context);
        // Contract §5 ack boundary: only a successful terminal state settles the injection
        // (`completed` / `suspended`). The sync success path leaves `phase` at its initial
        // `ready` marker (the terminal envelope carries the `turn:completed` fact), while the
        // failure/suspend paths set `failed` / `cancelled` / `suspended` explicitly.
        let ack_reason = match result.phase.as_str() {
            "failed" | "cancelled" => None,
            crate::agent::runtime::SUSPENDED_TURN_PHASE => Some("suspended"),
            _ => Some("completed"),
        };
        if let (Some(reason), Some(lease)) = (ack_reason, injection_lease.as_ref()) {
            self.ack_pending_ask_injection(lease, reason);
        }
        result
    }


    fn run_turn_inner(
        &self,
        input: TurnInput,
        ask_injection: Option<GraphAskResumeInjection>,
        governed_context: Option<DispatchContext>,
    ) -> TurnResult {
        let turn_started_at = Instant::now();
        // PA-095：同步入口事件 turn_id 一次性计算——started/failed/completed 全路径
        // 复用同一 id，保证"每个 turn:started 必有配对 turn:end"的配对不变式。
        let sync_turn_id = Self::sync_turn_event_id(&input);
        // Wave43 task-8 挂载切片：turn root span（agent.turn）在入口创建，函数返回时 Drop
        // 自动结束 span，覆盖同步轮全部终态出口。
        // ADR 0055 P0-3：序号取「会话已登记轮次数」（0-based），不再硬编码 0。
        let sync_turn_sequence = self.session_turn_sequence(input.session_id.as_deref());
        // W3 trace：队友/子代理 runtime 继承宿主 traceparent（宿主 runtime 为 `None` ⇒ root）。
        let mut otel_turn_scope = crate::agent::otel::begin_turn_trace_with_parent(
            input.session_id.as_deref().unwrap_or(""),
            &sync_turn_id,
            sync_turn_sequence,
            self.parent_traceparent.as_deref(),
        );
        // Wave-7 B003（同步入口同构）：**turn 启动前**的用量治理判定（一次性、fail-open）。
        // 与流式入口同一策略与同一决策入口，避免「只挡流式 ⇒ 可绕过」。
        {
            let governance = {
                let store = self.sessions.read().unwrap_or_else(|error| {
                    eprintln!("[pony-agent] sessions rwlock poisoned: {error}, recovering");
                    error.into_inner()
                });
                crate::agent::usage_governance::evaluate_with_store(
                    &store,
                    input.session_id.as_deref(),
                )
            };
            if let crate::agent::usage_governance::Decision::Block {
                reason, current, limit, ..
            } = &governance.decision
            {
                // L3 B3：同步入口同样把治理阻断标记为「预期内失败」（不产生 issue）。
                crate::agent::ponysentry::mark_turn_failure_expected(&sync_turn_id);
                let error = format!(
                    "用量治理已阻断本轮：当前已用 ${current:.6}，上限 ${limit:.6}（触发 {reason}）。\
                     如需继续，请提高上限或关闭「强制拦截」——设置 → 用量治理 → 关闭 enforcement（一键出口）。"
                );
                otel_turn_scope.set_phase("failed");
                return build_failed_turn_result(
                    None,
                    String::new(),
                    error,
                    self.telemetry_builder.failed_trace_empty_input(),
                    Vec::new(),
                );
            }
        }

        let prepared = match self.prepare_turn(&input, false, ask_injection.as_ref()) {
            Ok(prepared) => prepared,
            Err(error) => {
                otel_turn_scope.set_phase("failed");
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
            otel_turn_scope.set_phase("failed");
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
                otel_turn_scope.set_phase("failed");
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
            // wave-54 C4：协议归一上下文输入口径（anthropic ⇒ input + cache_read）。
            Some(provider.protocol_label()),
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
            otel_turn_scope.set_phase("failed");
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
            // D3a：首个 hop 的整批调用 = planner 修补后的首个调用 + 上游响应的余下调用（保序）。
            let tool_calls = sync_tool_batch(tool_call, &first_decision.tool_calls);
            match self.handle_sync_tool_turn(
                input.session_id.as_deref(),
                user_message.clone(),
                display_message.clone(),
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                &first_decision,
                tool_calls,
                planning_hook_trace_records.clone(),
                initial_visible_first_token_latency_ms,
                &mut provider_call_records,
                governed_context.as_ref(),
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
                    // Ask 等暂停态不是终态：相位留空（不输出该键），恢复后由其自身出口定相位。
                    if failed_result.phase == SUSPENDED_TURN_PHASE {
                        otel_turn_scope.set_phase("");
                    } else {
                        otel_turn_scope.set_phase("failed");
                    }
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
                otel_turn_scope.set_phase("failed");
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
            otel_turn_scope.set_phase("failed");
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
            // wave-54 C2：失败轮也落已发生调用的加权速度真值（无 records ⇒ 无 speed 键）。
            crate::agent::turn_flow::record_turn_speed_truth(
                &mut otel_turn_scope,
                &provider_call_records,
                Some(turn_started_at.elapsed().as_millis() as u64),
            );
            otel_turn_scope.set_phase("failed");
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

        let trace_timeline = build_persisted_trace_timeline_with_provider_calls(
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
            // Wave-15 R2：同步入口同一序号映射（初始调用 + followup）。
            &provider_call_records,
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
        // wave-54 C2：根 span 速度/耗时真值（生产路径唯一出口，数据源 = 本轮 per-call records）。
        crate::agent::turn_flow::record_turn_speed_truth(
            &mut otel_turn_scope,
            &result.provider_call_records,
            result.turn_duration_ms,
        );
        otel_turn_scope.set_phase("completed");
        result
    }

}


/// D3a（P1-1 / B018）：批终止符判定 —— **交互类**工具执行后立即结束本批。
///
/// 判定真源是工具注册表的 `ToolKind`（`tools::tool_kind_for_name`：纯函数、名字先 canonicalize
/// 再查表），不另设名字白名单——新增交互工具只要登记 kind 即自动获得批终止语义。
///
/// 为什么必须停批（设计 §3.1）：交互类工具（Ask / Permission 等）等待宿主应答期间，若继续执行
/// 批内其余调用，会出现「已经停顿却仍在改工作区」的不可解释状态。Ask 的
/// `control_outcome_pending` 结果本就会让本轮挂起（不产生 followup），这里的判定是同一语义的
/// **兜底**：即便某个交互工具没有产生挂起标记，也绝不越过它继续执行批内后续调用。
fn sync_tool_call_ends_batch(tool_call: &ToolCall) -> bool {
    crate::agent::tools::tool_kind_for_name(&tool_call.name)
        == crate::agent::tools::ToolKind::Interactive
}

/// D3a：首个 hop 的整批调用 —— planner 修补/治理过的首个调用 + 上游响应的**余下**调用（保序）。
///
/// 首个调用必须用 planner 侧 `resolved_tool_call`（工具选择 hook 可改写、空洞调用已修补）；
/// 余下调用没有等价的 planner 通道，按上游顺序原样入批（D3a 不新增 second..N 的 planner 修补，
/// 见设计 §7 的偏差说明）。
fn sync_tool_batch(resolved_first: ToolCall, decision_calls: &[ToolCall]) -> Vec<ToolCall> {
    let mut batch = Vec::with_capacity(decision_calls.len().max(1));
    batch.push(resolved_first);
    batch.extend(decision_calls.iter().skip(1).cloned());
    batch
}
