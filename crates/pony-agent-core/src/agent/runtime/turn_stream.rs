use super::*;
// PA-171 C4：失败轮归因载体与带归因的派发旁路（`runtime/mod.rs` 的 glob 未覆盖新增符号）。
use crate::agent::turn_flow::emit_stream_failed_attributed;
// claim_failed_terminal 迁至 `turn_control`（终态闸门与其状态机同住），调用点保持不变。
use super::stream_support::flush_tail_delta_frames;
use super::turn_control::claim_failed_terminal;

/// wave-64 R2：流式决策因用户停止而中断、取消终态已在 fallback 之前派发时的控制流哨兵。
///
/// 仅在本模块内用作标记（绝不下发到宿主表面，也不进入任何落库字段）。
const STREAM_CANCELLED_HANDLED_SENTINEL: &str = "__wave64_stream_cancelled_handled__";


/// L1（dsh 对齐）：取消轮的 native transcript 片段——**只在已闭合的工具往返存在时**生成。
///
/// 取消轮没有 `assistant/message`，此前只能靠「文本检查点」把过程清单当成 assistant
/// 正文塞进下一轮上下文。dsh 的做法是「在中断处补齐调用/结果配对」，本函数即那一步：
/// 已完成的 hop 原样保留（调用与结果成对），末条用中断事实收口（带 reasoning，
/// 见 `native_transcript_for_cancelled_turn`）。无往返 ⇒ `None`，由文本检查点为
/// 非 native provider 兜底。
fn cancelled_turn_native_transcript(
    provider: &ProviderManager,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
) -> Option<Vec<Value>> {
    native_transcript_for_cancelled_turn(provider.protocol(), user_message, hop_records)
}

