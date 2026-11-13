// turn_persist: 轮次结果落库、附件保存、失败 trace 构建与 checkpoint 更新。
// 由 runtime/mod.rs 的 AgentRuntime 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;
// PA-171 C4：失败轮归因（`runtime/mod.rs` 的 glob 未覆盖新增旁路，故在此显式引入）。
use crate::agent::turn_flow::{emit_stream_failed_attributed, FailedTurnAttribution};

/// Wave-7 B001/B003：构造一条账本记录（纯函数，可单测）。
///
/// 语义（契约 G1.3 + L3 B2 修复）：
/// - **usage 缺失**（`input_tokens` 或 `output_tokens` 为 `None`，如失败/取消/部分 provider 不回报）
///   ⇒ `LedgerEntry::unpriced`（`cost_usd = NULL` / `priced = 0`），**绝不用 0 冒充已知成本**；
/// - 模型未命中价格表 ⇒ 同样 `unpriced`；
/// - 只有「真实 usage + 命中价格表」才写 `priced = 1`。
pub(crate) fn build_usage_ledger_entry(
    session_id: &str,
    turn_id: &str,
    meta: &ProviderEventMeta,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
    recorded_at_ms: i64,
) -> crate::agent::usage_ledger::LedgerEntry {
    let protocol = crate::agent::cost::ProviderCostProtocol::from_label(&meta.protocol);
    // usage 必须是**真实上报**的：任一 token 字段缺失即视为未计价（不猜、不补 0）。
    let cost = match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => crate::agent::cost::estimate_cost_with_protocol(
            &meta.model,
            input,
            output,
            cache_hit_input_tokens,
            protocol,
        )
        .map(|estimate| estimate.total_cost_usd),
        _ => None,
    };
    let input = input_tokens.unwrap_or(0);
    let output = output_tokens.unwrap_or(0);
    let cache_read = cache_hit_input_tokens.unwrap_or(0);
    match cost {
        Some(cost) => crate::agent::usage_ledger::LedgerEntry::priced(
            session_id,
            turn_id,
            recorded_at_ms,
            &meta.provider_name,
            &meta.model,
            &meta.protocol,
            input,
            output,
            cache_read,
            cost,
        ),
        None => crate::agent::usage_ledger::LedgerEntry::unpriced(
            session_id,
            turn_id,
            recorded_at_ms,
            &meta.provider_name,
            &meta.model,
            &meta.protocol,
            input,
            output,
            cache_read,
        ),
    }
}

/// Wave-5 K2 / B027 C1+C3：持久化 trace 的成本（未知模型 / 缺真实 usage / 无 meta ⇒ `None`）。
///
/// 只基于 provider 真实返回的 usage 计算，禁止本地 token 猜测（契约 K1.3）。
/// B027：改为**委托**单一出口 [`crate::agent::turn_flow::estimate_call_cost_usd`]，
/// 从而按 `provider_meta.protocol` 归一（Anthropic 的 input 不含 cache，不再被减法清空），
/// 并与 wire 侧逐位同值；禁再用固定 OpenAI 口径的 `cost::estimate_cost`。
fn estimate_trace_cost(
    provider_meta: Option<&ProviderEventMeta>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
) -> Option<f64> {
    crate::agent::turn_flow::estimate_call_cost_usd(
        provider_meta,
        input_tokens,
        output_tokens,
        cache_hit_input_tokens,
    )
}

