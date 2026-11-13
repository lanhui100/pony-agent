//! 流式轮的**失败/取消收口调用面**：从 `runtime/turn_stream.rs` 拆出，控制单文件规模。
//!
//! 四个方法都是「过闸门（terminal-once）+ 参数转发」的薄调用面，语义各自完全复用
//! `fail_stream_turn_with_hook_dispatch*` / `cancel_stream_turn`；拆出只为让
//! `turn_stream.rs`（主体是 3000+ 行的流式工具轮驱动）回落规模门禁基线。
//!
//! 模块身份：仍是 `impl AgentRuntime` 的固有方法，`pub(crate)` 供同 crate 调用点使用。

use super::*;
use super::turn_control::claim_failed_terminal;

impl AgentRuntime {
    /// wave-64 R1（BLOCKING）：终态失败派发的过闸门版本 —— 除 terminal-once 闸门外，
    /// 与 `fail_stream_turn_with_hook_dispatch` **逐字等价**（原参数原样转发）。
    ///
    /// 未抢到 `running → failed` 的调用者静默返回（结构化 debug 留痕），
    /// 从而杜绝 C5 已派发 `turn:cancelled` 之后再冒出一条 `turn:failed`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fail_stream_turn_gated_dispatch(
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
        // B017：本轮失败前**已产出**的 assistant 正文（`None`/全空白 ⇒ 回落既有占位文案）。
        partial_text: Option<String>,
        // P0：本轮失败前已产生的 native 工具往返（`None` ⇒ 本轮还没进入工具阶段）。
        failed_turn_native_transcript: Option<Vec<serde_json::Value>>,
    ) {
        if !claim_failed_terminal(control, turn_id) {
            return;
        }
        self.fail_stream_turn_with_hook_dispatch(
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
        );
    }

    /// R3（L3-A #6）：失败面的**带未执行调用**版本（与取消面同构）。
    ///
    /// 唯一调用点：批内「工具被中止 + 用户已请求停止」分支——那一刻同一响应里被跳过的
    /// 调用必须一起收口，否则续跑上下文里它们彻底消失。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fail_stream_turn_gated_dispatch_pending(
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
        failed_turn_native_transcript: Option<Vec<serde_json::Value>>,
        pending_calls: Vec<crate::agent::session::interrupted_turn::PendingToolCall>,
    ) {
        if !claim_failed_terminal(control, turn_id) {
            return;
        }
        self.fail_stream_turn_with_hook_dispatch_pending(
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
            pending_calls,
        );
    }

    /// PA-171 C4：带**触发方标签**的过闸门失败派发（仅 provider 类失败使用）。
    ///
    /// 与 [`Self::fail_stream_turn_gated_dispatch`] 逐字等价，只多一个 `trigger`
    /// （写入 `fallback_reason`，使失败可归因到「首跳 decision」还是「工具 followup」）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fail_stream_turn_gated_dispatch_tagged(
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
        failed_turn_native_transcript: Option<Vec<serde_json::Value>>,
        trigger: &'static str,
    ) {
        if !claim_failed_terminal(control, turn_id) {
            return;
        }
        self.fail_stream_turn_with_hook_dispatch_tagged(
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
            trigger,
        );
    }

    /// wave-64 R2：流式轮的取消收口统一调用面（调用点已判定为"用户取消"）。
    ///
    /// 只做参数装配，语义完全复用 [`AgentRuntime::cancel_stream_turn`]
    /// （`turn:cancelled` + `stopReason` 取该轮 checkpoint 记录的原因 + checkpoint 置 cancelled）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cancel_stream_turn_for_stop<S: TurnEventSink>(
        &self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        input: &TurnInput,
        display_message: &str,
        provider_meta: &ProviderEventMeta,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        first_token_latency_ms: Option<u64>,
        turn_started_at: &Instant,
        build_context_observation: Option<BuildContextObservation>,
        // 同 [`AgentRuntime::cancel_stream_turn`]：本轮已渲染给用户的正文（可为空）。
        partial_assistant_text: &str,
        // R3：模型已宣告但未执行/未返回的调用（本调用面的三个点都在工具阶段之前 ⇒ 恒空）。
        pending_calls: Vec<crate::agent::session::interrupted_turn::PendingToolCall>,
    ) {
        self.cancel_stream_turn(
            sink,
            control,
            turn_id,
            input.session_id.as_deref(),
            display_message,
            &input.images,
            Some(provider_meta),
            trace_steps,
            tool_activities,
            first_token_latency_ms,
            Some(turn_started_at.elapsed().as_millis() as u64),
            build_context_observation,
            partial_assistant_text,
            // 本调用面的三个调用点都在**工具阶段之前**：本轮没有已闭合的工具往返，
            // 因此不写 transcript，由文本检查点承载（与失败轮同一口径）。
            None,
            pending_calls,
        );
    }
}