/// R3（dsh 对齐）：批内取消时"**已宣告但未执行/未返回**"的调用列表。
///
/// 判据用**批内序号**而不是 call_id：同一批调用可能来自上游但 `call_id` 缺失（`None`），
/// 按 id 比较会把不同调用误判成同一个。`executed_calls` 只在每次调用**完全**收尾后追加，
/// 故取消点上的当前调用序号 = `executed_calls.len()`，其后（`start_index..`）即被跳过的调用。
/// 这些调用必须以"结果未知/未开始"的 error 结果闭合进收口结构，不得静默消失（不截断在飞工作）。
fn pending_calls_after_batch_index(
    all_calls: &[ToolCall],
    start_index: usize,
) -> Vec<crate::agent::session::interrupted_turn::PendingToolCall> {
    all_calls
        .iter()
        .skip(start_index)
        .map(|call| crate::agent::session::interrupted_turn::PendingToolCall {
            name: call.name.clone(),
        })
        .collect()
}
impl AgentRuntime {
    /// D3b（P1-1 / B018）：流式工具轮驱动 —— 一次上游响应里的**整批**工具调用都在本函数内执行
    /// （与 D3a 的 `turn_sync.rs::handle_sync_tool_turn` 同构，`.dev-team/designs/multi-tool-call-execution.md` §3/§7）。
    ///
    /// 批语义：
    /// - 批内**顺序**执行，逐调用复用 `execute_registered_tool_call_with_context`（治理/沙箱/预算
    ///   路径与单调用完全一致），逐调用发工具事件、逐调用一条 [`ToolTurnHopRecord`]、逐调用计 hop
    ///   与熔断（per-call 计数语义，不因批而放宽）；
    /// - 批执行完**一次** followup 回填整批结果（`N==1` 逐字走既有 1:1 流式入口，`N>1` 走 D2 批量入口）；
    /// - 交互类工具（`ToolKind::Interactive`）是**批终止符**：执行到它即停，其后调用不执行
    ///   （Ask 的 `control_outcome_pending` 直接挂起本轮）。
    #[allow(clippy::too_many_arguments)]
    fn handle_stream_tool_turn<S: TurnEventSink>(
        &self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        input: &TurnInput,
        injection_lease: Option<&AskInjectionLease>,
        user_message: &str,
        display_message: &str,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_calls: Vec<ToolCall>,
        initial_hook_trace_records: Vec<HookTraceRecord>,
        initial_turn_first_token_latency_ms: Option<u64>,
        turn_started_at: &Instant,
        provider_call_records: &mut Vec<ProviderCallCacheRecord>,
        governed_context: Option<&DispatchContext>,
    ) {
        let context_observation = build_context_observation(planning_request, tools);
        let mut hop_records = Vec::new();
        let mut tool_activities = Vec::new();
        // D3b：本 hop 待执行的**整批**调用（批内顺序执行；批长 1 即既有单调用语义）。
        let mut current_tool_calls = tool_calls;
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
        // 同签名重复调用闸（契约矩阵 C2/C3）：同一工具 + 同一归一化参数允许**有限次**执行，
        // 第 `limit + 1` 次命中闸后走恢复式收尾。**本 turn 起始一律建空计数（`new()`）**——
        // 若按首批调用预置计数（note 最初的 `from_batch`），预置 1 次 + 执行点再记 1 次构成
        // 双重计数，默认 limit=2 时第 2 次即熔断，直接违反「前 2 次放行」契约。
        let mut same_signatures = SameSignatureTracker::new();
        let mut consecutive_failures = ConsecutiveFailureTracker::new();
        let mut accumulated_messages: Vec<Value> = Vec::new();
        // B017：本轮**已产出**的 assistant 正文累积器。每进入一次 hop 循环就把该 hop 的 assistant
        // 正文追加进来（`current_assistant_output_text` 只在新 hop 响应到达时更新，故不会重复计入）。
        // 用途：失败终态（hook 失败 / 熔断闸）把它交给 `emit_stream_failed`，避免 trace
        // `pa_01a1a134` turn-3 那种「失败轮只剩 This turn failed.」的正文丢失。
        let mut accumulated_turn_text = String::new();
        // D4（W28b / B018 后续）：批内执行段规划见 `batch_plan`；开关 `PONY_AGENT_PARALLEL_TOOLS`
        // 默认关 ⇒ 每段都是「预阶段 → 执行 → 后阶段」的单调用段，逐调用交织语义与本波之前逐字一致。
        let mut executed_calls: Vec<ToolCall> = Vec::new();
        let mut executed_results: Vec<ToolResult> = Vec::new();
        // 交互类工具（批终止符）命中标记：后阶段置位，段循环据此停批。
        let mut stream_batch_terminated: bool;

        // 预阶段（逐调用、保序）：hop 限额/计数、检查点、取消检查、工具事件、ToolCallStart hook。
        //
        // 为什么用函数内宏：本阶段与后阶段共享 20+ 个可变局部状态（hop_records / tool_activities /
        // hook_trace_records / 熔断跟踪器 / …），提取成方法要么塞一长串参数、要么引入上下文结构体，
        // 都会把「逐调用语义等价」藏进重构本身；宏展开的是**同一段代码文本**，`return` 也直接结束
        // 本函数，行为与内联逐字等价。
        macro_rules! stream_tool_call_pre {
            ($tool_call:expr) => {{
                let current_tool_call: ToolCall = $tool_call.clone();
                // hop 限额仍按**调用**计数：批内任一调用将越过上限即硬停，与批间同口径
                // （不因批而放宽；批长 1 时该闸恒被批间闸先命中，行为不变）。
                if completed_hops >= max_tool_hops_per_turn() {
                    self.fail_stream_turn_gated_dispatch(
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
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(provider.protocol(), user_message, &hop_records),
                    );
                    return;
                }
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
                // R3（L3-A #2）：**预阶段**取消——当前调用尚未执行，必须与其余未执行调用一起进
                // pending（`executed_calls.len()` 就是当前调用序号）；用 `+1` 会把这一个调用
                // 静默丢掉（既不在 tool_activities，也不在 pending）。
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
                        accumulated_turn_text.as_str(),
                        cancelled_turn_native_transcript(provider, user_message, &hop_records),
                        pending_calls_after_batch_index(&current_tool_calls, executed_calls.len()));
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
                    input.session_id.clone(),
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
                    self.fail_stream_turn_gated_dispatch(
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
                        // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(provider.protocol(), user_message, &hop_records),
                    );
                    return;
                }

            }};
        }

        // 后阶段（逐调用、保序）：记账、tool span、熔断闸、取消 / pending / aborted 判定、批终止符。
        macro_rules! stream_tool_call_post {
            ($tool_call:expr, $outcome:expr) => {{
                let current_tool_call: ToolCall = $tool_call.clone();
                let tool_outcome = $outcome;
                let tool_result = tool_outcome.result;
                let invocation_record = tool_outcome.invocation_record;
                hook_trace_records.extend(tool_outcome.capability_hook_trace_records);
                all_tools_ok &= tool_result.status == "ok";
                // Wave43 task-8 挂载切片：实时路径「工具执行结束 → 产生 TurnToolActivity」单点。
                // 一层工具执行 = 一个 tool.call span：只对顶层活动（parent_activity_id 为 None）记录，
                // 子调用活动从属顶层活动，避免 span 基数爆炸。
                let otel_activity_batch = self
                    .telemetry_builder
                    .tool_activities_after_result(&current_tool_call, &tool_result);
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
                            args_preview: serde_json::to_string(&current_tool_call.arguments).ok(),
                            result_preview: Some(tool_result.output.clone()),
                        },
                        current_tool_call.call_id.clone(),
                        otel_tool_error_kind.as_deref(),
                    );
                }
                tool_activities.extend(annotate_capability_tool_activities(
                    otel_activity_batch,
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
                        accumulated_turn_text.as_str(),
                        cancelled_turn_native_transcript(provider, user_message, &hop_records),
                        pending_calls_after_batch_index(&current_tool_calls, executed_calls.len() + 1));
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
                    self.fail_stream_turn_gated_dispatch(
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
                        // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(provider.protocol(), user_message, &hop_records),
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
                    input.session_id.clone(),
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
                    // W1b：治理路径已产出挂起标记，却匹配不到可恢复的待处理请求 ⇒
                    // 不得只发一条裸 `turn:suspended`（用户永远等不到恢复），派发可读失败终态。
                    if self.suspension_without_recoverable_request(&suspension) {
                        // F3（NFR）：日志里的 code 必须**就是**终态派发的错误码（首段），否则
                        // 「日志码 == 终态码」这条排障前提在 approval/ask 两族上失效。
                        let unrecoverable_error =
                            super::turn_control::build_unrecoverable_suspension_error(&suspension);
                        runtime_log(format!(
                            "turn:control-suspend-unrecoverable-stream hop={} name={} code={}",
                            completed_hops,
                            current_tool_call.name,
                            super::turn_control::suspension_error_code(&unrecoverable_error)
                        ));
                        self.fail_stream_turn_gated_dispatch(
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
                            unrecoverable_error,
                            // B017：本轮失败前已产出的正文必须保留。
                            Some(accumulated_turn_text.clone()),

                            native_transcript_for_failed_turn(provider.protocol(), user_message, &hop_records),
                        );
                        return;
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
                    if let Some(lease) = injection_lease {
                        self.ack_pending_ask_injection(lease, "suspended");
                    }
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

                // Tool execution errors (status "error") are fed back to the model so it can
                // decide how to respond — e.g. try a different path, use another tool, or
                // explain the problem to the user.
                // 优化：仅当明确由用户请求取消（如 control.should_cancel_turn）时才判死 turn；
                // 普通子工具超时或中断（如 timeout / interrupted）转为结构化错误反馈给模型，而非直接崩溃整个 turn。
                if tool_result.status == "aborted" {
                    if self.should_cancel_turn(control, turn_id) {
                        let error = build_tool_execution_error(
                            &current_tool_call.name,
                            tool_result.output.as_str(),
                        );
                        // R3（L3-A #6）：中止发生在批内 ⇒ 同一响应里**尚未执行**的调用
                        // 也要进失败收口（本轮工具被中止，其后调用从未开始）。
                        self.fail_stream_turn_gated_dispatch_pending(
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
                            // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                            Some(accumulated_turn_text.clone()),

                            native_transcript_for_failed_turn(provider.protocol(), user_message, &hop_records),
                            pending_calls_after_batch_index(
                                &current_tool_calls,
                                executed_calls.len() + 1,
                            ),
                        );
                        return;
                    }
                }

                // Consecutive-failure stop-loss: the same tool failing with the same error code
                // `limit` times in a row means the model is retrying a doomed invocation. Stop the
                // follow-up loop and surface the real error instead of burning the remaining
                // follow-up budget on identical retries.
                let failure_signal = tool_failure_signal(&current_tool_call, &tool_result);
                let failure_count = consecutive_failures.record(failure_signal.clone());
                if failure_count >= max_consecutive_tool_failures_per_turn() {
                    self.fail_stream_turn_gated_dispatch(
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
                                .map(|(_, _, code)| code.as_str())
                                .unwrap_or("unknown"),
                            tool_result.output.as_str(),
                        ),
                        // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(provider.protocol(), user_message, &hop_records),
                    );
                    return;
                }
                // P1-3 第二道闸：同一工具连续失败（忽略入参/错误码差异）。既有指纹闸要求三元组
                // 完全一致，模型换参数或换错误码即可绕过——实测 JobOutput 连续失败 5 次未停。
                if consecutive_failures.same_tool_count()
                    >= max_consecutive_same_tool_failures_per_turn()
                {
                    self.fail_stream_turn_gated_dispatch(
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
                        build_same_tool_consecutive_failure_error(
                            max_consecutive_same_tool_failures_per_turn(),
                            &current_tool_call.name,
                            tool_result.output.as_str(),
                        ),
                        // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(provider.protocol(), user_message, &hop_records),
                    );
                    return;
                }

                // 逐调用记录（供批后**一次**回填）：同签名计数亦按 per-call 口径在**执行点**记账
                // （闸点只读；被拦下的调用不记账，否则收尾后的重试会被自己推高计数）。
                same_signatures.record_executed(
                    tool_call_signature(&current_tool_call),
                    tool_execution_changed_world(&current_tool_call, &tool_result),
                );
                executed_calls.push(current_tool_call.clone());
                executed_results.push(tool_result.clone());
                // 批终止符：交互类工具（Ask / Permission…）执行完即停批，后续调用不执行。
                if stream_tool_call_ends_batch(&current_tool_call) {
                    runtime_log(format!(
                        "turn:tool-batch-terminator hop={} name={} executed={} skipped={}",
                        completed_hops,
                        current_tool_call.name,
                        executed_calls.len(),
                        current_tool_calls.len().saturating_sub(executed_calls.len())
                    ));
                    stream_batch_terminated = true;
                }
            }};
        }

        loop {
            accumulated_turn_text.push_str(&current_assistant_output_text);
            // ── 1) 执行本 hop 的**整批**调用（段内可并发；规划与引擎接线见 `batch_plan`）──
            executed_calls.clear();
            executed_results.clear();
            stream_batch_terminated = false;
            for segment in plan_batch_segments(&current_tool_calls) {
                // 1a) 预阶段：逐调用（保序）计 hop、发工具事件、跑 ToolCallStart hook。
                for tool_call in current_tool_calls[segment.calls.clone()].iter() {
                    stream_tool_call_pre!(tool_call);
                }
                // 1b) 执行阶段：并发段整段交 `parallel_tools` 引擎（结果保序）；其余逐调用顺序执行。
                let outcomes: Vec<ToolCallOutcome> = if segment.parallel {
                    let segment_calls: Vec<ToolCall> =
                        current_tool_calls[segment.calls.clone()].to_vec();
                    let session_id = input.session_id.as_deref();
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
                    let session_id = input.session_id.as_deref();
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
                    stream_tool_call_post!(tool_call, outcome);
                }
                if stream_batch_terminated {
                    break;
                }
            }

            // ── 2) 批后**一次** followup：N==1 逐字走既有 1:1 流式入口，N>1 走 D2 批量入口 ──
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
            // F3：客户端节流的读线程侧载体——旧实现是 `last_emit` + 内联 sleep，
            // 现在是时间窗聚合器（见 `StreamDeltaCoalescer`）：窗口内合并、窗口外上屏，
            // **读线程不再睡眠**（睡眠 = 停读 socket = TCP 背压）。
            let delta_coalescer = Rc::new(RefCell::new(StreamDeltaCoalescer::new()));
            let delta_coalescer_for_emit = Rc::clone(&delta_coalescer);
            let delta_turn_id_for_emit = delta_turn_id.clone();
            // PA-095 #4 修复：followup delta 闭包此前 session_id 传 None——事件以
            // 空 session 缓冲且永不 flush（生产事件流丢失全部 followup chunk）。
            let delta_session_id_for_emit = input.session_id.clone();
            // F2（重试窗口纯净）：同上——账本起点即 per-call 计时起点，重试/退避在
            // `provider_followup_stream_with_calls` 内部（followup 作用域还会切 20s/40s
            // 限流退避），不共源就会被算进 dur。
            let provider_call_started_at = crate::agent::retry::begin_provider_attempt_window();
            // PA-095 #4：本次 followup call 的 hop 索引（Copy 捕获进 delta 闭包）。
            let followup_step = completed_hops as u32;
            let response = match provider_followup_stream_with_calls(
                provider,
                planning_request,
                tools,
                &mut accumulated_messages,
                current_assistant_message.as_ref(),
                &executed_calls,
                &executed_results,
                move |delta| {
                    // ADR 0055 P0-2 / Task-2 修复：首包时点信号（StreamStart）不承载内容，
                    // 也**不参与** UI 首字延迟口径，但对于 provider call 本身，StreamStart 标志着首包
                    // 已经到达。即使本次 call 是纯工具调用（无 Text/Reasoning 增量），也必须在提前返回前
                    // 记录 provider_call_first_token_latency，避免 TTFT 丢失。
                    if matches!(delta, ProviderStreamChunk::StreamStart) {
                        if supports_true_streaming_followup
                            && provider_call_first_token_latency_for_emit.get().is_none()
                        {
                            let value = provider_call_started_at.elapsed().as_millis() as u64;
                            provider_call_first_token_latency_for_emit.set(Some(value));
                        }
                        return;
                    }
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

                    match delta {
                        ProviderStreamChunk::StreamStart => {
                            // ADR 0055 P0-2：首包时点信号，无内容（已在闭包入口提前返回，此臂仅为穷尽性）。
                        }
                        ProviderStreamChunk::Text(text) => {
                            if let Some(reasoning) = reasoning_batcher_for_emit.borrow_mut().flush()
                            {
                                delta_coalescer_for_emit.borrow_mut().push(
                                    None,
                                    Some(reasoning),
                                    latency,
                                );
                            }
                            delta_coalescer_for_emit
                                .borrow_mut()
                                .push(Some(text), None, latency);
                        }
                        ProviderStreamChunk::Reasoning(reasoning) => {
                            if let Some(buffered_reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().push(reasoning)
                            {
                                delta_coalescer_for_emit.borrow_mut().push(
                                    None,
                                    Some(buffered_reasoning),
                                    latency,
                                );
                            }
                        }
                    }
                    // F3（I1/I3）：这里只做一次时间戳比较；窗口内的内容留在 pending，
                    // 不 sleep、不等客户端消费。`sink.emit` 是**内联**调用——其契约是
                    // 非阻塞投递（慢 sink 会串进读循环，属已知限制，见文件头）。
                    let now = turn_started_at_for_latency.elapsed().as_millis() as u64;
                    let due = delta_coalescer_for_emit.borrow_mut().take_if_due(now);
                    if let Some(batch) = due {
                        for frame in batch.into_frames().into_iter().flatten() {
                            flush_delta(frame.text, frame.reasoning, frame.latency_ms);
                        }
                    }
                },
                crate::agent::turn_flow::LlmRequestMode::ToolFollowup,
            ) {
                Ok(response) => response,
                Err(error) => {
                    // wave-64 R2 / A2（可达性说明）：本通道（工具结果后续轮）**没有**取消观测面 ——
                    // `provider_followup_stream_with_mode` 内部使用 `ProviderStreamGuard::without_cancel()`
                    // （见 `provider/mod.rs` 的 `continue_with_tool_result_stream`），故此处永远不会
                    // 收到 `provider_stream_cancelled` 取消码，不存在"取消码 ⇒ cancelled"的分流。
                    // 用户停止在本通道的表现是：随后命中循环头/回读检查点的 `should_cancel_turn`
                    // 分支（in-band 取消），或 C5 宽限监督先行收口；两者都经 terminal-once 闸门，
                    // 确保至多一条终态。
                    self.fail_stream_turn_gated_dispatch(
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
                        // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(
                            provider.protocol(),
                            user_message,
                            &hop_records,
                        ),
                    );
                    return;
                }
            };
            if let Some(buffered_reasoning) = reasoning_batcher.borrow_mut().flush() {
                delta_coalescer
                    .borrow_mut()
                    .push(None, Some(buffered_reasoning), None);
            }
            // F3（I2）：收尾把 pending 帧**无视窗口**一次发完（允许合并、禁止丢最终帧）。
            // 实现在 `stream_support::flush_tail_delta_frames`（从本函数外移以控规模）。
            flush_tail_delta_frames(
                sink,
                &delta_coalescer,
                &delta_turn_id,
                input.session_id.clone(),
                followup_step,
                turn_started_at_for_latency.elapsed().as_millis() as u64,
            );
            let mut response = response;
            // F2：本次 followup 调用的 dur/ttft 收敛到「最后一次成功 attempt」。
            let pure_call_window = crate::agent::retry::take_pure_call_window(
                provider_call_started_at,
                provider_call_first_token_latency.get(),
            );
            let provider_call_used_true_stream =
                response.provider_source == "provider_followup_stream";
            provider_call_records.push(build_provider_call_cache_record(
                ProviderRequestKind::ToolFollowup,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                // wave-54 C4：协议归一上下文输入口径（anthropic ⇒ input + cache_read）。
                Some(provider.protocol_label()),
                response.token_usage.as_ref(),
                if provider_call_used_true_stream {
                    pure_call_window.first_token_latency_ms
                } else {
                    None
                },
                Some(pure_call_window.duration_ms),
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
            emit_stream_hop_complete(
                sink,
                turn_id,
                provider_meta,
                response.provider_source.as_str(),
                response.provider_mode.as_str(),
                accumulated_token_usage.as_ref(),
                first_token_latency.get(),
                turn_started_at.elapsed().as_millis() as u64,
                provider_call_records.clone(),
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
                    accumulated_turn_text.as_str(),
                    cancelled_turn_native_transcript(provider, user_message, &hop_records),
                    Vec::new(),
                );
                return;
            }

            if let Some(error) = provider_failure_message(
                &response.provider_mode,
                response.fallback_reason.as_deref(),
            ) {
                // PA-171 C4：工具 followup 的**模型调用**失败 ⇒ 触发方 `provider_followup_failed`
                // （`fallback_reason` 记录触发阶段；provider_source/mode 仍取本轮真实来源）。
                self.fail_stream_turn_gated_dispatch_tagged(
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
                    // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                    Some(accumulated_turn_text.clone()),
                    native_transcript_for_failed_turn(
                        provider.protocol(),
                        user_message,
                        &hop_records,
                    ),
                    "provider_followup_failed",
                );
                return;
            }

            // 本 hop 之后是否还有调用：单值字段是「还有调用」的总开关（恢复路径会把它清空），
            // 复数位给出整批（D3b 起 OpenAI 流式入口全量上报；测试替身只填单值时回落单批）。
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
                        self.fail_stream_turn_gated_dispatch(
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
                            // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                            Some(accumulated_turn_text.clone()),
                            native_transcript_for_failed_turn(
                                provider.protocol(),
                                user_message,
                                &hop_records,
                            ),
                        );
                        return;
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
                    let recovery = recover_tool_followup_completion_stream(
                        sink,
                        provider,
                        planning_request,
                        &mut accumulated_messages,
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
                    self.fail_stream_turn_gated_dispatch(
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
                        // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(
                            provider.protocol(),
                            user_message,
                            &hop_records,
                        ),
                    );
                    return;
                } else {
                    // 同签名重复闸（契约矩阵 C3）：**只读**判定（不记账、不改状态），
                    // 命中后走恢复式收尾而不是继续 follow-up。
                    let next_signature = tool_call_signature(&next_tool_call);
                    let same_signature_limit = max_same_signature_occurrences_per_turn();
                    if same_signatures.is_exhausted(&next_signature, same_signature_limit) {
                        let occurrences = same_signatures.occurrence_count(&next_signature) + 1;
                        let recovery = recover_tool_followup_completion_stream(
                            sink,
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
                        // D3b：下一 hop 的整批（保序；首元素已过 normalize_tool_directive）。
                        current_tool_calls = next_tool_calls;
                        continue;
                    }
                }
            }

            let completed_text = response.output_text.clone();
            let completed_mode = response.provider_mode.clone();
            let attachments = match self.save_input_attachments(input) {
                Ok(attachments) => attachments,
                Err(error) => {
                    self.fail_stream_turn_gated_dispatch(
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
                        // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                        Some(accumulated_turn_text.clone()),
                        native_transcript_for_failed_turn(
                            provider.protocol(),
                            user_message,
                            &hop_records,
                        ),
                    );
                    return;
                }
            };
            let persisted = self.persist_turn_outcome(
                input.session_id.as_deref(),
                // A1：成功轮同样写轮次身份（gate 历史腿对任意轮一致成立）。
                Some(turn_id),
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
                None,
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
                // Wave-15 R1 扩展：末次调用的 per-call 记录由 turn:output_end 承载
                // （此时 provider_call_records 已含本次调用；累加字段语义不变）。
                Some(provider_call_records.clone()),
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
                self.fail_stream_turn_gated_dispatch(
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
                    // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                    Some(accumulated_turn_text.clone()),
                    native_transcript_for_failed_turn(
                        provider.protocol(),
                        user_message,
                        &hop_records,
                    ),
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
                self.fail_stream_turn_gated_dispatch(
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
                    // B017：失败前已产出的正文（熔断 / hook 失败主路径必须保留）。
                    Some(accumulated_turn_text.clone()),
                    native_transcript_for_failed_turn(
                        provider.protocol(),
                        user_message,
                        &hop_records,
                    ),
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

            let completed_timeline = build_persisted_trace_timeline_with_provider_calls(
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
                // Wave-15 R2：per-call 记录按调用序号写入各 call_model 条目。
                provider_call_records.as_slice(),
            );
            if let Some(lease) = injection_lease {
                self.ack_pending_ask_injection(lease, "completed");
            }
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
        // Wave43 task-8 挂载切片：turn root span（agent.turn）在流式轮唯一顶层入口创建，
        // 函数返回时 Drop 自动结束 span，覆盖整轮生命周期（含同步执行的工具子轮）。
        // ADR 0055 P0-3：序号取「会话已登记轮次数」（0-based），不再硬编码 0。
        let turn_sequence = self.session_turn_sequence(input.session_id.as_deref());
        let turn_started_at = Instant::now(); // P5：与 `agent.turn` span 同点起算（旧实现实测差 35.4s）
        // W3 trace：队友/子代理 runtime 继承宿主 traceparent（宿主 runtime 为 `None` ⇒ root）。
        let mut otel_turn_scope = crate::agent::otel::begin_turn_trace_with_parent(
            input.session_id.as_deref().unwrap_or(""),
            &turn_id,
            turn_sequence,
            self.parent_traceparent.as_deref(),
        );
        // T2-1：流式 turn 的 session/run/turn 事实在本 turn 内部解析，并随每次工具调用
        // 显式下传（Ask 控制请求因此仍绑定真实调用；并发 turn 之间不再共享任何槽位）。
        let mut governed_context = self.resolve_governed_turn_context(&input, &facts);
        // wave-62 C2/C3：把本 turn 的取消令牌注入治理事实（registry 是唯一创建点）。
        // 之后 dispatch 闸门复用同一令牌，工具层（run_command）也据此真杀子进程。
        if let Some(context) = governed_context.as_mut() {
            context.turn_cancellation = control.cancellation_token(&turn_id);
        }
        // The one-shot Ask resume injection is consumed **after** the governance gate below
        // (ADR 0074/T8): a governance-blocked turn must not swallow the answer.
        let injection_run_id = facts.run_id.clone();
        let injection_session_id = input.session_id.clone();
        // Wave-7 B003：**turn 启动前**的用量治理判定（一次性；只在入口，绝不中断进行中的 turn）。
        // fail-open：账本不可读/策略非法 ⇒ Allow + governance_degraded（G3），不会进这里的分支。
        {
            let governance = {
                let store = self.sessions.read().unwrap_or_else(|error| {
                    tracing::warn!(target: "control_plane", error = %error, "[pony-agent] sessions rwlock poisoned");
                    error.into_inner()
                });
                crate::agent::usage_governance::evaluate_with_store(
                    &store,
                    input.session_id.as_deref(),
                )
            };
            if let crate::agent::usage_governance::Decision::Block {
                reason,
                current,
                limit,
                ..
            } = &governance.decision
            {
                // L3 B3：阻断属**预期内用户状态**，标记后跳过 issue 上报（仍会发 turn:failed 事件）。
                // wave-64 R1（BLOCKING，terminal-once）：仅抢到终态派发权者才继续。
                if !claim_failed_terminal(control, &turn_id) {
                    return;
                }
                crate::agent::ponysentry::mark_turn_failure_expected(&turn_id);
                let error = format!(
                    "用量治理已阻断本轮：当前已用 ${current:.6}，上限 ${limit:.6}（触发 {reason}）。\
                     如需继续，请提高上限或关闭「强制拦截」——设置 → 用量治理 → 关闭 enforcement（一键出口）。"
                );
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
                otel_turn_scope.set_phase("failed");
                return;
            }
        }

        // Contract §5: peek (do not consume) — ack happens only on a successful terminal state.
        let injection_lease = self.peek_pending_ask_injection(
            injection_run_id.as_deref(),
            injection_session_id.as_deref(),
        );
        let ask_injection = injection_lease
            .as_ref()
            .map(|lease| lease.injection.clone());
        let prepared = match self.prepare_turn(&input, true, ask_injection.as_ref()) {
            Ok(prepared) => prepared,
            Err(error) => {
                // wave-64 R1（BLOCKING，terminal-once）：`prepare_turn` 内含 auto-compaction 等
                // 网络调用（耗时可能超过 C5 宽限期），期间宽限监督可先行把该轮收口为 cancelled；
                // 此时必须静默退出（不 persist、不 emit），禁止第二条 turn:failed 覆盖已落盘终态。
                if !claim_failed_terminal(control, &turn_id) {
                    return;
                }
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
                otel_turn_scope.set_phase("failed");
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
        self.mark_stream_calling_model_checkpoint(
            control,
            &turn_id,
            &prepared_provider_meta,
            &start_trace_steps,
        );

        emit_stream_turn_started(
            sink,
            turn_id.clone(),
            prepared.user_message.clone(),
            &prepared_provider_meta,
            prepared.build_context_observation.clone(),
            start_trace_steps.clone(),
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
                "",
                // 决策阶段之前取消：本轮没有已闭合的工具往返。
                None,
                Vec::new(),
            );
            otel_turn_scope.set_phase("cancelled");
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
            // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
            // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
            if !claim_failed_terminal(control, &turn_id) {
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
            otel_turn_scope.set_phase("failed");
            return;
        }
        if let Some(error) = preflight_dispatch.blocked_error.take() {
            // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
            // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
            if !claim_failed_terminal(control, &turn_id) {
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
            otel_turn_scope.set_phase("failed");
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
        // 用户中止时的上下文保全：真流式决策期间**已渲染给用户**的正文 delta 在此累积。
        // 取消收口（`cancel_stream_turn`）把它落成中断检查点正文；否则被中断那轮已经出现在
        // 界面上的内容不会进入下一轮会话历史（只剩占位文案）。
        let streamed_assistant_text = Rc::new(RefCell::new(String::new()));
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
            // F2（重试窗口纯净）：账本起点与 per-call 计时**同源**。重试/退避发生在
            // `provider_decision_stream_guarded_with_mode` 内部
            // （`provider::retry_provider_scoped_with_report_config`），因此取样点必须
            // 就是本次调用的起点，返回后再由账本把窗口收敛到「最后一次成功 attempt」。
            let initial_decision_started_at = crate::agent::retry::begin_provider_attempt_window();
            let initial_turn_first_token_latency = Rc::new(Cell::new(None));
            let initial_call_first_token_latency = Rc::new(Cell::new(None));
            let initial_turn_first_token_latency_for_emit =
                Rc::clone(&initial_turn_first_token_latency);
            let initial_call_first_token_latency_for_emit =
                Rc::clone(&initial_call_first_token_latency);
            let reasoning_batcher = Rc::new(RefCell::new(StreamReasoningBatcher::default()));
            let reasoning_batcher_for_emit = Rc::clone(&reasoning_batcher);
            // F3：读线程侧的时间窗聚合器（替代旧的 `last_emit` + 内联 sleep 节流）。
            let delta_coalescer = Rc::new(RefCell::new(StreamDeltaCoalescer::new()));
            let delta_coalescer_for_emit = Rc::clone(&delta_coalescer);
            let turn_id_for_delta_emit = turn_id_for_stream.clone();
            let stream_session_id_for_delta_emit = stream_session_id.clone();

            // T9（W56-A）：把既有取消源（ExecutionControlRegistry + 本 turn id）接进 provider 帧循环；
            // 总时长上界用缺省 30min（可注入 seam 见 ProviderStreamGuard::new）。
            let stream_turn_id_for_guard = turn_id.clone();
            let stream_cancel_check = || control.is_stop_requested(stream_turn_id_for_guard.as_str());
            let stream_guard = crate::agent::provider::ProviderStreamGuard::with_cancel_check(
                &stream_cancel_check,
            );
            let streamed_assistant_text_for_delta = Rc::clone(&streamed_assistant_text);
            let decision = provider_decision_stream_guarded_with_mode(
                &prepared.provider,
                &prepared.planning_request,
                &prepared.tools,
                &stream_guard,
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
                    // ADR 0055 P0-2：首包时点信号无内容，不参与 UI 首字延迟口径（同上）。
                    // 但对于首轮初始请求若是纯工具调用，StreamStart 标志着首包到达，需记录 initial_call_first_token_latency。
                    if matches!(delta, ProviderStreamChunk::StreamStart) {
                        if initial_call_first_token_latency_for_emit.get().is_none() {
                            let value = initial_decision_started_at.elapsed().as_millis() as u64;
                            initial_call_first_token_latency_for_emit.set(Some(value));
                        }
                        return;
                    }
                    if initial_call_first_token_latency_for_emit.get().is_none() {
                        let value = initial_decision_started_at.elapsed().as_millis() as u64;
                        initial_call_first_token_latency_for_emit.set(Some(value));
                    }
                    match delta {
                        ProviderStreamChunk::StreamStart => {
                            // ADR 0055 P0-2：首包时点信号，无内容（已在闭包入口提前返回，此臂仅为穷尽性）。
                        }
                        ProviderStreamChunk::Text(text) => {
                            if let Some(reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().flush()
                            {
                                delta_coalescer_for_emit.borrow_mut().push(
                                    None,
                                    Some(reasoning),
                                    None,
                                );
                            }
                            streamed_assistant_text_for_delta
                                .borrow_mut()
                                .push_str(&text);
                            delta_coalescer_for_emit
                                .borrow_mut()
                                .push(Some(text), None, None);
                        }
                        ProviderStreamChunk::Reasoning(reasoning) => {
                            if let Some(buffered_reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().push(reasoning)
                            {
                                delta_coalescer_for_emit.borrow_mut().push(
                                    None,
                                    Some(buffered_reasoning),
                                    None,
                                );
                            }
                        }
                    }
                    // F3（I1/I3）：只做一次时间戳比较——窗口内的内容留在 pending，
                    // **不 sleep、不等客户端消费**。`sink.emit` 内联且契约=非阻塞投递
                    // （慢 sink 会串进读循环，属已知限制，见文件头）。
                    let now = turn_started_at.elapsed().as_millis() as u64;
                    let due = delta_coalescer_for_emit.borrow_mut().take_if_due(now);
                    if let Some(batch) = due {
                        for frame in batch.into_frames().into_iter().flatten() {
                            flush_delta(frame.text, frame.reasoning);
                        }
                    }
                },
                crate::agent::turn_flow::LlmRequestMode::Initial,
            )?;
            if let Some(buffered_reasoning) = reasoning_batcher.borrow_mut().flush() {
                delta_coalescer
                    .borrow_mut()
                    .push(None, Some(buffered_reasoning), None);
            }
            // F3（I2）：收尾**无条件**把 pending 发完——允许合并，禁止丢最终帧。
            let tail_now = turn_started_at.elapsed().as_millis() as u64;
            let tail_batch = delta_coalescer.borrow_mut().flush(tail_now);
            if let Some(batch) = tail_batch {
                for frame in batch.into_frames().into_iter().flatten() {
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
                        frame.text,
                        frame.reasoning,
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
                }
            }
            // F2：per-call (dur, ttft) 收敛为「只覆盖最后一次成功 attempt」的纯净窗口；
            // 失败尝试/退避只留痕在账本里，不进 `gen`。轮级 TTFT 口径不变（轮级样本，
            // 不受 per-call 纯净性约束）。
            let pure_call_window = crate::agent::retry::take_pure_call_window(
                initial_decision_started_at,
                initial_call_first_token_latency.get(),
            );
            Ok((
                decision,
                Some(pure_call_window.duration_ms),
                initial_turn_first_token_latency.get(),
                pure_call_window.first_token_latency_ms,
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
            // F2：同步（buffered）决策的窗口同样必须纯净——无 ttft 的调用会落到
            // `end_to_end` 口径（`Σout/Σdur`），跨 attempt 的 dur 会把失败尝试
            // （429/525 1.9–8.0s + 退避）当成生成时间，速度被系统性压低。
            let started_at = crate::agent::retry::begin_provider_attempt_window();
            let decision = provider_decision_with_mode(
                &prepared.provider,
                &prepared.planning_request,
                &prepared.tools,
                crate::agent::turn_flow::LlmRequestMode::Initial,
            )?;
            let pure_call_window =
                crate::agent::retry::take_pure_call_window(started_at, None);
            Ok((
                decision,
                Some(pure_call_window.duration_ms),
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
                    stream_initial_decision().or_else(|error| {
                        // wave-64 R2：用户已请求停止 ⇒ **不得**再降级为同步决策（那会把取消
                        // 吞成不可中断的阻塞调用，或让后续错误被误判为真实失败）。直接按
                        // 取消收口并返回（取消码本身不参与本判定，故非取消错误的 fallback
                        // 语义逐字不变 —— 契约 §2 R2 / C4）。
                        if self.should_cancel_turn(control, &turn_id) {
                            // wave-64 A4：取消收口前留一条**结构化**日志（含 turn_id 与原始 error），
                            // 避免"有停止位即取消"把根因彻底丢弃（不得新增 eprintln 面）。
                            super::turn_control::log_stop_short_circuit(
                                &turn_id,
                                &error,
                                "native_tool_flow",
                            );
                            self.cancel_stream_turn_for_stop(
                                sink,
                                control,
                                &turn_id,
                                &input,
                                &prepared.display_message,
                                &prepared_provider_meta,
                                self.telemetry_builder.failed_trace_before_tool(),
                                Vec::new(),
                                None,
                                &turn_started_at,
                                Some(prepared.build_context_observation.clone()),
                                streamed_assistant_text.borrow().as_str(),
                                Vec::new());
                            return Err(STREAM_CANCELLED_HANDLED_SENTINEL.to_string());
                        }
                        // 仅诊断：不改变控制流（失败仍然 fallback 到同步决策），
                        // 只是不再像过去的 `|_|` 那样把根因吞掉。Windows CI 上真实
                        // 原因（连接级 `client error (SendRequest)`）曾被完全掩盖，
                        // 只在下游留下 "expected value at line 1 column 1"。
                        eprintln!(
                            "[pony-agent] 流式决策失败，fallback 到同步决策 \
                             (scope=native_tool_flow): session={} turn={turn_id_for_stream} \
                             error={error}",
                            stream_session_id.as_deref().unwrap_or("-"),
                        );
                        decide_sync()
                    })
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
                            stream_initial_decision().or_else(|error| {
                                // wave-64 R2：同 native_tool_flow 分支 —— 用户已请求停止时
                                // 不再 fallback，直接按取消收口（见上方判据说明）。
                                if self.should_cancel_turn(control, &turn_id) {
                                    // wave-64 A4：同 native_tool_flow 分支的结构化留痕。
                                    super::turn_control::log_stop_short_circuit(
                                        &turn_id,
                                        &error,
                                        "no_preflight_decision",
                                    );
                                    self.cancel_stream_turn_for_stop(
                                        sink,
                                        control,
                                        &turn_id,
                                        &input,
                                        &prepared.display_message,
                                        &prepared_provider_meta,
                                        self.telemetry_builder.failed_trace_before_tool(),
                                        Vec::new(),
                                        None,
                                        &turn_started_at,
                                        Some(prepared.build_context_observation.clone()),
                                        streamed_assistant_text.borrow().as_str(),
                                        Vec::new());
                                    return Err(STREAM_CANCELLED_HANDLED_SENTINEL.to_string());
                                }
                                // 同 1855 处：仅记录根因，控制流不变（失败即 fallback）。
                                eprintln!(
                                    "[pony-agent] 流式决策失败，fallback 到同步决策 \
                                     (scope=no_preflight_decision): session={} turn={turn_id_for_stream} \
                                     error={error}",
                                    stream_session_id.as_deref().unwrap_or("-"),
                                );
                                decide_sync()
                            })
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
                if error == STREAM_CANCELLED_HANDLED_SENTINEL {
                    // wave-64 R2：取消终态已在 fallback 之前派发（判据见上方 `or_else`），
                    // 这里只落相位，绝不重复派发终态、也不派生 turn:failed。
                    otel_turn_scope.set_phase("cancelled");
                    return;
                }
                if error == HOOK_FAILTURN_HANDLED_SENTINEL {
                    if let Some((hook_trace_records, hook_error)) =
                        pending_model_call_fail_turn.borrow_mut().take()
                    {
                        let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                        self.fail_stream_turn_gated_dispatch(
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
                            // B017：本函数（首轮决策 / 收尾 hook）作用域内没有「已渲染正文」累积器，传 None 走既有占位文案。
                            None,
                            // 本调用点在工具阶段之前：本轮没有可恢复的工具往返。
                            None,
                        );
                    }
                    otel_turn_scope.set_phase("failed");
                    return;
                }
                // wave-64 R2：provider 读通道以取消码 `provider_stream_cancelled` 结束且
                // 该轮已有用户停止请求 ⇒ 归 `turn:cancelled`（checkpoint 终态 cancelled），
                // **禁止**派生 `turn:failed`；其余错误逐字走下方既有 failed 路径。
                if super::turn_control::classify_stream_error(
                    self.should_cancel_turn(control, &turn_id),
                    &error,
                ) == super::turn_control::StreamErrorClass::Cancelled
                {
                    self.cancel_stream_turn_for_stop(
                        sink,
                        control,
                        &turn_id,
                        &input,
                        &prepared.display_message,
                        &prepared_provider_meta,
                        self.telemetry_builder.failed_trace_before_tool(),
                        Vec::new(),
                        None,
                        &turn_started_at,
                        Some(prepared.build_context_observation.clone()),
                        streamed_assistant_text.borrow().as_str(),
                        Vec::new(),
                    );
                    otel_turn_scope.set_phase("cancelled");
                    return;
                }
                // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
                // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
                if !claim_failed_terminal(control, &turn_id) {
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
                otel_turn_scope.set_phase("failed");
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
                    // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
                    // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
                    if !claim_failed_terminal(control, &turn_id) {
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
                    otel_turn_scope.set_phase("failed");
                    return;
                }
            };
            first_decision.tool_call = normalized.tool_call;
            first_decision.assistant_message = normalized.assistant_message;
            if first_decision.tool_call.is_none()
                && first_decision.output_text.trim().is_empty()
                && !normalized.output_text.trim().is_empty()
            {
                first_decision.output_text = normalized.output_text;
            }
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
            // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
            // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
            if !claim_failed_terminal(control, &turn_id) {
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
            otel_turn_scope.set_phase("failed");
            return;
        }
        if let Some(error) = tool_selection_dispatch.blocked_error.take() {
            // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
            // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
            if !claim_failed_terminal(control, &turn_id) {
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
            otel_turn_scope.set_phase("failed");
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
            // wave-54 C4：协议归一上下文输入口径（anthropic ⇒ input + cache_read）。
            Some(provider.protocol_label()),
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
            // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
            // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
            if !claim_failed_terminal(control, &turn_id) {
                return;
            }
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
            // PA-171 C4：首跳 decision 失败 ⇒ 触发方 `provider_decision_failed`；
            // 归因三字段落真实值（provider_source 含真实 provider 名），并附 `error_class`。
            let decision_failure_attribution = crate::agent::turn_flow::FailedTurnAttribution {
                fallback_reason: Some("provider_decision_failed".to_string()),
                ..crate::agent::turn_flow::FailedTurnAttribution::from_failure(
                    Some(&provider_meta),
                    &error,
                )
            };
            emit_stream_failed_attributed(
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
                // P0-3：工具选择分派即失败，`stream_text_chunks` 尚未调用 ⇒ 没有任何已渲染正文。
                None,
                decision_failure_attribution,
            );
            otel_turn_scope.set_phase("failed");
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
                first_decision.output_text.as_str(),
                // 工具分派失败：没有任何已闭合的工具往返。
                None,
                Vec::new(),
            );
            otel_turn_scope.set_phase("cancelled");
            return;
        }

        if let Some(tool_call) = resolved_tool_call {
            // D3b：首个 hop 的整批调用 = planner 修补后的首个调用 + 上游响应的余下调用（保序）。
            let tool_calls = stream_tool_batch(tool_call, &first_decision.tool_calls);
            self.handle_stream_tool_turn(
                sink,
                control,
                &turn_id,
                &input,
                injection_lease.as_ref(),
                &user_message,
                &display_message,
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                &first_decision,
                tool_calls,
                planner_hook_trace_records.clone(),
                initial_turn_first_token_latency_ms,
                &turn_started_at,
                &mut provider_call_records,
                governed_context.as_ref(),
            );
            // 工具子轮的成功/失败/取消都在 handle_stream_tool_turn 内部发射终态，外层拿不到
            // 返回值；这里按终态 checkpoint.status 回读相位（不改 helper 签名）。
            let otel_delegated_status = control
                .load_checkpoint(Some(turn_id.as_str()), input.session_id.as_deref())
                .map(|checkpoint| checkpoint.status)
                .unwrap_or_default();
            otel_turn_scope.set_phase(match otel_delegated_status.as_str() {
                "completed" => "completed",
                "cancelled" | "aborted" => "cancelled",
                "failed" | "error" => "failed",
                _ => "",
            });
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
                first_decision.output_text.as_str(),
                // 无工具返回分支取消：本轮没有已闭合的工具往返。
                None,
                Vec::new(),
            );
            // wave-54 C2：取消轮同样落已发生调用的加权速度真值（records 为空 ⇒ 仅 calls=0）。
            crate::agent::turn_flow::record_turn_speed_truth(
                &mut otel_turn_scope,
                &provider_call_records,
                Some(turn_started_at.elapsed().as_millis() as u64),
            );
            otel_turn_scope.set_phase("cancelled");
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
                        input.session_id.as_deref(),
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
                input.session_id.as_deref(),
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
                // wave-64 R1（BLOCKING，terminal-once）：已有终态派发者（如 C5 宽限监督已收口
                // cancelled）⇒ 禁止二次派发 turn:failed、禁止覆盖已落盘的终态。
                if !claim_failed_terminal(control, &turn_id) {
                    return;
                }
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
                // PA-171 C4：附件保存失败（首跳回复已渲染）走 14 参冻结入口，
                // 归因由 `FailedTurnAttribution::from_failure` 派生（三字段非 None + error_class）。
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
                    // P0-3：首轮回复正文已由 `stream_text_chunks` 渲染出去（`completed_text`），
                    // 失败（保存附件）后仍须保留它，避免失败轮只剩占位文案。
                    Some(completed_text.clone()),
                );
                otel_turn_scope.set_phase("failed");
                return;
            }
        };
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            // A1：成功轮同样写轮次身份（gate 历史腿对任意轮一致成立）。
            Some(turn_id.as_str()),
            &display_message,
            &completed_text,
            provider.name(),
            &completed_mode,
            first_decision.token_usage.as_ref(),
            native_transcript_for_completed_turn(
                &user_message,
                &first_decision,
                provider.persists_native_tool_transcript(),
            ),
            attachments,
            input.workspace_mode.as_deref(),
            None,
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
            // Wave-15 R1 扩展：单调用轮次输出结束即携带本次调用 per-call 记录。
            Some(provider_call_records.clone()),
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
            self.fail_stream_turn_gated_dispatch(
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
                // B017：本函数（首轮决策 / 收尾 hook）作用域内没有「已渲染正文」累积器，传 None 走既有占位文案。
                None,
                // 本调用点在工具阶段之前：本轮没有可恢复的工具往返。
                None,
            );
            otel_turn_scope.set_phase("failed");
            return;
        }
        let finalize_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::TurnFinalizeEnd);
        let finalize_hook_trace_records = finalize_hook_outcome.trace_records.clone();
        let mut terminal_hook_trace_records = planner_hook_trace_records.clone();
        terminal_hook_trace_records.extend(checkpoint_hook_trace_records.clone());
        terminal_hook_trace_records.extend(finalize_hook_trace_records.clone());
        if let Some(error) = finalize_hook_outcome.fail_turn_error {
            self.fail_stream_turn_gated_dispatch(
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
                // B017：本函数（首轮决策 / 收尾 hook）作用域内没有「已渲染正文」累积器，传 None 走既有占位文案。
                None,
                // 本调用点在工具阶段之前：本轮没有可恢复的工具往返。
                None,
            );
            otel_turn_scope.set_phase("failed");
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

        let completed_timeline = build_persisted_trace_timeline_with_provider_calls(
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
            // Wave-15 R2：单调用轮次的 per-call 记录同样按序号写入。
            &provider_call_records,
        );
        // wave-54 C2：根 span 速度/耗时真值（生产路径唯一出口，数据源 = 本轮 per-call records）。
        crate::agent::turn_flow::record_turn_speed_truth(
            &mut otel_turn_scope,
            &provider_call_records,
            turn_duration_ms,
        );
        if let Some(lease) = injection_lease.as_ref() {
            self.ack_pending_ask_injection(lease, "completed");
        }
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
        otel_turn_scope.set_phase("completed");
    }
}

/// D3b（P1-1 / B018）：批终止符判定 —— **交互类**工具执行后立即结束本批。
///
/// 与 D3a 的 `turn_sync.rs::sync_tool_call_ends_batch` 同一判定真源（工具注册表的 `ToolKind`，
/// 纯函数、名字先 canonicalize 再查表），不另设名字白名单——新增交互工具只要登记 kind 即自动
/// 获得批终止语义。设计 §3.1：交互类工具等待宿主应答期间若继续执行批内其余调用，会出现
/// 「已经停顿却仍在改工作区」的不可解释状态；即便某交互工具未产生挂起标记，也绝不越过它继续执行。
fn stream_tool_call_ends_batch(tool_call: &ToolCall) -> bool {
    crate::agent::tools::tool_kind_for_name(&tool_call.name)
        == crate::agent::tools::ToolKind::Interactive
}

/// D3b：首个 hop 的整批调用 —— planner 修补/治理过的首个调用 + 上游响应的**余下**调用（保序）。
///
/// 与 D3a 的 `turn_sync.rs::sync_tool_batch` 同构（各持一份以免改动已交付的同步路径）：
/// 首个调用必须用 planner 侧 `resolved_tool_call`（工具选择 hook 可改写、空洞调用已修补）；
/// 余下调用没有等价的 planner 通道，按上游顺序原样入批（与 D3a 同口径的已知偏差）。
fn stream_tool_batch(resolved_first: ToolCall, decision_calls: &[ToolCall]) -> Vec<ToolCall> {
    let mut batch = Vec::with_capacity(decision_calls.len().max(1));
    batch.push(resolved_first);
    batch.extend(decision_calls.iter().skip(1).cloned());
    batch
}