impl AgentRuntime {
    pub(crate) fn persist_turn_outcome(
        &self,
        session_id: Option<&str>,
        // A1：本轮 `turn_id` —— 写进 user/assistant 两条历史消息的轮次身份，使
        // `turn_outcome_persisted` 能以历史作为唯一持久化真值源（watchdog 只写 trace）。
        turn_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_name: &str,
        provider_mode: &str,
        token_usage: Option<&TokenUsage>,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
        workspace_mode: Option<&str>,
        // L2：本轮的 assistant 行是否为运行时合成的中断记录（取消/失败）。
        // 成功轮传 `None`；它坐实「载体种类」，展示层不再嗅探文本。
        assistant_kind: Option<HistoryMessageKind>,
    ) -> PersistedTurnOutcome {
        // PA-095 #2：物化前置——在获取 sessions 写锁之前请求提交本会话缓冲事件
        // （flush 闭包需要 sessions 写锁；持锁调用会自锁死锁）。提交后 append_turn
        // 内部的 flush 为空操作（缓冲已清），物化从事件表读取本 turn 消息。
        let flush_session_key = session_id.unwrap_or(crate::agent::session::DEFAULT_SESSION_ID);
        if let Err(error) =
            crate::agent::turn_flow::flush_session_buffered_events(flush_session_key)
        {
            eprintln!(
                "[pony-agent][runtime] pre-materialization event flush failed: {error}"
            );
        }
        let updated_session = self
            .sessions
            .write()
            .unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            })
            .append_turn_with_kind(
                session_id,
                user_message,
                assistant_message,
                provider_native_transcript,
                attachments,
                assistant_kind,
                turn_id,
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
        // PA-171 C4-S3：失败轮落盘同样带归类（唯一来源 `retry::classify_error_class`）。
        let error_class = error
            .as_deref()
            .map(|text| crate::agent::retry::classify_error_class(text).to_string());
        let trace_timeline = build_persisted_trace_timeline_with_provider_calls(
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
            // Wave-15 R2：每条 call_model 携带自身 per-call 指标（按调用序号）。
            &provider_call_records,
        );
        // 内存更新同步（快），SQLite 落库通过后台队列异步执行，
        // 避免 trace（可观测性）写盘阻塞主对话执行路径。
        // C1：健康度字段取自 OTel 同一计数器（与根 span 的 tool_failures/degraded 同源同口径）。
        let (degraded, tool_failures) = Self::session_trace_health_fields(phase);
        let (session_key, mutation) = self
            .sessions
            .write()
            .unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            })
            .record_turn_trace_in_memory(
                session_id,
                TurnTraceRecord {
                    turn_id: turn_id.to_string(),
                    session_id: session_id.map(str::to_string),
                    event_id: None,
                    // Wave-4.1 A6：导出态 trace_id（门禁关闭/未采样/本地降级时一律 None）——
                    // 由当前 turn 的 otel 守卫在本调用栈上下文中登记，这里按同一上下文读取。
                    trace_id: crate::agent::otel::exported_trace_id(),
                    // Wave-5 K2：会话 trace 成本（未知模型 / 缺 usage ⇒ None，键缺席）。
                    cost_usd: estimate_trace_cost(
                        provider_meta,
                        input_tokens,
                        output_tokens,
                        cache_hit_input_tokens,
                    ),
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
                    build_context_observation_ref: None,
                    session_summary,
                    fallback_reason,
                    error,
                    error_class,
                    input_tokens,
                    cache_hit_input_tokens,
                    reasoning_tokens,
                    output_tokens,
                    total_tokens,
                    first_token_latency_ms,
                    turn_duration_ms,
                    // wave-54 C3/C4：来源/上下文口径由 store 侧按 per-call records 派生。
                    usage_origin: None,
                    context_input_tokens: None,
                    input_text: Some(user_message.to_string()),
                    output_text: return_text.as_deref().map(str::to_string),
                    degraded,
                    tool_failures,
                    updated_at: 0,
                },
            );
        self.enqueue_trace_persistence(session_key, mutation);

        // Wave-7 B001/B003：本轮用量写入 `usage_ledger`（幂等 UPSERT；协议感知计价）。
        // 失败只 warn + spool（append_entry_resilient 内部处理），**绝不阻塞 turn 返回**。
        self.append_usage_ledger(
            session_id,
            turn_id,
            provider_meta,
            input_tokens,
            output_tokens,
            cache_hit_input_tokens,
        );
    }

    /// Wave-7 B003：把本轮用量写入账本（未知模型 ⇒ `cost_usd = NULL` / `priced = 0`）。
    ///
    /// 计价口径与 K2/K4 完全同源：`cost::estimate_cost_with_protocol`（协议语义归一）。
    fn append_usage_ledger(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        provider_meta: Option<&ProviderEventMeta>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cache_hit_input_tokens: Option<u64>,
    ) {
        let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) else {
            return; // 无会话归属 ⇒ 不入账（避免污染统计）
        };
        let Some(meta) = provider_meta else {
            return; // 无 provider 元数据（未走到模型调用）⇒ 不入账
        };
        let entry = build_usage_ledger_entry(
            session_id,
            turn_id,
            meta,
            input_tokens,
            output_tokens,
            cache_hit_input_tokens,
            crate::agent::usage_ledger::now_epoch_ms(),
        );
        let store = self.sessions.read().unwrap_or_else(|error| {
            tracing::warn!(target: "control_plane", error = %error, "[pony-agent] sessions rwlock poisoned");
            error.into_inner()
        });
        // 失败已在 append_entry_resilient 内降级（warn + spool），这里只吞掉结果。
        let _ = store.append_usage_ledger_entry(&entry);
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
    /// C1（P1-2 收尾）：把 **OTel 侧同一计数器**回填到会话 trace 记录。
    ///
    /// 口径与根 span 完全一致（**不新建第二份统计**）：
    /// - `tool_failures`：`agent.turn.tool_failures` 的同一计数，`> 0` 才落值；
    /// - `degraded`：同一计数 `> 0` **且** `phase == "completed"`（部分失败但轮次正常收尾）。
    ///
    /// 返回 `(degraded, tool_failures)`，两个 `Option` 都会被 `TurnTraceRecord` 的
    /// 值敏感序列化过滤（`Some(false)`/`Some(0)` 同样不写键）。
    fn session_trace_health_fields(phase: &str) -> (Option<bool>, Option<usize>) {
        let failures = crate::agent::otel::active_turn_tool_failures();
        let tool_failures = usize::try_from(failures).ok().filter(|count| *count > 0);
        let degraded = (phase == "completed" && tool_failures.is_some()).then_some(true);
        (degraded, tool_failures)
    }

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
        // B017：本轮失败前已产出的 assistant 正文（`None`/全空白 ⇒ 事件正文回落既有占位文案）。
        partial_text: Option<String>,
        // P0：本轮失败前已产生的 native 工具往返（`None` ⇒ 本轮还没进入工具阶段）。
        failed_turn_native_transcript: Option<Vec<Value>>,
    ) {
        // PA-171 C4-S1：归因按「本轮 provider 元数据 + 错误文本」派生（矩阵 §4.2 冻结取值），
        // 三字段**不再写 None**；落盘（checkpoint + trace record）与 wire 事件同源同值。
        self.fail_stream_turn_with_hook_dispatch_impl(
            sink,
            control,
            session_id,
            turn_id,
            user_message,
            provider_meta,
            build_context_observation,
            trace_steps,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            first_token_latency_ms,
            turn_duration_ms,
            completed_hops,
            error,
            partial_text,
            failed_turn_native_transcript,
            None,
        );
    }

    /// PA-171 C4：带**触发方标签**的失败派发（`fallback_reason` 记录触发阶段）。
    ///
    /// 仅 provider 类失败使用：首跳 decision 失败 ⇒ `provider_decision_failed`；
    /// 工具 followup 的模型调用失败 ⇒ `provider_followup_failed`。其它失败（工具/钩子/附件）
    /// 走不带标签的同名入口，`fallback_reason` 保留矩阵冻结值 `turn_failed_no_fallback`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fail_stream_turn_with_hook_dispatch_tagged(
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
        partial_text: Option<String>,
        // P0：本轮失败前已产生的 native 工具往返（`None` ⇒ 本轮还没进入工具阶段）。
        failed_turn_native_transcript: Option<Vec<Value>>,
        trigger: &'static str,
    ) {
        self.fail_stream_turn_with_hook_dispatch_impl(
            sink,
            control,
            session_id,
            turn_id,
            user_message,
            provider_meta,
            build_context_observation,
            trace_steps,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            first_token_latency_ms,
            turn_duration_ms,
            completed_hops,
            error,
            partial_text,
            failed_turn_native_transcript,
            Some(trigger),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn fail_stream_turn_with_hook_dispatch_impl(
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
        partial_text: Option<String>,
        failed_turn_native_transcript: Option<Vec<Value>>,
        trigger: Option<&'static str>,
    ) {
        let attribution = match trigger {
            Some(trigger) => FailedTurnAttribution {
                fallback_reason: Some(trigger.to_string()),
                ..FailedTurnAttribution::from_failure(provider_meta, &error)
            },
            None => FailedTurnAttribution::from_failure(provider_meta, &error),
        };
        // P0（真实事故 pa_01a1c4d0）：失败轮同样必须进上下文。
        //
        // 事故形态：上游 LLM 报错 ⇒ 本轮只落 trace + 事件，**历史与 native transcript
        // 都没写**；下一条用户消息进来时 `prepare_turn` 取到的上下文里没有这一轮的任何
        // 痕迹（原话、已执行工具、已得结果全部消失），模型只能从头再问一遍。
        //
        // 收口口径与取消轮（`persist_cancelled_turn_outcome`）一致：历史侧写
        // 「user 原话 + 中断检查点」，native 侧写「user + 已闭合工具往返 + 中断标记」。
        // 为什么两层都要写：native 会话的 prompt 只取 transcript（历史层不进 prompt），
        // 只写历史等于没写；遗留会话反之。两层都写才是「两条 provider 路径都续得上」。
        self.persist_failed_turn_context(
            session_id,
            Some(turn_id),
            user_message,
            &tool_activities,
            partial_text.as_deref(),
            &error,
            provider_meta,
            failed_turn_native_transcript,
        );
        self.update_execution_checkpoint(
            control,
            turn_id,
            "failed",
            provider_meta,
            completed_hops,
            None,
            &trace_steps,
            &tool_activities,
            attribution.provider_source.as_deref(),
            attribution.provider_mode.as_deref(),
            attribution.fallback_reason.as_deref(),
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
            attribution.provider_source.clone(),
            attribution.provider_mode.clone(),
            build_context_observation.clone(),
            None,
            None,
            attribution.fallback_reason.clone(),
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
        emit_stream_failed_attributed(
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
            // B017：调用方（`handle_stream_tool_turn` 等）把「本轮已产出正文」透传进来，
            // 失败事件因此不再只剩占位文案；`None` 时仍是既有 `This turn failed.`（兼容锁）。
            partial_text,
            attribution,
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
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            })
            .save_input_attachments(session_id, images)
    }

    /// P0：失败轮的上下文收口（历史 + native transcript，**不写 trace**）。
    ///
    /// 分工：trace（含 per-call / hook 记录）由
    /// `persist_turn_trace_with_provider_calls_and_hooks` 负责；本方法只补「下一轮上下文
    /// 需要看到的两层载体」，所以不能复用 `append_failed_turn`（那会把 trace 再写一遍）。
    /// 落盘走 `persist_turn_outcome` ⇒ 与成功轮同一条 `append_turn` 通道（同一套日志命令、
    /// 同一套两阶段暂存提交），失败轮由此获得与成功轮一致的「可重放」语义。
    #[allow(clippy::too_many_arguments)]
    fn persist_failed_turn_context(
        &self,
        session_id: Option<&str>,
        // A1：失败轮同样把 `turn_id` 写进历史消息（gate 的历史腿唯一判据）。
        turn_id: Option<&str>,
        user_message: &str,
        tool_activities: &[TurnToolActivity],
        partial_text: Option<&str>,
        error: &str,
        provider_meta: Option<&ProviderEventMeta>,
        provider_native_transcript: Option<Vec<Value>>,
    ) {
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
        // A2（ADR 0123 补充）：失败轮一律经 A1 的唯一投影产检查点文本——**不得**因为
        // 本轮存在工具步骤就丢弃 `error`：`failed_turn_checkpoint_text` 在既有检查点
        // 文本后追加「失败原因：<reason>」，无步骤/无正文时同样保留原因。
        let assistant_message = crate::agent::session::failed_turn_checkpoint_text(
            partial_text.unwrap_or_default(),
            &checkpoint_tools,
            error,
        )
        .unwrap_or_else(|| error.to_string());
        self.persist_turn_outcome(
            session_id,
            turn_id,
            user_message,
            &assistant_message,
            provider_meta
                .map(|meta| meta.provider_name.as_str())
                .unwrap_or("runtime"),
            "failed",
            None,
            provider_native_transcript,
            Vec::new(),
            None,
            Some(HistoryMessageKind::Interrupted),
        );
    }

    pub(crate) fn persist_cancelled_turn_outcome(
        &self,
        session_id: Option<&str>,
        // A1：取消轮同样把 `turn_id` 写进历史消息（gate 的历史腿唯一判据）。
        turn_id: Option<&str>,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        attachments: Vec<SessionAttachment>,
        workspace_mode: Option<&str>,
        prior_assistant_text: &str,
        tool_activities: Vec<String>,
        // L1：本轮已闭合的工具往返 → native transcript 片段（无往返 ⇒ `None`）。
        provider_native_transcript: Option<Vec<Value>>,
    ) -> PersistedTurnOutcome {
        // wave-tui-session-resilience C5：中断轮次把已落盘正文 + 工具步骤一并投影为
        // additive 检查点；旧会话无步骤时回落既有占位文案，读取方按历史文本回放。
        let assistant_message = crate::agent::session::interrupted_turn_checkpoint_text(
            prior_assistant_text,
            &tool_activities,
        )
        .unwrap_or_else(|| CANCELLED_TURN_MESSAGE.to_string());
        self.persist_turn_outcome(
            session_id,
            turn_id,
            user_message,
            &assistant_message,
            provider_meta
                .map(|meta| meta.provider_name.as_str())
                .unwrap_or("runtime"),
            "cancelled",
            None,
            provider_native_transcript,
            attachments,
            workspace_mode,
            Some(HistoryMessageKind::Cancelled),
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
        // PA-171 C4-S3：失败轮落盘归类（与 wire 事件同源同值）。
        let error_class =
            Some(crate::agent::retry::classify_error_class(error_message.as_str()).to_string());
        let assistant_message = error_message.clone();
        let trace_timeline = build_persisted_trace_timeline_with_provider_calls(
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
            // Wave-15 R2：失败轮同样按序号写入已发生调用的 per-call 指标。
            &provider_call_records,
        );
        // C1：失败轮的健康度字段（同一 OTel 计数器；phase="failed" ⇒ degraded=None）。
        let failed_turn_health = Self::session_trace_health_fields("failed");
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
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
                    // Wave-4.1 A6：导出态 trace_id（门禁关闭/未采样/本地降级时一律 None）——
                    // 由当前 turn 的 otel 守卫在本调用栈上下文中登记，这里按同一上下文读取。
                    trace_id: crate::agent::otel::exported_trace_id(),
                    // Wave-5 K2：失败/取消轮无有效 usage ⇒ 成本键缺席。
                    cost_usd: None,
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
                    build_context_observation_ref: None,
                    session_summary: Some(error_message.clone()),
                    fallback_reason,
                    error: Some(error_message),
                    error_class,
                    input_tokens: None,
                    cache_hit_input_tokens: None,
                    reasoning_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms,
                    turn_duration_ms,
                    // wave-54 C3/C4：来源/上下文口径由 store 侧按 per-call records 派生。
                    usage_origin: None,
                    context_input_tokens: None,
                    input_text: Some(user_message.to_string()),
                    output_text: None,
                    // C1：失败轮同样取同一 OTel 计数；`phase="failed"` ⇒ degraded 天然为 None
                    // （与根 span 语义一致：degraded 只在「有工具失败但轮次 completed」时成立）。
                    degraded: failed_turn_health.0,
                    tool_failures: failed_turn_health.1,
                    updated_at: 0,
                },
            );
    }

    /// watchdog 兜底失败落盘（契约 watchdog-fix backend 必需行为 1+2）：
    /// 命令面无活 TurnEventSink（命令行拿不到、不可复用 fail_stream_turn_with_hook_dispatch），
    /// 改用 SessionStore::record_turn_trace（按 turn_id 幂等 upsert，重复触发不会重复
    /// append 同 turn）+ update_execution_checkpoint（phase/status='failed'，写入 error）。
    /// 该路径与阻塞 provider 读（provider/mod.rs build_streaming_http_client：15s 建连 +
    /// 180s 读空闲、无总 deadline）
    /// 解耦，前端 watchdog 触发时后端可立即落盘 failed trace。
    ///
    /// trace_steps 为空时用现有 failed 步 `telemetry_builder.failed_trace_before_tool()`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn persist_watchdog_failed_turn(
        &self,
        control: &ExecutionControlRegistry,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        completed_hops: usize,
        error: String,
    ) {
        let trace_steps = if trace_steps.is_empty() {
            self.telemetry_builder.failed_trace_before_tool()
        } else {
            trace_steps
        };
        // PA-171 C4-S3：watchdog 失败落盘同样带归类（唯一来源 `classify_error_class`）。
        let error_class =
            Some(crate::agent::retry::classify_error_class(error.as_str()).to_string());
        let trace_timeline = build_persisted_trace_timeline(
            user_message,
            "failed",
            provider_meta,
            provider_source.as_deref(),
            provider_mode.as_deref(),
            None,
            &tool_activities,
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
            None,
            None,
        );
        // 必需行为 1：按 turn_id 幂等 upsert 持久化 failed trace。
        // C1：watchdog 失败落盘同样带健康度字段（同源同口径）。
        let failed_turn_health = Self::session_trace_health_fields("failed");
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            })
            .record_turn_trace(
                session_id,
                TurnTraceRecord {
                    turn_id: turn_id.to_string(),
                    session_id: session_id.map(str::to_string),
                    event_id: None,
                    // Wave-4.1 A6：导出态 trace_id（门禁关闭/未采样/本地降级时一律 None）——
                    // 由当前 turn 的 otel 守卫在本调用栈上下文中登记，这里按同一上下文读取。
                    trace_id: crate::agent::otel::exported_trace_id(),
                    // Wave-5 K2：失败/取消轮无有效 usage ⇒ 成本键缺席。
                    cost_usd: None,
                    event_type: None,
                    event_version: None,
                    sequence: None,
                    emitted_at_ms: None,
                    title: build_turn_trace_title(user_message),
                    phase: "failed".to_string(),
                    trace_steps: trace_steps.clone(),
                    trace_timeline,
                    tool_activities: tool_activities.clone(),
                    provider_call_records: Vec::new(),
                    hook_trace_records: Vec::new(),
                    provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
                    provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
                    provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
                    provider_model: provider_meta.map(|meta| meta.model.clone()),
                    provider_source: provider_source.clone(),
                    provider_mode: provider_mode.clone(),
                    build_context_observation: None,
                    build_context_observation_ref: None,
                    session_summary: Some(error.clone()),
                    fallback_reason: None,
                    error: Some(error.clone()),
                    error_class,
                    input_tokens: None,
                    cache_hit_input_tokens: None,
                    reasoning_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms: None,
                    turn_duration_ms: None,
                    // wave-54 C3/C4：失败轮无 per-call records ⇒ 来源未知。
                    usage_origin: None,
                    context_input_tokens: None,
                    input_text: Some(user_message.to_string()),
                    output_text: None,
                    // C1：失败轮同样取同一 OTel 计数；`phase="failed"` ⇒ degraded 天然为 None
                    // （与根 span 语义一致：degraded 只在「有工具失败但轮次 completed」时成立）。
                    degraded: failed_turn_health.0,
                    tool_failures: failed_turn_health.1,
                    updated_at: 0,
                },
            );
        // 必需行为 2：checkpoint phase='failed' / status='failed' / error 写入。
        self.update_execution_checkpoint(
            control,
            turn_id,
            "failed",
            provider_meta,
            completed_hops,
            None,
            &trace_steps,
            &tool_activities,
            provider_source.as_deref(),
            provider_mode.as_deref(),
            None,
            Some("failed"),
            Some(&error),
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

    /// 流式入口 prepare 成功后的 `calling_model` 检查点（0 hop、无工具活动、无 source/mode/error）。
    ///
    /// 与 `update_execution_checkpoint` 的内联调用**逐字等价**（参数原样转发），
    /// 抽出自 `start_turn_stream_with_control_and_facts` 以免该入口函数继续膨胀。
    pub(super) fn mark_stream_calling_model_checkpoint(
        &self,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        provider_meta: &ProviderEventMeta,
        trace_steps: &[TurnTraceStep],
    ) {
        self.update_execution_checkpoint(
            control,
            turn_id,
            "calling_model",
            Some(provider_meta),
            0,
            None,
            trace_steps,
            &[],
            None,
            None,
            None,
            Some("running"),
            None,
        );
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

#[cfg(test)]
mod usage_ledger_entry_tests {
    use super::*;

    fn meta() -> ProviderEventMeta {
        ProviderEventMeta {
            requested_name: "openai".to_string(),
            provider_name: "openai".to_string(),
            protocol: "openai-completions".to_string(),
            model: "gpt-4o".to_string(),
            context_window_tokens: None,
        }
    }

    /// L3 B2：**usage 缺失**的 turn 必须写 `unpriced`（cost=NULL / priced=0），绝不用 0 冒充已知成本。
    #[test]
    fn missing_usage_is_recorded_as_unpriced_not_zero_cost() {
        let entry = build_usage_ledger_entry(
            "sess-b2",
            "turn-b2",
            &meta(),
            None,
            None,
            None,
            1_700_000_000_000,
        );
        assert!(
            entry.cost_usd.is_none(),
            "usage 缺失时 cost_usd 必须为 None（SQL NULL）"
        );
        assert!(!entry.is_priced(), "usage 缺失时 priced 必须为 0");
        // 半缺失（只有 input）同样不得计价。
        let half = build_usage_ledger_entry(
            "sess-b2",
            "turn-b2-half",
            &meta(),
            Some(100),
            None,
            None,
            1_700_000_000_000,
        );
        assert!(half.cost_usd.is_none(), "只有一半 usage 时同样不得计价");
    }

    /// 对照：真实 usage + 已知模型 ⇒ priced，且与 `cost::estimate_cost_with_protocol` 完全同源。
    #[test]
    fn real_usage_is_priced_with_protocol_aware_cost() {
        let entry = build_usage_ledger_entry(
            "sess-b2",
            "turn-priced",
            &meta(),
            Some(1_000),
            Some(500),
            Some(200),
            1_700_000_000_000,
        );
        let expected = crate::agent::cost::estimate_cost_with_protocol(
            "gpt-4o",
            1_000,
            500,
            Some(200),
            crate::agent::cost::ProviderCostProtocol::from_label("openai-completions"),
        )
        .expect("known model")
        .total_cost_usd;
        assert!(entry.is_priced());
        assert_eq!(entry.cost_usd, Some(expected));
        assert_eq!(entry.input_tokens, 1_000);
        assert_eq!(entry.cache_read_tokens, 200);
    }

    /// 未知模型（真实 usage）⇒ 同样 unpriced（G1.3）。
    #[test]
    fn unknown_model_with_usage_is_unpriced() {
        let mut unknown = meta();
        unknown.model = "definitely-not-priced-model".to_string();
        let entry = build_usage_ledger_entry(
            "sess-b2",
            "turn-unknown",
            &unknown,
            Some(10),
            Some(5),
            None,
            1_700_000_000_000,
        );
        assert!(entry.cost_usd.is_none());
        assert!(!entry.is_priced());
    }
}
