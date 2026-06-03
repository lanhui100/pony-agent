use crate::agent::config::{
    ProviderReasoningEffort, ProviderRegistryStore, ProviderSelectionResolver,
};
use crate::agent::capability_bridge::{
    CapabilityFailureKind, CapabilityRegistry, CapabilityToolExecutionResult,
    McpSourceSnapshot,
};
use crate::agent::context::{DefaultTurnContextBuilder, RetrievedContextState, TurnContextBuilder};
use crate::agent::execution_control::ExecutionCheckpoint;
use crate::agent::execution_control::ExecutionControlRegistry;
use crate::agent::graph::{GraphDecision, GraphEngine, GraphRun, GraphTurnHandoff};
use crate::agent::input::TurnInputImage;
use crate::agent::planner::{GraphPlanner, LocalTurnPlanner, TurnPlanner};
use crate::agent::provider::{
    build_context_observation, provider_native_assistant_message_with_reasoning,
    provider_native_assistant_message_with_reasoning_value,
    provider_native_assistant_tool_call_message,
    provider_native_assistant_tool_call_message_with_reasoning_value,
    provider_native_tool_result_message,
    provider_native_user_message, BuildContextObservation, ProviderDecision, ProviderManager,
    ProviderRequest, ProviderResponse, ProviderStreamChunk, TokenUsage,
};
use crate::agent::session::{
    HistoryBranch, HistoryCheckoutMode, HistoryCursor, HistoryNode, SessionAttachment,
    SessionOverview, SessionSnapshot, SessionStore, TraceTimelineEntry, TurnHistoryMessage,
    TurnTraceRecord,
};
use crate::agent::telemetry::{
    DefaultTurnTelemetryBuilder, ProviderCallCacheRecord, ProviderLatencyKind,
    ProviderRequestKind, TurnTelemetryBuilder, TurnToolActivity, TurnTraceStep,
};
use crate::agent::tools::{
    builtin_tools, ToolCall, ToolDefinition, ToolExecutor, ToolRouter,
};
use crate::agent::turn_flow::{
    build_failed_turn_result, emit_stream_cancelled, emit_stream_event, emit_stream_failed,
    emit_turn_failed, normalize_user_message, preview_text, provider_decision,
    provider_decision_stream, provider_event_meta, provider_failure_message, provider_followup,
    provider_followup_stream, runtime_log,
    stream_reasoning_chunks, stream_text_chunks, token_usage_parts, PersistedTurnOutcome,
    PlannedTurn, PreparedTurn, ProviderEventMeta, SyncToolTurnOutcome, TurnEventSink,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub message: String,
    pub display_message: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    pub session_id: Option<String>,
    pub node_id: Option<String>,
    #[serde(default)]
    pub history: Vec<TurnHistoryMessage>,
    #[serde(default)]
    pub images: Vec<TurnInputImage>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    pub phase: String,
    pub provider_requested_name: String,
    pub provider_name: String,
    pub provider_protocol: String,
    pub provider_model: String,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub user_message: String,
    pub assistant_message: String,
    pub trace_steps: Vec<TurnTraceStep>,
    pub trace_timeline: Vec<TraceTimelineEntry>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub provider_call_records: Vec<ProviderCallCacheRecord>,
    pub session_summary: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStreamEvent {
    pub turn_id: String,
    pub kind: String,
    pub phase: Option<String>,
    pub text: Option<String>,
    pub reasoning_content: Option<String>,
    pub error: Option<String>,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub trace_steps: Option<Vec<TurnTraceStep>>,
    pub trace_timeline: Option<Vec<TraceTimelineEntry>>,
    pub tool_activities: Option<Vec<TurnToolActivity>>,
    pub provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    pub session_summary: Option<String>,
}

const DEFAULT_MAX_TOOL_HOPS_PER_TURN: usize = 1024;
const MAX_ALLOWED_TOOL_HOPS_PER_TURN: usize = 4096;
const MAX_TOOL_HOPS_ENV: &str = "PONY_AGENT_MAX_TOOL_HOPS_PER_TURN";
const MAX_TURN_IMAGES: usize = 3;
const MAX_TURN_IMAGE_BYTES: u64 = 24 * 1024 * 1024;
const CANCELLED_TURN_MESSAGE: &str = "This turn was cancelled.";

#[derive(Clone)]
struct ToolTurnHopRecord {
    assistant_output_text: String,
    assistant_reasoning_content: Option<String>,
    assistant_reasoning_content_value: Option<Value>,
    tool_call: ToolCall,
    tool_result: crate::agent::tools::ToolResult,
}

struct NormalizedToolDirective {
    tool_call: ToolCall,
    assistant_message: Option<Value>,
}

pub struct AgentRuntime {
    graph: GraphEngine,
    sessions: SessionStore,
    provider_resolver: Box<dyn ProviderSelectionResolver>,
    capability_registry: CapabilityRegistry,
    tool_executor: Box<dyn ToolExecutor>,
    planner: Box<dyn TurnPlanner>,
    context_builder: Box<dyn TurnContextBuilder>,
    telemetry_builder: Box<dyn TurnTelemetryBuilder>,
}

impl AgentRuntime {
    pub fn new() -> Self {
        Self::with_dependencies(
            SessionStore::new(),
            Box::new(ProviderRegistryStore::new()),
            Box::new(ToolRouter::new()),
            Box::new(LocalTurnPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    pub(crate) fn with_dependencies(
        sessions: SessionStore,
        provider_resolver: Box<dyn ProviderSelectionResolver>,
        tool_executor: Box<dyn ToolExecutor>,
        planner: Box<dyn TurnPlanner>,
        context_builder: Box<dyn TurnContextBuilder>,
        telemetry_builder: Box<dyn TurnTelemetryBuilder>,
    ) -> Self {
        Self {
            graph: GraphEngine::new("state-machine-v1"),
            sessions,
            provider_resolver,
            capability_registry: CapabilityRegistry::new(),
            tool_executor,
            planner,
            context_builder,
            telemetry_builder,
        }
    }

    pub fn name(&self) -> &'static str {
        "rust-core"
    }

    pub fn graph_engine(&self) -> &str {
        self.graph.name()
    }

    pub fn graph_contract_version(&self) -> &str {
        self.graph.contract_version()
    }

    pub fn apply_mcp_source_snapshot(&mut self, snapshot: McpSourceSnapshot) {
        self.capability_registry.replace_mcp_source_snapshot(snapshot);
    }

    #[cfg(test)]
    pub fn inspect_capability(
        &self,
        capability_id: &str,
    ) -> Option<crate::agent::capability_bridge::CapabilityView> {
        self.capability_registry.inspect_capability(capability_id)
    }

    #[cfg(test)]
    pub fn register_mcp_capability_for_test(
        &mut self,
        capability: crate::agent::capability_bridge::CapabilityView,
    ) {
        self.capability_registry.register_mcp_capability(capability);
    }

    #[cfg(test)]
    pub fn remove_mcp_source_for_test(&mut self, source_id: &str) {
        self.capability_registry.remove_source_for_test(source_id);
    }

    #[allow(dead_code)]
    pub fn start_graph_run(
        &self,
        run_id: impl Into<String>,
        goal: impl Into<String>,
        session_id: Option<&str>,
    ) -> GraphRun {
        self.graph.start_run(run_id, goal, session_id)
    }

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        self.sessions.list_sessions()
    }

    pub fn load_session_snapshot(&mut self, session_id: Option<&str>) -> SessionSnapshot {
        self.load_session_snapshot_at(session_id, None)
    }

    pub fn load_session_snapshot_at(
        &mut self,
        session_id: Option<&str>,
        node_id: Option<&str>,
    ) -> SessionSnapshot {
        self.sessions.snapshot_at(session_id, node_id, &[])
    }

    pub fn inspect_retrieved_context(
        &mut self,
        session_id: Option<&str>,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> RetrievedContextState {
        self.inspect_retrieved_context_at(session_id, None, run, checkpoint)
    }

    pub fn inspect_retrieved_context_at(
        &mut self,
        session_id: Option<&str>,
        node_id: Option<&str>,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> RetrievedContextState {
        let snapshot = self.load_session_snapshot_at(session_id, node_id);
        let inspection_user_message = snapshot
            .history
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .unwrap_or("");
        self.context_builder.retrieve_context_state(
            inspection_user_message,
            &[],
            &snapshot,
            run,
            checkpoint,
        )
    }

    #[allow(dead_code)]
    pub fn build_graph_turn_handoff(
        &mut self,
        run: Option<&GraphRun>,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> GraphTurnHandoff {
        let snapshot = self.load_session_snapshot(session_id);
        let retrieved = self.context_builder.retrieve_context_state(
            &result.user_message,
            &[],
            &snapshot,
            run,
            checkpoint,
        );
        self.graph
            .build_turn_handoff(turn_id, session_id, result, &retrieved)
    }

    #[allow(dead_code)]
    pub fn decide_graph_after_turn(
        &mut self,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> GraphDecision {
        let handoff = self.build_graph_turn_handoff(None, turn_id, session_id, result, checkpoint);
        self.graph.decide_after_turn(&handoff)
    }

    #[allow(dead_code)]
    pub fn decide_graph_after_turn_with_planner(
        &mut self,
        run: &GraphRun,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
        planner: &dyn GraphPlanner,
    ) -> GraphDecision {
        let handoff =
            self.build_graph_turn_handoff(Some(run), turn_id, session_id, result, checkpoint);
        self.graph
            .decide_after_turn_with_planner(run, &handoff, planner)
    }

    pub fn remove_session(&mut self, session_id: &str) -> Vec<SessionOverview> {
        self.sessions.remove_session(session_id)
    }

    pub fn load_history_graph(
        &mut self,
        session_id: Option<&str>,
    ) -> (Vec<HistoryNode>, Vec<HistoryBranch>, HistoryCursor) {
        self.sessions.load_history_graph(session_id)
    }

    pub fn load_history_cursor(&mut self, session_id: Option<&str>) -> HistoryCursor {
        self.sessions.load_history_cursor(session_id)
    }

    pub fn checkout_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        mode: HistoryCheckoutMode,
    ) -> Result<SessionSnapshot, String> {
        self.sessions
            .checkout_history_node(session_id, node_id, mode)
    }

    pub fn restore_branch_head(
        &mut self,
        session_id: Option<&str>,
        branch_id: Option<&str>,
    ) -> Result<SessionSnapshot, String> {
        self.sessions.restore_branch_head(session_id, branch_id)
    }

    pub fn fork_from_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
    ) -> Result<SessionSnapshot, String> {
        self.sessions.fork_from_history_node(session_id, node_id)
    }

    pub fn switch_history_branch(
        &mut self,
        session_id: Option<&str>,
        branch_id: &str,
    ) -> Result<SessionSnapshot, String> {
        self.sessions.switch_history_branch(session_id, branch_id)
    }

    fn prepare_turn(
        &mut self,
        input: &TurnInput,
        reject_empty: bool,
    ) -> Result<PreparedTurn, String> {
        let user_message = if reject_empty {
            let trimmed = input.message.trim();
            if trimmed.is_empty() {
                return Err("Message is empty.".to_string());
            }
            trimmed.to_string()
        } else {
            normalize_user_message(&input.message)
        };

        let session = self.sessions.snapshot_at(
            input.session_id.as_deref(),
            input.node_id.as_deref(),
            &input.history,
        );
        let provider = self.resolve_provider(input);
        let preliminary_retrieved =
            self.context_builder
                .retrieve_context_state(&user_message, &[], &session, None, None);
        let effective_images =
            self.resolve_turn_images(input, &preliminary_retrieved, &provider)?;
        let tools = builtin_tools();
        let retrieved = if effective_images.is_empty() {
            preliminary_retrieved
        } else {
            self.context_builder.retrieve_context_state(
                &user_message,
                &effective_images,
                &session,
                None,
                None,
            )
        };
        let planning_request =
            self.context_builder
                .build_request(self.graph.name(), &provider, &retrieved);
        let build_context_observation = build_context_observation(&planning_request, &tools);
        let display_message = input
            .display_message
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| user_message.clone());

        Ok(PreparedTurn {
            user_message,
            display_message,
            retrieved,
            provider,
            tools,
            planning_request,
            build_context_observation,
        })
    }

    fn plan_turn(&self, prepared: &PreparedTurn) -> Result<PlannedTurn, String> {
        let preflight_decision = self
            .planner
            .preflight_decision(&prepared.user_message, prepared.retrieved.planner_history());

        let (mut first_decision, initial_decision_duration_ms) =
            if prepared.provider.requires_provider_native_tool_flow() {
                match preflight_decision {
                    Some(decision) if planner_decision_can_override_native_tool_flow(&decision) => {
                        (decision, None)
                    }
                    _ => {
                        let started_at = Instant::now();
                        let decision = provider_decision(
                            &prepared.provider,
                            &prepared.planning_request,
                            &prepared.tools,
                        )?;
                        (decision, Some(started_at.elapsed().as_millis() as u64))
                    }
                }
            } else {
                match preflight_decision {
                    Some(decision) => (decision, None),
                    None => {
                        let started_at = Instant::now();
                        let decision = provider_decision(
                            &prepared.provider,
                            &prepared.planning_request,
                            &prepared.tools,
                        )?;
                        (decision, Some(started_at.elapsed().as_millis() as u64))
                    }
                }
            };

        if let Some(tool_call) = first_decision.tool_call.take() {
            let normalized = normalize_tool_directive(
                tool_call,
                first_decision.assistant_message.take(),
                &first_decision.output_text,
                first_decision.reasoning_content.as_deref(),
                first_decision.reasoning_content_value.as_ref(),
            )?;
            first_decision.tool_call = Some(normalized.tool_call);
            first_decision.assistant_message = normalized.assistant_message;
        }

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            return Err(error);
        }

        let resolved_tool_call = self.resolve_tool_call(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            first_decision.tool_call.clone(),
            !prepared.provider.requires_provider_native_tool_flow(),
        );

        Ok(PlannedTurn {
            first_decision,
            resolved_tool_call,
            initial_decision_duration_ms,
        })
    }

    fn resolve_turn_images(
        &self,
        input: &TurnInput,
        retrieved: &RetrievedContextState,
        provider: &ProviderManager,
    ) -> Result<Vec<TurnInputImage>, String> {
        let mut images = input.images.clone();

        if images.is_empty()
            && provider.supports_image_input()
            && should_recall_recent_images(retrieved)
        {
            let recall_limit = recalled_image_limit(&retrieved.turn_context.user_message);
            images = self
                .sessions
                .load_recent_images(input.session_id.as_deref(), recall_limit);
        }

        validate_turn_images(&images)?;
        Ok(images)
    }

    fn persist_turn_outcome(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_name: &str,
        provider_mode: &str,
        token_usage: Option<&TokenUsage>,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
    ) -> PersistedTurnOutcome {
        let updated_session = self.sessions.append_turn(
            session_id,
            user_message,
            assistant_message,
            provider_native_transcript,
            attachments,
        );
        let retrieved = self.context_builder.retrieve_context_state(
            user_message,
            &[],
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
    fn persist_turn_trace(
        &mut self,
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
    fn persist_turn_trace_with_provider_calls(
        &mut self,
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
        let trace_timeline = build_persisted_trace_timeline(
            user_message,
            phase,
            provider_meta,
            provider_source.as_deref(),
            provider_mode.as_deref(),
            build_context_observation.as_ref(),
            &tool_activities,
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
        self.sessions.record_turn_trace(
            session_id,
            TurnTraceRecord {
                turn_id: turn_id.to_string(),
                title: build_turn_trace_title(user_message),
                phase: phase.to_string(),
                trace_steps,
                trace_timeline,
                tool_activities,
                provider_call_records,
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
    }

    fn save_input_attachments(
        &mut self,
        input: &TurnInput,
    ) -> Result<Vec<SessionAttachment>, String> {
        self.save_input_attachments_for_session(input.session_id.as_deref(), &input.images)
    }

    fn save_input_attachments_for_session(
        &mut self,
        session_id: Option<&str>,
        images: &[TurnInputImage],
    ) -> Result<Vec<SessionAttachment>, String> {
        let Some(session_id) = session_id else {
            return Ok(Vec::new());
        };
        self.sessions.save_input_attachments(session_id, images)
    }

    fn persist_cancelled_turn_outcome(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        attachments: Vec<SessionAttachment>,
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
        )
    }

    fn update_execution_checkpoint(
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

    fn should_cancel_turn(&self, control: &ExecutionControlRegistry, turn_id: &str) -> bool {
        control.is_stop_requested(turn_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn cancel_stream_turn<S: TurnEventSink>(
        &mut self,
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
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_stream_tool_turn<S: TurnEventSink>(
        &mut self,
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
                Some("calling_tool"),
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
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &tool_activities,
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_tool",
                )),
                None,
                None,
                None,
            );

            let running_tool_activities = running_tool_activities_with_history(
                &tool_activities,
                self.telemetry_builder
                    .tool_activities_running(&current_tool_call),
            );
            emit_stream_event(
                sink,
                "turn:tool",
                turn_id.to_string(),
                "tool",
                Some("calling_tool"),
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
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_tool",
                )),
                Some(running_tool_activities),
                None,
                None,
            );

            let execution = self.execute_capability_tool_call(&current_tool_call);
            if let Some(capability) = execution.capability.as_ref() {
                runtime_log(format!(
                    "turn:capability-execute capability_id={} mode={}",
                    capability.capability_id,
                    capability.invocation_mode.as_str()
                ));
            }
            if let Some(failure_kind) = execution.failure_kind.as_ref() {
                runtime_log(format!(
                    "turn:capability-failure tool={} class={:?}",
                    current_tool_call.name, failure_kind
                ));
            }
            let invocation_record = execution.invocation_record();
            let tool_result = execution.tool_result;
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

            emit_stream_event(
                sink,
                "turn:tool",
                turn_id.to_string(),
                "tool",
                Some("calling_model"),
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
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_model",
                )),
                Some(tool_activities.clone()),
                None,
                None,
            );

            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.to_string(),
                "trace",
                Some("calling_model"),
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
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &tool_activities,
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_model",
                )),
                None,
                None,
                None,
            );

            let delta_turn_id = turn_id.to_string();
            let first_token_latency_for_emit = Rc::clone(&first_token_latency);
            let context_observation_for_delta = context_observation.clone();
            let tool_activities_for_delta = tool_activities.clone();
            let display_message_for_delta = display_message.to_string();
            let provider_meta_for_delta = provider_meta.clone();
            let turn_started_at_for_latency = *turn_started_at;
            let supports_true_streaming_followup = provider.supports_true_streaming_followup();
            let provider_call_started_at = Instant::now();
            let provider_call_first_token_latency = Rc::new(Cell::new(None));
            let provider_call_first_token_latency_for_emit =
                Rc::clone(&provider_call_first_token_latency);
            let response = match provider_followup_stream(
                provider,
                planning_request,
                tools,
                current_assistant_message.as_ref(),
                &current_tool_call,
                &tool_result,
                move |delta| {
                    let call_latency = if supports_true_streaming_followup
                        && provider_call_first_token_latency_for_emit.get().is_none()
                    {
                        let value = provider_call_started_at.elapsed().as_millis() as u64;
                        provider_call_first_token_latency_for_emit.set(Some(value));
                        Some(value)
                    } else {
                        None
                    };
                    let latency = if supports_true_streaming_followup
                        && first_token_latency_for_emit.get().is_none()
                    {
                        let value = turn_started_at_for_latency.elapsed().as_millis() as u64;
                        first_token_latency_for_emit.set(Some(value));
                        Some(value)
                    } else {
                        None
                    };

                    let (text, reasoning_content) = match delta {
                        ProviderStreamChunk::Text(text) => (Some(text), None),
                        ProviderStreamChunk::Reasoning(reasoning) => (None, Some(reasoning)),
                    };
                    let timeline_reasoning_content = reasoning_content.clone();

                    emit_stream_event(
                        sink,
                        "turn:delta",
                        delta_turn_id.clone(),
                        "delta",
                        Some("calling_model"),
                        text.clone(),
                        reasoning_content.clone(),
                        None,
                        None,
                        None,
                        None,
                        Some(context_observation_for_delta.clone()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        latency,
                        None,
                        None,
                        Some(build_stream_progress_trace_timeline(
                            display_message_for_delta.as_str(),
                            &provider_meta_for_delta,
                            None,
                            None,
                            &context_observation_for_delta,
                            &tool_activities_for_delta,
                            None,
                            timeline_reasoning_content.as_deref(),
                            call_latency.or(provider_call_first_token_latency_for_emit.get()),
                            "calling_model",
                        )),
                        None,
                        None,
                        None,
                    );
                },
            ) {
                Ok(response) => response,
                Err(error) => {
                    let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                    self.persist_turn_trace_with_provider_calls(
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        "failed",
                        trace_steps.clone(),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        Some(provider_meta),
                        None,
                        None,
                        Some(context_observation.clone()),
                        None,
                        None,
                        accumulated_fallback_reason.clone(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        None,
                        Some(error.clone()),
                    );
                    emit_stream_failed(
                        sink,
                        turn_id.to_string(),
                        Some(provider_meta),
                        trace_steps,
                        Some(tool_activities),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        Some(context_observation.clone()),
                        None,
                        Some(provider_call_records.clone()),
                        error,
                    );
                    return;
                }
            };
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
                let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                self.persist_turn_trace_with_provider_calls(
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    "failed",
                    trace_steps.clone(),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    Some(provider_meta),
                    None,
                    Some(response.provider_mode.clone()),
                    Some(context_observation.clone()),
                    None,
                    None,
                    accumulated_fallback_reason.clone(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    None,
                    Some(error.clone()),
                );
                emit_stream_failed(
                    sink,
                    turn_id.to_string(),
                    Some(provider_meta),
                    trace_steps,
                    Some(tool_activities),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(context_observation.clone()),
                    None,
                    Some(provider_call_records.clone()),
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
                        let trace_steps =
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                        self.persist_turn_trace_with_provider_calls(
                            input.session_id.as_deref(),
                            turn_id,
                            display_message,
                            "failed",
                            trace_steps.clone(),
                            tool_activities.clone(),
                            provider_call_records.clone(),
                            Some(provider_meta),
                            Some(response.provider_source.clone()),
                            Some(response.provider_mode.clone()),
                            Some(context_observation.clone()),
                            None,
                            None,
                            accumulated_fallback_reason.clone(),
                            None,
                            None,
                            None,
                            None,
                            None,
                            first_token_latency.get(),
                            Some(turn_started_at.elapsed().as_millis() as u64),
                            None,
                            Some(error.clone()),
                        );
                        emit_stream_failed(
                            sink,
                            turn_id.to_string(),
                            Some(provider_meta),
                            trace_steps,
                            Some(tool_activities),
                            first_token_latency.get(),
                            Some(turn_started_at.elapsed().as_millis() as u64),
                            Some(context_observation.clone()),
                            None,
                            Some(provider_call_records.clone()),
                            error,
                        );
                        return;
                    }
                };
                response.tool_call = Some(normalized.tool_call);
                response.assistant_message = normalized.assistant_message;
            }

            if let Some(next_tool_call) = response.tool_call.clone() {
                if completed_hops >= max_tool_hops_per_turn() {
                    let error = build_tool_hop_limit_error(max_tool_hops_per_turn());
                    let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                    self.persist_turn_trace_with_provider_calls(
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        "failed",
                        trace_steps.clone(),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        Some(provider_meta),
                        Some(response.provider_source.clone()),
                        Some(response.provider_mode.clone()),
                        Some(context_observation.clone()),
                        None,
                        None,
                        accumulated_fallback_reason.clone(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        None,
                        Some(error.clone()),
                    );
                    emit_stream_failed(
                        sink,
                        turn_id.to_string(),
                        Some(provider_meta),
                        trace_steps,
                        Some(tool_activities),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        Some(context_observation.clone()),
                        None,
                        Some(provider_call_records.clone()),
                        error,
                    );
                    return;
                }

                current_assistant_message = response.assistant_message.clone();
                current_assistant_output_text = response.output_text.clone();
                current_assistant_reasoning = response.reasoning_content.clone();
                current_assistant_reasoning_value = response.reasoning_content_value.clone();
                current_tool_call = next_tool_call;
                continue;
            }

            let completed_text = response.output_text.clone();
            let completed_mode = response.provider_mode.clone();
            let attachments = match self.save_input_attachments(input) {
                Ok(attachments) => attachments,
                Err(error) => {
                    let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                    self.persist_turn_trace_with_provider_calls(
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        "failed",
                        trace_steps.clone(),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        Some(provider_meta),
                        Some(response.provider_source.clone()),
                        Some(response.provider_mode.clone()),
                        Some(context_observation.clone()),
                        None,
                        None,
                        accumulated_fallback_reason.clone(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        None,
                        Some(error.clone()),
                    );
                    emit_stream_failed(
                        sink,
                        turn_id.to_string(),
                        Some(provider_meta),
                        trace_steps,
                        Some(tool_activities),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        Some(context_observation.clone()),
                        None,
                        Some(provider_call_records.clone()),
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
                native_transcript_for_tool_turn(user_message, &hop_records, &response),
                attachments,
            );
            let trace_steps = self
                .telemetry_builder
                .completed_trace_with_tool(all_tools_ok);
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
            self.persist_turn_trace_with_provider_calls(
                input.session_id.as_deref(),
                turn_id,
                display_message,
                "completed",
                trace_steps.clone(),
                tool_activities.clone(),
                provider_call_records.clone(),
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
                Some(turn_started_at.elapsed().as_millis() as u64),
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
                Some(turn_started_at.elapsed().as_millis() as u64),
            );
            emit_stream_event(
                sink,
                "turn:completed",
                turn_id.to_string(),
                "completed",
                Some("ready"),
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
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(trace_steps),
                Some(completed_timeline),
                Some(tool_activities),
                Some(provider_call_records.clone()),
                Some(persisted.session_summary),
            );
            return;
        }
    }

    fn handle_sync_tool_turn(
        &mut self,
        user_message: String,
        display_message: String,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_call: ToolCall,
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

        loop {
            completed_hops += 1;
            runtime_log(format!(
                "turn:tool-execute hop={} name={} args={}",
                completed_hops, current_tool_call.name, current_tool_call.arguments
            ));
            let execution = self.execute_capability_tool_call(&current_tool_call);
            if let Some(capability) = execution.capability.as_ref() {
                runtime_log(format!(
                    "turn:capability-execute capability_id={} mode={}",
                    capability.capability_id,
                    capability.invocation_mode.as_str()
                ));
            }
            if let Some(failure_kind) = execution.failure_kind.as_ref() {
                runtime_log(format!(
                    "turn:capability-failure tool={} class={:?}",
                    current_tool_call.name, failure_kind
                ));
            }
            let invocation_record = execution.invocation_record();
            let tool_result = execution.tool_result;
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
                assistant_output_text: current_assistant_output_text.clone(),
                assistant_reasoning_content: current_assistant_reasoning.clone(),
                assistant_reasoning_content_value: current_assistant_reasoning_value.clone(),
                tool_call: current_tool_call.clone(),
                tool_result: tool_result.clone(),
            });

            let provider_call_started_at = Instant::now();
            let response = match provider_followup(
                provider,
                planning_request,
                tools,
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
                return Err(build_failed_turn_result(
                    Some(provider_meta),
                    display_message,
                    error,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities,
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
                        return Err(build_failed_turn_result(
                            Some(provider_meta),
                            display_message,
                            error,
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                            tool_activities,
                        ));
                    }
                };
                response.tool_call = Some(normalized.tool_call);
                response.assistant_message = normalized.assistant_message;
            }

            if let Some(next_tool_call) = response.tool_call.clone() {
                if completed_hops >= max_tool_hops_per_turn() {
                    return Err(build_failed_turn_result(
                        Some(provider_meta),
                        display_message,
                        build_tool_hop_limit_error(max_tool_hops_per_turn()),
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities,
                    ));
                }
                current_assistant_message = response.assistant_message.clone();
                current_assistant_output_text = response.output_text.clone();
                current_assistant_reasoning = response.reasoning_content.clone();
                current_assistant_reasoning_value = response.reasoning_content_value.clone();
                current_tool_call = next_tool_call;
                continue;
            }

            return Ok(SyncToolTurnOutcome {
                assistant_message: response.output_text.clone(),
                provider_native_transcript: native_transcript_for_tool_turn(
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
                trace_timeline: Vec::new(),
                tool_activities,
                first_token_latency_ms,
            });
        }
    }

    pub fn run_turn(&mut self, input: TurnInput) -> TurnResult {
        let turn_started_at = Instant::now();
        let prepared = match self.prepare_turn(&input, false) {
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

        let planned = match self.plan_turn(&prepared) {
            Ok(planned) => planned,
            Err(error) => {
                let provider_meta = provider_event_meta(&prepared.provider);
                return build_failed_turn_result(
                    Some(&provider_meta),
                    prepared.display_message,
                    error,
                    self.telemetry_builder.failed_trace_before_tool(),
                    Vec::new(),
                );
            }
        };

        let PlannedTurn {
            first_decision,
            resolved_tool_call,
            initial_decision_duration_ms,
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
        let provider_meta = provider_event_meta(&provider);
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
            return build_failed_turn_result(
                Some(&provider_meta),
                display_message,
                error,
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
            );
        }
        let (
            assistant_message,
            provider_native_transcript,
            provider_source,
            provider_mode,
            fallback_reason,
            token_usage,
            trace_steps,
            tool_activities,
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
                initial_visible_first_token_latency_ms,
                &turn_started_at,
                &mut provider_call_records,
            ) {
                Ok(outcome) => (
                    outcome.assistant_message,
                    outcome.provider_native_transcript,
                    outcome.provider_source,
                    outcome.provider_mode,
                    outcome.fallback_reason,
                    outcome.token_usage,
                    outcome.trace_steps,
                    outcome.tool_activities,
                    outcome.first_token_latency_ms,
                ),
                Err(failed_result) => return failed_result,
            }
        } else {
            (
                first_decision.output_text.clone(),
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
                None,
            )
        };
        let attachments = match self.save_input_attachments(&input) {
            Ok(attachments) => attachments,
            Err(error) => {
                let failed_trace_steps = if tool_activities.is_empty() {
                    self.telemetry_builder.failed_trace_before_tool()
                } else {
                    let all_tools_ok = tool_activities
                        .iter()
                        .all(|activity| activity.status != "error");
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok)
                };
                let trace_turn_id = format!(
                    "sync:{}:{}",
                    input.session_id.as_deref().unwrap_or("local-dev-session"),
                    turn_started_at.elapsed().as_nanos()
                );
                self.persist_turn_trace_with_provider_calls(
                    input.session_id.as_deref(),
                    &trace_turn_id,
                    &display_message,
                    "failed",
                    failed_trace_steps.clone(),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    Some(&provider_meta),
                    None,
                    None,
                    Some(build_context_observation.clone()),
                    None,
                    None,
                    fallback_reason.clone(),
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
                return build_failed_turn_result(
                    Some(&provider_meta),
                    display_message,
                    error,
                    failed_trace_steps,
                    tool_activities,
                );
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
        );
        let trace_turn_id = format!(
            "sync:{}:{}",
            input.session_id.as_deref().unwrap_or("local-dev-session"),
            turn_started_at.elapsed().as_nanos()
        );
        self.persist_turn_trace_with_provider_calls(
            input.session_id.as_deref(),
            &trace_turn_id,
            &display_message,
            "completed",
            trace_steps.clone(),
            tool_activities.clone(),
            provider_call_records.clone(),
            Some(&provider_meta),
            Some(provider_source.clone()),
            Some(provider_mode.clone()),
            Some(build_context_observation.clone()),
            Some(assistant_message.clone()),
            None,
            fallback_reason.clone(),
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            Some(turn_started_at.elapsed().as_millis() as u64),
            Some(persisted.session_summary.clone()),
            None,
        );

        let trace_timeline = build_persisted_trace_timeline(
            display_message.as_str(),
            "completed",
            Some(&provider_meta),
            Some(provider_source.as_str()),
            Some(provider_mode.as_str()),
            Some(&build_context_observation),
            &tool_activities,
            Some(assistant_message.as_str()),
            None,
            fallback_reason.as_deref(),
            None,
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            Some(turn_started_at.elapsed().as_millis() as u64),
        );

        TurnResult {
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
            session_summary: persisted.session_summary,
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
        control.register_turn(&turn_id, input.session_id.as_deref());
        self.start_turn_stream_with_control(sink, &control, turn_id, input);
    }

    pub fn start_turn_stream_with_control<S: TurnEventSink>(
        &mut self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: String,
        input: TurnInput,
    ) {
        let turn_started_at = Instant::now();
        let prepared = match self.prepare_turn(&input, true) {
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

        let preflight_decision = self
            .planner
            .preflight_decision(&prepared.user_message, prepared.retrieved.planner_history());
        let supports_true_streaming_decision = prepared.provider.supports_true_streaming_decision();
        let turn_id_for_stream = turn_id.clone();
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
            let initial_decision_started_at = Instant::now();
            let initial_turn_first_token_latency = Rc::new(Cell::new(None));
            let initial_call_first_token_latency = Rc::new(Cell::new(None));
            let initial_turn_first_token_latency_for_emit =
                Rc::clone(&initial_turn_first_token_latency);
            let initial_call_first_token_latency_for_emit =
                Rc::clone(&initial_call_first_token_latency);
            let display_message_for_delta = prepared.display_message.clone();
            let provider_meta_for_delta = prepared_provider_meta.clone();
            let build_context_for_delta = prepared.build_context_observation.clone();

            let decision = provider_decision_stream(
                &prepared.provider,
                &prepared.planning_request,
                &prepared.tools,
                move |delta| {
                    let call_latency =
                        if initial_call_first_token_latency_for_emit.get().is_none() {
                            let value = initial_decision_started_at.elapsed().as_millis() as u64;
                            initial_call_first_token_latency_for_emit.set(Some(value));
                            Some(value)
                        } else {
                            None
                        };
                    let turn_latency =
                        if initial_turn_first_token_latency_for_emit.get().is_none() {
                            let value = turn_started_at.elapsed().as_millis() as u64;
                            initial_turn_first_token_latency_for_emit.set(Some(value));
                            Some(value)
                        } else {
                            None
                        };
                    let (text, reasoning_content) = match delta {
                        ProviderStreamChunk::Text(text) => (Some(text), None),
                        ProviderStreamChunk::Reasoning(reasoning) => (None, Some(reasoning)),
                    };
                    let timeline_reasoning_content = reasoning_content.clone();

                    emit_stream_event(
                        sink,
                        "turn:delta",
                        turn_id_for_stream.clone(),
                        "delta",
                        Some("calling_model"),
                        text,
                        reasoning_content,
                        None,
                        None,
                        None,
                        None,
                        Some(build_context_for_delta.clone()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        turn_latency,
                        None,
                        None,
                        Some(build_stream_progress_trace_timeline(
                            display_message_for_delta.as_str(),
                            &provider_meta_for_delta,
                            None,
                            None,
                            &build_context_for_delta,
                            &[],
                            None,
                            timeline_reasoning_content.as_deref(),
                            call_latency.or(initial_call_first_token_latency_for_emit.get()),
                            "calling_model",
                        )),
                        None,
                        None,
                        None,
                    );
                },
            )?;
            Ok((
                decision,
                Some(initial_decision_started_at.elapsed().as_millis() as u64),
                initial_turn_first_token_latency.get(),
                initial_call_first_token_latency.get(),
                ProviderLatencyKind::ProviderStream,
            ))
        };
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
                return match preflight_decision {
                    Some(decision) if planner_decision_can_override_native_tool_flow(&decision) => {
                        Ok((decision, None, None, None, ProviderLatencyKind::Unknown))
                    }
                    _ => {
                        if supports_true_streaming_decision {
                            stream_initial_decision().or_else(|_| decide_sync())
                        } else {
                            decide_sync()
                        }
                    }
                };
            }

            match preflight_decision {
                Some(decision) => Ok((decision, None, None, None, ProviderLatencyKind::Unknown)),
                None => {
                    if supports_true_streaming_decision {
                        stream_initial_decision().or_else(|_| decide_sync())
                    } else {
                        decide_sync()
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
        ) =
            match planned {
                Ok(result) => result,
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
            first_decision.tool_call.clone(),
            !prepared.provider.requires_provider_native_tool_flow(),
        );
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
                error,
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
            Some("calling_model"),
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
            Some(build_stream_progress_trace_timeline(
                display_message.as_str(),
                &provider_meta,
                Some(first_decision.provider_source.as_str()),
                Some(first_decision.provider_mode.as_str()),
                &build_context_observation,
                &[],
                None,
                None,
                if initial_latency_kind == ProviderLatencyKind::ProviderStream {
                    initial_call_first_token_latency_ms
                } else {
                    None
                },
                "calling_model",
            )),
            None,
            None,
            None,
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

        let first_token_latency_ms = if initial_latency_kind == ProviderLatencyKind::ProviderStream {
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
                    error,
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
        );
        let trace_steps = self.telemetry_builder.completed_trace_without_tool();
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
        self.persist_turn_trace_with_provider_calls(
            input.session_id.as_deref(),
            &turn_id,
            &display_message,
            "completed",
            trace_steps.clone(),
            Vec::new(),
            provider_call_records.clone(),
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
            Some(turn_started_at.elapsed().as_millis() as u64),
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
            Some(turn_started_at.elapsed().as_millis() as u64),
        );
        emit_stream_event(
            sink,
            "turn:completed",
            turn_id,
            "completed",
            Some("ready"),
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
            Some(turn_started_at.elapsed().as_millis() as u64),
            Some(trace_steps),
            Some(completed_timeline),
            Some(Vec::new()),
            Some(provider_call_records),
            Some(persisted.session_summary),
        );
    }

    fn resolve_provider(&self, input: &TurnInput) -> ProviderManager {
        let mut selection = self
            .provider_resolver
            .resolve_provider_selection(input.provider_id.as_deref(), input.model_id.as_deref());

        if selection.capabilities.supports_reasoning {
            selection.reasoning_effort = input.reasoning_effort.clone();
        } else {
            selection.reasoning_effort = None;
        }

        ProviderManager::new(selection)
    }

    fn resolve_tool_call(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        provider_tool_call: Option<ToolCall>,
        allow_local_fallback: bool,
    ) -> Option<ToolCall> {
        if allow_local_fallback {
            self.planner
                .select_tool_call(user_message, history, provider_tool_call)
        } else {
            provider_tool_call
        }
    }

    fn execute_capability_tool_call(&self, tool_call: &ToolCall) -> CapabilityToolExecutionResult {
        let action = match self.capability_registry.resolve_tool_call(tool_call) {
            Ok(action) => action,
            Err(failure_kind) => {
                runtime_log(format!(
                    "turn:capability-resolve-failure tool={} class={}",
                    tool_call.name,
                    failure_kind.as_str()
                ));
                return self
                    .capability_registry
                    .capability_failure_result(tool_call, failure_kind);
            }
        };

        runtime_log(format!(
            "turn:capability-resolved capability_id={} kind={} mode={}",
            action.capability.capability_id,
            action.capability.kind.as_str(),
            action.capability.invocation_mode.as_str()
        ));

        let tool_result = self.tool_executor.execute(&action.tool_call);
        let failure_kind = if tool_result.status == "ok" {
            None
        } else {
            Some(CapabilityFailureKind::InvocationFailed)
        };

        CapabilityToolExecutionResult {
            capability: Some(action.capability),
            tool_call: action.tool_call,
            tool_result,
            failure_kind,
        }
    }
}

fn annotate_capability_tool_activities(
    mut activities: Vec<TurnToolActivity>,
    invocation_record: crate::agent::telemetry::CapabilityInvocationRecord,
) -> Vec<TurnToolActivity> {
    if let Some(parent) = activities.first_mut() {
        parent.capability_invocation = Some(invocation_record);
    }
    activities
}
/*
    #[cfg(any())]
    fn start_turn_stream_uses_compat_sync_for_deepseek_tool_followup() {
        let final_text = "deepseek 工具 follow-up 已直接成功返回。";
        let server = MockHttpServer::start(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "先调用工具。",
                            "reasoning_content": "需要先读取目录再回答。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "工具结果已经足够，直接收口。"
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 60,
                    "completion_tokens": 24,
                    "total_tokens": 84
                }
            })),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "先列出文件再总结".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .expect("delta event");
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.is_some())
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: serde_json::Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: serde_json::Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            followup_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert!(followup_request.get("stream_options").is_none());
        assert_eq!(
            followup_request
                .get("messages")
                .and_then(Value::as_array)
                .and_then(|messages| messages.get(1))
                .and_then(|message| message.get("reasoning_content"))
                .and_then(Value::as_str),
            Some("需要先读取目录再回答。")
        );
        assert_eq!(first_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("ready"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream_compat_sync")
        );
        assert_eq!(completed.fallback_reason, None);
    }

    #[cfg(any())]
    fn start_turn_stream_uses_live_stream_for_deepseek_tool_followup() {
        let final_text = "deepseek follow-up completed";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "need workspace listing before answering"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 12,
                        "total_tokens": 72
                    }
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "tool output is sufficient"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 24,
                        "total_tokens": 84
                    }
                }),
            ]),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "read Cargo.toml then answer".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.as_deref() == Some(final_text))
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            followup_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert!(followup_request.get("stream_options").is_none());
        let replayed_assistant_message = followup_request
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .expect("follow-up request should replay assistant tool call message");
        assert_eq!(
            replayed_assistant_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("need workspace listing before answering")
        );
        assert_eq!(first_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("ready"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream_compat_sync")
        );
        assert_eq!(completed.fallback_reason, None);
    }
}

*/

fn planner_decision_can_override_native_tool_flow(decision: &ProviderDecision) -> bool {
    decision
        .tool_call
        .as_ref()
        .and_then(|call| call.plan.as_ref())
        .is_some()
}

fn validate_turn_images(images: &[TurnInputImage]) -> Result<(), String> {
    if images.len() > MAX_TURN_IMAGES {
        return Err(format!(
            "Too many images attached for a single turn. Limit={MAX_TURN_IMAGES}."
        ));
    }

    let total_bytes = images
        .iter()
        .map(TurnInputImage::payload_size_bytes)
        .sum::<u64>();
    if total_bytes > MAX_TURN_IMAGE_BYTES {
        return Err(format!(
            "Attached image payload is too large for a single turn. Limit={} bytes.",
            MAX_TURN_IMAGE_BYTES
        ));
    }

    Ok(())
}

fn should_recall_recent_images(retrieved: &RetrievedContextState) -> bool {
    let latest_user_message_has_attachments = retrieved
        .session_context
        .recent_history
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| !message.attachments.is_empty())
        .unwrap_or(false);
    if !latest_user_message_has_attachments {
        return false;
    }

    retrieved.turn_context.references_image
}

fn recalled_image_limit(user_message: &str) -> usize {
    if user_message.contains("这几张")
        || user_message.contains("那几张")
        || user_message.contains("那组图")
        || user_message.contains("those images")
        || user_message.contains("these images")
    {
        MAX_TURN_IMAGES
    } else {
        1
    }
}

fn normalize_tool_directive(
    mut tool_call: ToolCall,
    assistant_message: Option<Value>,
    output_text: &str,
    reasoning_content: Option<&str>,
    reasoning_content_value: Option<&Value>,
) -> Result<NormalizedToolDirective, String> {
    if !tool_call.name.trim().is_empty() {
        return Ok(NormalizedToolDirective {
            tool_call,
            assistant_message,
        });
    }

    let repaired_name = infer_tool_name_from_arguments(&tool_call.arguments).ok_or_else(|| {
        format!(
            "provider 返回了缺少工具名的 tool call，且当前无法根据参数自动修复；arguments={}",
            preview_text(&tool_call.arguments.to_string(), 200)
        )
    })?;
    runtime_log(format!(
        "turn:tool-call-repaired repaired_name={} args={}",
        repaired_name, tool_call.arguments
    ));
    tool_call.name = repaired_name;

    let rebuilt_message = match reasoning_content_value {
        Some(raw_reasoning) => provider_native_assistant_tool_call_message_with_reasoning_value(
            non_empty_text(output_text),
            Some(raw_reasoning),
            &tool_call,
        ),
        None => provider_native_assistant_tool_call_message(
            non_empty_text(output_text),
            reasoning_content,
            &tool_call,
        ),
    };

    Ok(NormalizedToolDirective {
        assistant_message: Some(rebuilt_message),
        tool_call,
    })
}

fn infer_tool_name_from_arguments(arguments: &Value) -> Option<String> {
    let object = arguments.as_object()?;
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_query = object.contains_key("query");
    let has_limit = object.contains_key("limit");
    let has_line_count = object.contains_key("lineCount");
    let has_start_line = object.contains_key("startLine");

    if object
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Some("echo_input".to_string());
    }

    let path = path?;
    if has_start_line {
        return Some("workspace_read_file_segment".to_string());
    }
    if has_query || has_line_count {
        return Some("workspace_gather_context".to_string());
    }
    if has_limit {
        if looks_like_file_path(path) {
            return Some("workspace_read_file".to_string());
        }
        return Some("workspace_list_files".to_string());
    }
    if looks_like_file_path(path) {
        return Some("workspace_read_file".to_string());
    }

    Some("workspace_path_info".to_string())
}

fn looks_like_file_path(path: &str) -> bool {
    Path::new(path).extension().is_some()
}

fn non_empty_text(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn native_transcript_for_completed_turn(
    user_message: &str,
    decision: &ProviderDecision,
    use_provider_native_tool_flow: bool,
) -> Option<Vec<Value>> {
    if !use_provider_native_tool_flow {
        return None;
    }

    let assistant_message = decision.assistant_message.clone().unwrap_or_else(|| {
        match decision.reasoning_content_value.as_ref() {
            Some(raw_reasoning) => provider_native_assistant_message_with_reasoning_value(
                &decision.output_text,
                Some(raw_reasoning),
            ),
            None => provider_native_assistant_message_with_reasoning(
                &decision.output_text,
                decision.reasoning_content.as_deref(),
            ),
        }
    });

    Some(vec![
        provider_native_user_message(user_message),
        assistant_message,
    ])
}

fn top_level_tool_activities(tool_activities: &[TurnToolActivity]) -> Vec<&TurnToolActivity> {
    tool_activities
        .iter()
        .filter(|activity| !activity.id.contains("-planned-") && !activity.id.contains("-child-"))
        .collect()
}

fn tool_activities_for_parent(
    tool_activities: &[TurnToolActivity],
    parent: &TurnToolActivity,
) -> Vec<TurnToolActivity> {
    let prefix = format!("{}-", parent.id);
    tool_activities
        .iter()
        .filter(|activity| activity.id == parent.id || activity.id.starts_with(&prefix))
        .cloned()
        .collect()
}

fn timeline_state_for_phase(phase: &str) -> String {
    match phase {
        "cancelled" => "cancelled".to_string(),
        "failed" => "error".to_string(),
        _ => "completed".to_string(),
    }
}

fn build_context_uses_retrieval(build_context_observation: &BuildContextObservation) -> bool {
    build_context_observation.message_count > 2
        || !build_context_observation.prefix_mutation_reasons.is_empty()
        || !build_context_observation
            .semi_stable_context_text
            .trim()
            .is_empty()
}

fn build_stream_started_trace_timeline(
    user_message: &str,
    provider_meta: &ProviderEventMeta,
    build_context_observation: &BuildContextObservation,
) -> Vec<TraceTimelineEntry> {
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

    timeline.push(TraceTimelineEntry {
        id: format!("input-{}", sequence),
        kind: "input".to_string(),
        label: "RECEIVE INPUT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: Some(user_message.to_string()),
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    if build_context_uses_retrieval(build_context_observation) {
        timeline.push(TraceTimelineEntry {
            id: format!("retrieval-{}", sequence),
            kind: "prepare_retrieval".to_string(),
            label: "PREPARE RETRIEVAL".to_string(),
            state: "completed".to_string(),
            sequence,
            provider_requested_name: Some(provider_meta.requested_name.clone()),
            provider_name: Some(provider_meta.provider_name.clone()),
            provider_protocol: Some(provider_meta.protocol.clone()),
            provider_model: Some(provider_meta.model.clone()),
            provider_source: None,
            provider_mode: None,
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: None,
            reasoning_content: None,
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
        });
        sequence += 1;
    }

    timeline.push(TraceTimelineEntry {
        id: format!("context-{}", sequence),
        kind: "build_context".to_string(),
        label: "BUILD CONTEXT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: Some(provider_meta.requested_name.clone()),
        provider_name: Some(provider_meta.provider_name.clone()),
        provider_protocol: Some(provider_meta.protocol.clone()),
        provider_model: Some(provider_meta.model.clone()),
        provider_source: None,
        provider_mode: None,
        build_context_observation: Some(build_context_observation.clone()),
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    timeline.push(TraceTimelineEntry {
        id: format!("model-{}", sequence),
        kind: "call_model".to_string(),
        label: "CALL MODEL #1".to_string(),
        state: "active".to_string(),
        sequence,
        provider_requested_name: Some(provider_meta.requested_name.clone()),
        provider_name: Some(provider_meta.provider_name.clone()),
        provider_protocol: Some(provider_meta.protocol.clone()),
        provider_model: Some(provider_meta.model.clone()),
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });

    timeline
}

#[allow(clippy::too_many_arguments)]
fn build_stream_progress_trace_timeline(
    user_message: &str,
    provider_meta: &ProviderEventMeta,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    build_context_observation: &BuildContextObservation,
    tool_activities: &[TurnToolActivity],
    model_output_text: Option<&str>,
    model_reasoning_content: Option<&str>,
    first_token_latency_ms: Option<u64>,
    phase: &str,
) -> Vec<TraceTimelineEntry> {
    let top_level_tools = top_level_tool_activities(tool_activities);
    let model_hops = if phase == "calling_tool" {
        top_level_tools.len().max(1)
    } else {
        top_level_tools.len() + 1
    };
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

    timeline.push(TraceTimelineEntry {
        id: format!("input-{}", sequence),
        kind: "input".to_string(),
        label: "RECEIVE INPUT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: Some(user_message.to_string()),
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    if build_context_uses_retrieval(build_context_observation) {
        timeline.push(TraceTimelineEntry {
            id: format!("retrieval-{}", sequence),
            kind: "prepare_retrieval".to_string(),
            label: "PREPARE RETRIEVAL".to_string(),
            state: "completed".to_string(),
            sequence,
            provider_requested_name: Some(provider_meta.requested_name.clone()),
            provider_name: Some(provider_meta.provider_name.clone()),
            provider_protocol: Some(provider_meta.protocol.clone()),
            provider_model: Some(provider_meta.model.clone()),
            provider_source: provider_source.map(str::to_string),
            provider_mode: provider_mode.map(str::to_string),
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: None,
            reasoning_content: None,
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
        });
        sequence += 1;
    }

    timeline.push(TraceTimelineEntry {
        id: format!("context-{}", sequence),
        kind: "build_context".to_string(),
        label: "BUILD CONTEXT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: Some(provider_meta.requested_name.clone()),
        provider_name: Some(provider_meta.provider_name.clone()),
        provider_protocol: Some(provider_meta.protocol.clone()),
        provider_model: Some(provider_meta.model.clone()),
        provider_source: provider_source.map(str::to_string),
        provider_mode: provider_mode.map(str::to_string),
        build_context_observation: Some(build_context_observation.clone()),
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    for model_index in 0..model_hops {
        let is_last_model = model_index + 1 == model_hops;
        let model_state = if phase == "calling_model" && is_last_model {
            "active"
        } else {
            "completed"
        };
        timeline.push(TraceTimelineEntry {
            id: format!("model-{}", sequence),
            kind: "call_model".to_string(),
            label: format!("CALL MODEL #{}", model_index + 1),
            state: model_state.to_string(),
            sequence,
            provider_requested_name: Some(provider_meta.requested_name.clone()),
            provider_name: Some(provider_meta.provider_name.clone()),
            provider_protocol: Some(provider_meta.protocol.clone()),
            provider_model: Some(provider_meta.model.clone()),
            provider_source: provider_source.map(str::to_string),
            provider_mode: provider_mode.map(str::to_string),
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: if phase == "calling_model" && is_last_model {
                model_output_text.map(str::to_string)
            } else {
                None
            },
            reasoning_content: if phase == "calling_model" && is_last_model {
                model_reasoning_content.map(str::to_string)
            } else {
                None
            },
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: if phase == "calling_model" && is_last_model {
                first_token_latency_ms
            } else {
                None
            },
            turn_duration_ms: None,
        });
        sequence += 1;

        if let Some(parent_tool) = top_level_tools.get(model_index) {
            let grouped_tool_activities = tool_activities_for_parent(tool_activities, parent_tool);
            let tool_state = if parent_tool.status == "running" {
                "active"
            } else if parent_tool.status == "error" {
                "error"
            } else {
                "completed"
            };
            timeline.push(TraceTimelineEntry {
                id: format!("tool-{}", sequence),
                kind: "call_tool".to_string(),
                label: format!("CALL TOOL #{} · {}", model_index + 1, parent_tool.name),
                state: tool_state.to_string(),
                sequence,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                tool_activities: grouped_tool_activities,
                text: Some(parent_tool.summary.clone()),
                reasoning_content: None,
                fallback_reason: None,
                error: if parent_tool.status == "error" {
                    Some(parent_tool.summary.clone())
                } else {
                    None
                },
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
        }
    }

    timeline
}

#[allow(clippy::too_many_arguments)]
fn build_persisted_trace_timeline(
    user_message: &str,
    phase: &str,
    provider_meta: Option<&ProviderEventMeta>,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    build_context_observation: Option<&BuildContextObservation>,
    tool_activities: &[TurnToolActivity],
    return_text: Option<&str>,
    return_reasoning_content: Option<&str>,
    fallback_reason: Option<&str>,
    error: Option<&str>,
    input_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
) -> Vec<TraceTimelineEntry> {
    let terminal_state = timeline_state_for_phase(phase);
    let tool_hops = top_level_tool_activities(tool_activities);
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

    timeline.push(TraceTimelineEntry {
        id: format!("input-{}", sequence),
        kind: "input".to_string(),
        label: "RECEIVE INPUT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: Some(user_message.to_string()),
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    if let Some(observation) = build_context_observation {
        if build_context_uses_retrieval(observation) {
            timeline.push(TraceTimelineEntry {
                id: format!("retrieval-{}", sequence),
                kind: "prepare_retrieval".to_string(),
                label: "PREPARE RETRIEVAL".to_string(),
                state: "completed".to_string(),
                sequence,
                provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
                provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
                provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
                provider_model: provider_meta.map(|meta| meta.model.clone()),
                provider_source: provider_source.map(str::to_string),
                provider_mode: provider_mode.map(str::to_string),
                build_context_observation: None,
                tool_activities: Vec::new(),
                text: None,
                reasoning_content: None,
                fallback_reason: None,
                error: None,
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
        }
    }

    timeline.push(TraceTimelineEntry {
        id: format!("context-{}", sequence),
        kind: "build_context".to_string(),
        label: "BUILD CONTEXT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
        provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
        provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
        provider_model: provider_meta.map(|meta| meta.model.clone()),
        provider_source: provider_source.map(str::to_string),
        provider_mode: provider_mode.map(str::to_string),
        build_context_observation: build_context_observation.cloned(),
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    let model_hops = if tool_hops.is_empty() {
        1
    } else {
        tool_hops.len() + 1
    };

    for model_index in 0..model_hops {
        let state = if phase == "failed" && model_index + 1 == model_hops {
            "error".to_string()
        } else if phase == "cancelled" && model_index + 1 == model_hops {
            "cancelled".to_string()
        } else {
            "completed".to_string()
        };
        timeline.push(TraceTimelineEntry {
            id: format!("model-{}", sequence),
            kind: "call_model".to_string(),
            label: format!("CALL MODEL #{}", model_index + 1),
            state,
            sequence,
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: provider_source.map(str::to_string),
            provider_mode: provider_mode.map(str::to_string),
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: None,
            reasoning_content: None,
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: if model_index == 0 {
                first_token_latency_ms
            } else {
                None
            },
            turn_duration_ms: None,
        });
        sequence += 1;

        if let Some(parent_tool) = tool_hops.get(model_index) {
            let grouped_tool_activities = tool_activities_for_parent(tool_activities, parent_tool);
            let tool_state = if parent_tool.status == "error" {
                "error".to_string()
            } else {
                "completed".to_string()
            };
            timeline.push(TraceTimelineEntry {
                id: format!("tool-{}", sequence),
                kind: "call_tool".to_string(),
                label: format!("CALL TOOL #{} · {}", model_index + 1, parent_tool.name),
                state: tool_state,
                sequence,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                tool_activities: grouped_tool_activities,
                text: Some(parent_tool.summary.clone()),
                reasoning_content: None,
                fallback_reason: None,
                error: if parent_tool.status == "error" {
                    Some(parent_tool.summary.clone())
                } else {
                    None
                },
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
        }
    }

    if let Some(last_model_entry) = timeline
        .iter_mut()
        .rev()
        .find(|entry| entry.kind == "call_model")
    {
        last_model_entry.state = terminal_state.to_string();
        last_model_entry.provider_requested_name =
            provider_meta.map(|meta| meta.requested_name.clone());
        last_model_entry.provider_name = provider_meta.map(|meta| meta.provider_name.clone());
        last_model_entry.provider_protocol = provider_meta.map(|meta| meta.protocol.clone());
        last_model_entry.provider_model = provider_meta.map(|meta| meta.model.clone());
        last_model_entry.provider_source = provider_source.map(str::to_string);
        last_model_entry.provider_mode = provider_mode.map(str::to_string);
        last_model_entry.text = return_text.map(str::to_string);
        last_model_entry.reasoning_content = return_reasoning_content.map(str::to_string);
        last_model_entry.fallback_reason = fallback_reason.map(str::to_string);
        last_model_entry.error = error.map(str::to_string);
        last_model_entry.input_tokens = input_tokens;
        last_model_entry.cache_hit_input_tokens = cache_hit_input_tokens;
        last_model_entry.reasoning_tokens = reasoning_tokens;
        last_model_entry.output_tokens = output_tokens;
        last_model_entry.total_tokens = total_tokens;
        last_model_entry.first_token_latency_ms = first_token_latency_ms;
        last_model_entry.turn_duration_ms = turn_duration_ms;
    }

    timeline
}

fn build_turn_trace_title(message: &str) -> String {
    let compact = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return "空白输入".to_string();
    }

    let count = compact.chars().count();
    if count <= 44 {
        compact
    } else {
        format!("{}…", compact.chars().take(44).collect::<String>())
    }
}

fn native_transcript_for_tool_turn(
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
    final_response: &ProviderResponse,
) -> Option<Vec<Value>> {
    let mut transcript = vec![provider_native_user_message(user_message)];
    for hop in hop_records {
        transcript.push(tool_request_assistant_message(hop));
        transcript.push(provider_native_tool_result_message(
            &hop.tool_call,
            &hop.tool_result,
        ));
    }
    transcript.push(final_assistant_message(final_response));
    Some(transcript)
}

fn tool_request_assistant_message(hop: &ToolTurnHopRecord) -> Value {
    match hop.assistant_reasoning_content_value.as_ref() {
        Some(raw_reasoning) => provider_native_assistant_tool_call_message_with_reasoning_value(
            text_if_present(&hop.assistant_output_text),
            Some(raw_reasoning),
            &hop.tool_call,
        ),
        None => provider_native_assistant_tool_call_message(
            text_if_present(&hop.assistant_output_text),
            hop.assistant_reasoning_content.as_deref(),
            &hop.tool_call,
        ),
    }
}

fn final_assistant_message(response: &ProviderResponse) -> Value {
    response.assistant_message.clone().unwrap_or_else(|| {
        match response.reasoning_content_value.as_ref() {
            Some(raw_reasoning) => provider_native_assistant_message_with_reasoning_value(
                &response.output_text,
                Some(raw_reasoning),
            ),
            None => provider_native_assistant_message_with_reasoning(
                &response.output_text,
                response.reasoning_content.as_deref(),
            ),
        }
    })
}

fn text_if_present(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn merge_fallback_reason(existing: Option<String>, next: Option<String>) -> Option<String> {
    match (existing, next) {
        (Some(existing), Some(next)) if !next.trim().is_empty() && existing != next => {
            Some(format!("{} | {}", existing, next))
        }
        (Some(existing), _) => Some(existing),
        (None, Some(next)) if !next.trim().is_empty() => Some(next),
        (None, _) => None,
    }
}

fn build_provider_call_cache_record(
    request_kind: ProviderRequestKind,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    token_usage: Option<&TokenUsage>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    latency_kind: ProviderLatencyKind,
    build_context_observation: Option<&BuildContextObservation>,
) -> ProviderCallCacheRecord {
    let (input_tokens, cache_hit_input_tokens, reasoning_tokens, output_tokens, total_tokens) =
        token_usage_parts(token_usage);

    ProviderCallCacheRecord {
        request_kind,
        provider_source: provider_source.map(str::to_string),
        provider_mode: provider_mode.map(str::to_string),
        input_tokens,
        cache_hit_input_tokens,
        cache_miss_input_tokens: derive_cache_miss_input_tokens(token_usage),
        reasoning_tokens,
        output_tokens,
        total_tokens,
        first_token_latency_ms,
        turn_duration_ms,
        latency_kind,
        prefix_mutation_reasons: build_context_observation
            .map(|observation| observation.prefix_mutation_reasons.clone())
            .unwrap_or_default(),
    }
}

fn derive_cache_miss_input_tokens(token_usage: Option<&TokenUsage>) -> Option<u64> {
    let usage = token_usage?;
    let input_tokens = usage.input_tokens?;
    let cache_hit_input_tokens = usage.cache_hit_input_tokens?;
    Some(input_tokens.saturating_sub(cache_hit_input_tokens))
}

fn merge_token_usage(
    existing: Option<TokenUsage>,
    next: Option<&TokenUsage>,
) -> Option<TokenUsage> {
    match (existing, next) {
        (Some(existing), Some(next)) => Some(TokenUsage {
            input_tokens: add_optional_u64(existing.input_tokens, next.input_tokens),
            cache_hit_input_tokens: add_optional_u64(
                existing.cache_hit_input_tokens,
                next.cache_hit_input_tokens,
            ),
            reasoning_tokens: add_optional_u64(existing.reasoning_tokens, next.reasoning_tokens),
            output_tokens: add_optional_u64(existing.output_tokens, next.output_tokens),
            total_tokens: add_optional_u64(existing.total_tokens, next.total_tokens),
        }),
        (Some(existing), None) => Some(existing),
        (None, Some(next)) => Some(next.clone()),
        (None, None) => None,
    }
}

fn add_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn running_tool_activities_with_history(
    completed: &[TurnToolActivity],
    running: Vec<TurnToolActivity>,
) -> Vec<TurnToolActivity> {
    let mut combined = completed.to_vec();
    combined.extend(running);
    combined
}

fn build_tool_hop_limit_error(limit: usize) -> String {
    format!(
        "同一 turn 内连续工具调用超过 {} 次，已停止继续 follow-up 以避免进入无限循环；如属复杂任务，可提高 PONY_AGENT_MAX_TOOL_HOPS_PER_TURN。",
        limit
    )
}

fn max_tool_hops_per_turn() -> usize {
    static MAX_TOOL_HOPS: OnceLock<usize> = OnceLock::new();
    *MAX_TOOL_HOPS.get_or_init(|| {
        parse_max_tool_hops_per_turn(std::env::var(MAX_TOOL_HOPS_ENV).ok().as_deref())
    })
}

fn parse_max_tool_hops_per_turn(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_TOOL_HOPS_PER_TURN).contains(value))
        .unwrap_or(DEFAULT_MAX_TOOL_HOPS_PER_TURN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{
        ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    };
    use crate::agent::context::{DefaultTurnContextBuilder, TurnContextBuilder};
    use crate::agent::planner::TurnPlanner;
    use crate::agent::session::{
        FileSessionBackend, SessionSnapshot, SessionStore, TurnHistoryMessage,
    };
    use crate::agent::telemetry::DefaultTurnTelemetryBuilder;
    use serde_json::json;
    use std::cell::RefCell;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct RecordingTurnEventSink {
        events: RefCell<Vec<(String, TurnStreamEvent)>>,
    }

    impl RecordingTurnEventSink {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
            }
        }
    }

    impl TurnEventSink for RecordingTurnEventSink {
        fn emit(&self, name: &str, payload: TurnStreamEvent) {
            self.events.borrow_mut().push((name.to_string(), payload));
        }
    }

    #[derive(Clone)]
    struct StaticResolver {
        selection: ResolvedProviderSelection,
    }

    impl ProviderSelectionResolver for StaticResolver {
        fn resolve_provider_selection(
            &self,
            _provider_id: Option<&str>,
            _model_id: Option<&str>,
        ) -> ResolvedProviderSelection {
            self.selection.clone()
        }
    }

    struct PassthroughPlanner;

    impl TurnPlanner for PassthroughPlanner {
        fn preflight_decision(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
        ) -> Option<ProviderDecision> {
            None
        }

        fn select_tool_call(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            provider_tool_call: Option<ToolCall>,
        ) -> Option<ToolCall> {
            provider_tool_call
        }
    }

    struct SlowPassthroughPlanner {
        delay_ms: u64,
    }

    impl TurnPlanner for SlowPassthroughPlanner {
        fn preflight_decision(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
        ) -> Option<ProviderDecision> {
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            None
        }

        fn select_tool_call(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            provider_tool_call: Option<ToolCall>,
        ) -> Option<ToolCall> {
            provider_tool_call
        }
    }

    struct StubToolExecutor;

    impl crate::agent::tools::ToolExecutor for StubToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            let output = match call.name.as_str() {
                "workspace_list_files" => {
                    "{\"entries\":[\"Cargo.toml\",\"tauri.conf.json\",\"src/\"]}".to_string()
                }
                "workspace_read_file" => {
                    "{\n  \"productName\": \"Pony Agent\",\n  \"version\": \"0.1.0\"\n}".to_string()
                }
                other => format!("unsupported tool in test: {}", other),
            };

            crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "ok".to_string(),
                output,
                duration_ms: 1,
            }
        }
    }

    struct SlowToolExecutor {
        delay_ms: u64,
    }

    impl crate::agent::tools::ToolExecutor for SlowToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            StubToolExecutor.execute(call)
        }
    }

    struct MockHttpResponse {
        content_type: &'static str,
        body: String,
    }

    struct MockHttpServer {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl MockHttpServer {
        fn start(responses: Vec<MockHttpResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let address = listener.local_addr().expect("mock server addr");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_for_thread = Arc::clone(&requests);

            let handle = thread::spawn(move || {
                for response in responses {
                    let (mut stream, _) = listener.accept().expect("accept mock request");
                    let body = read_http_request_body(&mut stream);
                    requests_for_thread.lock().unwrap().push(body);
                    write_http_response(&mut stream, response);
                }
            });

            Self {
                base_url: format!("http://{}/v1", address),
                requests,
                handle: Some(handle),
            }
        }

        fn finish(mut self) -> Vec<String> {
            if let Some(handle) = self.handle.take() {
                handle.join().expect("join mock server");
            }
            self.requests.lock().unwrap().clone()
        }
    }

    fn read_http_request_body(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let mut header_end = None;
        let mut content_length = 0usize;

        loop {
            let read = stream.read(&mut chunk).expect("read mock request");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);

            if header_end.is_none() {
                header_end = find_header_end(&buffer);
                if let Some(end) = header_end {
                    let headers = String::from_utf8_lossy(&buffer[..end]).to_string();
                    content_length = parse_content_length(&headers);
                }
            }

            if let Some(end) = header_end {
                if buffer.len() >= end + content_length {
                    break;
                }
            }
        }

        let Some(end) = header_end else {
            return String::new();
        };
        String::from_utf8_lossy(&buffer[end..end + content_length]).to_string()
    }

    fn find_header_end(buffer: &[u8]) -> Option<usize> {
        buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
    }

    fn parse_content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("Content-Length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    fn write_http_response(stream: &mut TcpStream, response: MockHttpResponse) {
        let payload = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.content_type,
            response.body.len(),
            response.body
        );
        stream
            .write_all(payload.as_bytes())
            .expect("write mock response");
        stream.flush().expect("flush mock response");
    }

    fn json_response(body: serde_json::Value) -> MockHttpResponse {
        MockHttpResponse {
            content_type: "application/json",
            body: body.to_string(),
        }
    }

    fn json_completion(text: &str) -> MockHttpResponse {
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": text
                    }
                }
            ]
        }))
    }

    fn sse_response(chunks: &[serde_json::Value]) -> MockHttpResponse {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(&chunk.to_string());
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");

        MockHttpResponse {
            content_type: "text/event-stream",
            body,
        }
    }

    fn test_provider_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "test-openai".to_string(),
            provider_name: "test-openai".to_string(),
            protocol: crate::agent::provider::ProviderProtocol::OpenAi,
            base_url,
            api_key_env_var: "TEST_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "gpt-5.4".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
            },
        }
    }

    fn deepseek_provider_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: crate::agent::provider::ProviderProtocol::OpenAi,
            base_url,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "deepseek-v4-flash".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
            },
        }
    }

    fn build_runtime_for_test(selection: ResolvedProviderSelection) -> AgentRuntime {
        AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(StubToolExecutor),
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    fn build_runtime_for_test_with_tool_executor(
        selection: ResolvedProviderSelection,
        tool_executor: Box<dyn ToolExecutor>,
    ) -> AgentRuntime {
        AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            tool_executor,
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    fn build_runtime_with_session_store(
        selection: ResolvedProviderSelection,
        sessions: SessionStore,
    ) -> AgentRuntime {
        AgentRuntime::with_dependencies(
            sessions,
            Box::new(StaticResolver { selection }),
            Box::new(StubToolExecutor),
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    #[test]
    fn capability_bridge_resolves_dotted_builtin_tool_calls_before_execution() {
        let runtime = build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_time".to_string()),
            name: "time.now".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.as_str()),
            Some("builtin:time_now")
        );
        assert_eq!(execution.tool_call.name, "time.now");
        assert_eq!(execution.tool_result.tool_name, "time.now");
        assert_eq!(execution.tool_result.status, "ok");
        assert_eq!(execution.failure_kind, None);
    }

    #[test]
    fn capability_bridge_returns_normalized_not_found_failure_for_unknown_tools() {
        let runtime = build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_unknown".to_string()),
            name: "unknown_tool".to_string(),
            arguments: json!({ "path": "src" }),
            plan: None,
        });

        assert!(execution.capability.is_none());
        assert_eq!(execution.tool_result.status, "error");
        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::CapabilityNotFound)
        );
        assert!(execution.tool_result.output.contains("capability registry"));
    }

    #[test]
    fn capability_bridge_resolves_host_registered_mcp_tool_snapshot() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Local MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://local".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:workspace_search".to_string(),
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "workspace_search".to_string(),
                description: "List files through MCP".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{\"path\":\"string\"}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_workspace_search".to_string()),
            name: "workspace_search".to_string(),
            arguments: json!({ "path": "." }),
            plan: None,
        });

        assert_eq!(
            execution
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.as_str()),
            Some("mcp:tool:workspace_search")
        );
        assert_eq!(execution.tool_result.status, "ok");
        assert_eq!(execution.failure_kind, None);
    }

    #[test]
    fn capability_bridge_keeps_mcp_as_runtime_ingress_not_planner_scheduler_state() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Local MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://local".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:workspace_search".to_string(),
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "workspace_search".to_string(),
                description: "List files through MCP".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{\"query\":\"string\"}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let planner = LocalTurnPlanner;
        let provider_tool_call = ToolCall {
            call_id: Some("call_workspace_search".to_string()),
            name: "workspace_search".to_string(),
            arguments: json!({ "query": "Cargo.toml" }),
            plan: None,
        };
        let planned = planner
            .select_tool_call("搜索 Cargo.toml", &Vec::<TurnHistoryMessage>::new(), Some(provider_tool_call))
            .expect("planner should preserve provider tool call");

        assert_eq!(planned.name, "workspace_search");
        assert_eq!(planned.arguments, json!({ "query": "Cargo.toml" }));
        assert!(planned.arguments.get("sourceId").is_none());
        assert!(planned.arguments.get("transport").is_none());
        assert!(planned.arguments.get("capabilityId").is_none());

        let execution = runtime.execute_capability_tool_call(&planned);

        assert_eq!(
            execution
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.as_str()),
            Some("mcp:tool:workspace_search")
        );
        assert_eq!(execution.tool_call.name, "workspace_search");
        assert_eq!(execution.tool_call.arguments, json!({ "query": "Cargo.toml" }));
        assert_eq!(execution.tool_result.status, "ok");
    }

    #[test]
    fn capability_bridge_propagates_source_unavailable_from_runtime_execution_path() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-offline".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Offline MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://offline".to_string(),
                availability:
                    crate::agent::capability_bridge::CapabilityAvailability::Unreachable,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:offline_search".to_string(),
                source_id: "mcp-offline".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "offline_search".to_string(),
                description: "Offline search".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_offline_search".to_string()),
            name: "offline_search".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::SourceUnavailable)
        );
        assert!(execution.tool_result.output.contains("source"));
    }

    #[test]
    fn capability_bridge_propagates_permission_denied_from_runtime_execution_path() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-approval".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Approval MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://approval".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "requires-approval".to_string(),
                updated_at_ms: 1,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:guarded_search".to_string(),
                source_id: "mcp-approval".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "guarded_search".to_string(),
                description: "Guarded search".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: true,
                host_mediated: false,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_guarded_search".to_string()),
            name: "guarded_search".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::PermissionDenied)
        );
        assert!(execution.tool_result.output.contains("审批"));
    }

    #[test]
    fn capability_bridge_propagates_malformed_response_from_runtime_execution_path() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.register_mcp_capability_for_test(crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:orphaned".to_string(),
            source_id: "mcp-missing".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "orphaned_tool".to_string(),
            description: "Orphaned tool".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        });
        runtime.remove_mcp_source_for_test("mcp-missing");

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_orphaned_tool".to_string()),
            name: "orphaned_tool".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::MalformedResponse)
        );
        assert!(execution.tool_result.output.contains("registry"));
    }

    fn temp_marker_file_path(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{stamp}.tmp"))
    }

    fn decision_tool_call(tool_name: &str, arguments: serde_json::Value) -> serde_json::Value {
        json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": format!("先调用 {}。", tool_name),
                        "reasoning_content": format!("需要先执行 {}。", tool_name),
                        "tool_calls": [
                            {
                                "id": format!("call_{}", tool_name),
                                "type": "function",
                                "function": {
                                    "name": tool_name,
                                    "arguments": arguments.to_string()
                                }
                            }
                        ]
                    }
                }
            ]
        })
    }

    fn decision_blank_tool_call(arguments: serde_json::Value) -> serde_json::Value {
        json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "继续读取目标文件。",
                        "reasoning_content": "参数已经足够，继续执行下一步读取。",
                        "tool_calls": [
                            {
                                "id": "call_blank_name",
                                "type": "function",
                                "function": {
                                    "name": "",
                                    "arguments": arguments.to_string()
                                }
                            }
                        ]
                    }
                }
            ]
        })
    }

    #[test]
    fn start_turn_stream_uses_sink_for_empty_input_failure() {
        let mut runtime = AgentRuntime::new();
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-empty".to_string(),
            TurnInput {
                message: "   ".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("test-session".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let events = sink.events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "turn:failed");
        assert_eq!(events[0].1.turn_id, "turn-empty");
        assert_eq!(events[0].1.phase.as_deref(), Some("failed"));
        assert_eq!(events[0].1.error.as_deref(), Some("Message is empty."));
    }

    #[test]
    fn start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan() {
        let selection = test_provider_selection("http://127.0.0.1:1/v1".to_string());
        let mut runtime = build_runtime_for_test(selection);
        let sink = RecordingTurnEventSink::new();
        let control = ExecutionControlRegistry::new();

        control.register_turn("turn-cancelled", Some("stop-session"));
        let response = control.request_stop("turn-cancelled");
        assert!(response.accepted);

        runtime.start_turn_stream_with_control(
            &sink,
            &control,
            "turn-cancelled".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stop-session".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let events = sink.events.borrow();
        assert!(events.iter().any(|(name, payload)| {
            name == "turn:cancelled"
                && payload.turn_id == "turn-cancelled"
                && payload.error.as_deref() == Some("stopped_by_user")
        }));

        let snapshot = runtime.load_session_snapshot(Some("stop-session"));
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].role, "user");
        assert_eq!(snapshot.history[0].content, "继续读取 tauri.conf.json");
        assert_eq!(snapshot.history[1].role, "assistant");
        assert_eq!(snapshot.history[1].content, CANCELLED_TURN_MESSAGE);
    }

    #[test]
    fn runtime_can_build_graph_turn_handoff_from_stable_turn_artifacts() {
        let selection = test_provider_selection("http://127.0.0.1:1/v1".to_string());
        let mut runtime = build_runtime_for_test(selection);
        runtime.load_session_snapshot(Some("graph-session"));
        let result = TurnResult {
            phase: "ready".to_string(),
            provider_requested_name: "OpenAI".to_string(),
            provider_name: "OpenAI".to_string(),
            provider_protocol: "openai".to_string(),
            provider_model: "gpt-5".to_string(),
            provider_source: "primary".to_string(),
            provider_mode: "standard".to_string(),
            fallback_reason: None,
            build_context_observation: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            user_message: "请继续处理".to_string(),
            assistant_message: "当前轮已收口。".to_string(),
            trace_steps: Vec::new(),
            trace_timeline: Vec::new(),
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            session_summary: "summary".to_string(),
        };
        let checkpoint = ExecutionCheckpoint {
            turn_id: "turn-graph".to_string(),
            session_id: Some("graph-session".to_string()),
            status: "completed".to_string(),
            phase: "ready".to_string(),
            provider_requested_name: Some("OpenAI".to_string()),
            provider_name: Some("OpenAI".to_string()),
            provider_protocol: Some("openai".to_string()),
            provider_model: Some("gpt-5".to_string()),
            provider_source: Some("primary".to_string()),
            provider_mode: Some("standard".to_string()),
            fallback_reason: None,
            completed_hops: 0,
            max_hops: 16,
            active_tool_name: None,
            trace_steps: Vec::new(),
            tool_activities: Vec::new(),
            error: None,
            started_at_ms: 0,
            updated_at_ms: 0,
            stop_requested_at_ms: None,
        };

        let handoff = runtime.build_graph_turn_handoff(
            None,
            Some("turn-graph"),
            Some("graph-session"),
            &result,
            Some(&checkpoint),
        );
        let decision = runtime.decide_graph_after_turn(
            Some("turn-graph"),
            Some("graph-session"),
            &result,
            Some(&checkpoint),
        );

        assert_eq!(handoff.turn_id.as_deref(), Some("turn-graph"));
        assert_eq!(handoff.session_id.as_deref(), Some("graph-session"));
        assert_eq!(handoff.long_term_memory_status, "empty");
        assert_eq!(handoff.provider_name, "OpenAI");
        assert_eq!(
            decision.kind,
            crate::agent::graph::GraphDecisionKind::WaitUser
        );
    }

    #[test]
    fn run_turn_fails_when_attachment_persistence_fails() {
        let server = MockHttpServer::start(vec![json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "我看到了图片。"
                    }
                }
            ]
        }))]);
        let marker_path = temp_marker_file_path("pony-agent-attachment-failure");
        fs::write(&marker_path, "block attachment directory").expect("write marker file");
        let storage_path = marker_path.join("sessions.json");
        let sessions = SessionStore::with_backend(Box::new(FileSessionBackend::new(storage_path)));
        let mut selection = test_provider_selection(server.base_url.clone());
        selection.capabilities.supports_image_input = true;
        let mut runtime = build_runtime_with_session_store(selection, sessions);
        let result = runtime.run_turn(TurnInput {
            message: "请看这张图".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("attachment-failure".to_string()),
            node_id: None,
            history: Vec::new(),
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,AAAA".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("diagram.png".to_string()),
            }],
        });

        assert_eq!(result.phase, "failed");
        assert!(result
            .assistant_message
            .contains("failed to create attachment directory"));
        let snapshot = runtime.load_session_snapshot(Some("attachment-failure"));
        assert!(snapshot.history.is_empty());

        let _ = server.finish();
        let _ = fs::remove_file(&marker_path);
    }

    #[test]
    fn recent_image_recall_requires_latest_user_turn_to_have_attachments() {
        let builder = DefaultTurnContextBuilder;
        let session = SessionSnapshot {
            conversation_id: "recall-session".to_string(),
            title: "新对话".to_string(),
            summary: "".to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "[已附图片 1 张：old.png]".to_string(),
                    attachments: vec![SessionAttachment {
                        id: "att-old".to_string(),
                        asset_id: "asset-recall-session-att-old".to_string(),
                        name: Some("old.png".to_string()),
                        mime_type: "image/png".to_string(),
                        relative_path: "recall-session/att-old.dataurl".to_string(),
                        size_bytes: 4,
                        created_at_ms: 1,
                    }],
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "我看到了旧图。".to_string(),
                    attachments: Vec::new(),
                },
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "继续看 runtime.rs。".to_string(),
                    attachments: Vec::new(),
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "好的。".to_string(),
                    attachments: Vec::new(),
                },
            ],
            attachment_assets: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            long_term_memory_entries: Vec::new(),
            turn_count: 2,
            last_referenced_file: None,
            updated_at_ms: 0,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: Default::default(),
            resolved_node_id: None,
            latest_node_id: None,
        };

        let retrieved =
            builder.retrieve_context_state("那张图里有什么？", &[], &session, None, None);

        assert!(!should_recall_recent_images(&retrieved));
    }

    #[test]
    fn recent_image_recall_uses_retrieved_context_when_latest_user_turn_has_attachments() {
        let builder = DefaultTurnContextBuilder;
        let session = SessionSnapshot {
            conversation_id: "recall-session".to_string(),
            title: "新对话".to_string(),
            summary: "".to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "[已附图片 1 张：diagram.png]".to_string(),
                    attachments: vec![SessionAttachment {
                        id: "att-latest".to_string(),
                        asset_id: "asset-recall-session-att-latest".to_string(),
                        name: Some("diagram.png".to_string()),
                        mime_type: "image/png".to_string(),
                        relative_path: "recall-session/att-latest.dataurl".to_string(),
                        size_bytes: 4,
                        created_at_ms: 1,
                    }],
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "我看到了图。".to_string(),
                    attachments: Vec::new(),
                },
            ],
            attachment_assets: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            long_term_memory_entries: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: 0,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: Default::default(),
            resolved_node_id: None,
            latest_node_id: None,
        };

        let retrieved =
            builder.retrieve_context_state("继续看这张图里有什么？", &[], &session, None, None);

        assert!(should_recall_recent_images(&retrieved));
    }

    #[test]
    fn run_turn_records_first_token_latency_for_reasoning_decision() {
        let server = MockHttpServer::start(vec![json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "最终答案。",
                        "reasoning_content": "先想一下。"
                    }
                }
            ]
        }))]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "请直接回答。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-reasoning-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, "最终答案。");
        assert!(result.first_token_latency_ms.is_some());

        let _ = server.finish();
    }

    #[test]
    fn run_turn_measures_first_token_latency_from_turn_start_across_tool_hops() {
        let final_text = "同步工具调用后返回最终答案。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 40,
                    "completion_tokens": 20,
                    "total_tokens": 60
                }
            })),
        ]);
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(SlowToolExecutor { delay_ms: 40 }),
        );

        let result = runtime.run_turn(TurnInput {
            message: "先调用工具再同步回答".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-tool-hop-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.assistant_message, final_text);
        assert!(result.first_token_latency_ms.unwrap_or_default() >= 40);

        let _ = server.finish();
    }

    #[test]
    fn run_turn_completes_multi_hop_tool_followups_in_single_turn() {
        let final_text = "tauri.conf.json 的第 3 行是 `\"productName\": \"Pony Agent\",`。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(decision_tool_call(
                "workspace_read_file",
                json!({"path": "tauri.conf.json"}),
            )),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "已完成文件读取。"
                        }
                    }
                ]
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "tauri.conf.json 第三行是什么？".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-multi-hop".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });
        let request_bodies = server.finish();

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, final_text);
        assert_eq!(result.tool_activities.len(), 2);
        assert_eq!(request_bodies.len(), 3);
        assert!(request_bodies[1].contains("\"tool_choice\":\"auto\""));
        assert!(request_bodies[2].contains("\"tool_choice\":\"auto\""));

        let snapshot = runtime.load_session_snapshot(Some("sync-multi-hop"));
        assert_eq!(snapshot.provider_native_transcript.len(), 6);
        assert_eq!(
            snapshot.provider_native_transcript[1]
                .get("tool_calls")
                .and_then(serde_json::Value::as_array)
                .map(|calls| calls.len()),
            Some(1)
        );
        assert_eq!(
            snapshot.provider_native_transcript[3]
                .get("tool_calls")
                .and_then(serde_json::Value::as_array)
                .map(|calls| calls.len()),
            Some(1)
        );
    }

    #[test]
    fn run_turn_accumulates_token_usage_across_tool_followups() {
        let final_text = "已累计整轮 token usage。";
        let server = MockHttpServer::start(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "先调用 workspace_list_files。",
                            "reasoning_content": "需要先执行 workspace_list_files。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": json!({"path": ".", "limit": 40}).to_string()
                                    }
                                }
                            ]
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 100,
                    "prompt_cache_hit_tokens": 30,
                    "prompt_cache_miss_tokens": 70,
                    "completion_tokens": 20,
                    "total_tokens": 120,
                    "completion_tokens_details": {
                        "reasoning_tokens": 7
                    }
                }
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "继续调用 workspace_read_file。",
                            "reasoning_content": "目录已找到，继续读取文件。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_read_file",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": json!({"path": "tauri.conf.json"}).to_string()
                                    }
                                }
                            ]
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 80,
                    "prompt_cache_hit_tokens": 25,
                    "prompt_cache_miss_tokens": 55,
                    "completion_tokens": 10,
                    "total_tokens": 90,
                    "completion_tokens_details": {
                        "reasoning_tokens": 3
                    }
                }
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "已经整理完最终结果。"
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 60,
                    "prompt_cache_hit_tokens": 15,
                    "prompt_cache_miss_tokens": 45,
                    "completion_tokens": 40,
                    "total_tokens": 100,
                    "completion_tokens_details": {
                        "reasoning_tokens": 2
                    }
                }
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "继续读取 tauri.conf.json 第三行".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-usage-accumulated".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, final_text);
        assert_eq!(result.input_tokens, Some(240));
        assert_eq!(result.cache_hit_input_tokens, Some(70));
        assert_eq!(result.reasoning_tokens, Some(12));
        assert_eq!(result.output_tokens, Some(70));
        assert_eq!(result.total_tokens, Some(310));
        let snapshot = runtime.load_session_snapshot(Some("sync-usage-accumulated"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("sync accumulated trace");
        assert_eq!(trace.provider_call_records.len(), 3);
        assert_eq!(
            trace.provider_call_records[0].request_kind,
            ProviderRequestKind::InitialRequest
        );
        assert_eq!(
            trace.provider_call_records[1].request_kind,
            ProviderRequestKind::ToolFollowup
        );
        assert_eq!(
            trace.provider_call_records[2].cache_miss_input_tokens,
            Some(45)
        );

        let _ = server.finish();
    }

    #[test]
    fn run_turn_repairs_blank_tool_name_before_execution() {
        let final_text = "tauri.conf.json 已成功读取。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(decision_blank_tool_call(json!({
                "path": "tauri.conf.json"
            }))),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "空工具名已修复并继续执行。"
                        }
                    }
                ]
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "继续查看 tauri.conf.json".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("repair-blank-tool-sync".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, final_text);
        assert_eq!(result.tool_activities.len(), 2);
        assert!(result
            .tool_activities
            .iter()
            .any(|activity| activity.name == "workspace_read_file"));

        let snapshot = runtime.load_session_snapshot(Some("repair-blank-tool-sync"));
        assert_eq!(
            snapshot.provider_native_transcript[3]
                .get("tool_calls")
                .and_then(serde_json::Value::as_array)
                .and_then(|calls| calls.first())
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("workspace_read_file")
        );

        let _ = server.finish();
    }

    #[test]
    fn tool_hop_limit_uses_default_when_env_is_missing() {
        assert_eq!(
            parse_max_tool_hops_per_turn(None),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
    }

    #[test]
    fn tool_hop_limit_accepts_reasonable_env_override() {
        assert_eq!(parse_max_tool_hops_per_turn(Some("24")), 24);
        assert_eq!(parse_max_tool_hops_per_turn(Some("1000")), 1000);
    }

    #[test]
    fn tool_hop_limit_rejects_invalid_env_values() {
        assert_eq!(
            parse_max_tool_hops_per_turn(Some("0")),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
        assert_eq!(
            parse_max_tool_hops_per_turn(Some("5000")),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
        assert_eq!(
            parse_max_tool_hops_per_turn(Some("not-a-number")),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
    }

    #[test]
    fn start_turn_stream_completes_after_multi_hop_followup_stream() {
        let final_text = "tauri.conf.json 的第 3 行是 `\"productName\": \"Pony Agent\",`。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "已经找到 tauri.conf.json。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "找到了！让我读取 tauri.conf.json 的内容："
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_read_file",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_read_file",
                                            "arguments": "{\"path\":\"tauri"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "function": {
                                            "arguments": ".conf.json\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "已读取目标文件。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
            ]),
        ]);
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver {
                selection: test_provider_selection(server.base_url.clone()),
            }),
            Box::new(StubToolExecutor),
            Box::new(SlowPassthroughPlanner { delay_ms: 40 }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-multi-hop".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json 第三行".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-multi-hop".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );
        let request_bodies = server.finish();
        let events = sink.events.borrow();
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("completed event");

        assert_eq!(completed.text.as_deref(), Some(final_text));
        assert_eq!(
            completed
                .tool_activities
                .as_ref()
                .map(|activities| activities.len()),
            Some(2)
        );
        assert!(events.iter().any(|(name, payload)| {
            name == "turn:tool"
                && payload
                    .tool_activities
                    .as_ref()
                    .map(|activities| {
                        activities
                            .iter()
                            .any(|activity| activity.name == "workspace_read_file")
                    })
                    .unwrap_or(false)
        }));
        assert_eq!(request_bodies.len(), 3);
        assert!(request_bodies[1].contains("\"tool_choice\":\"auto\""));
        assert!(request_bodies[2].contains("\"tool_choice\":\"auto\""));
    }

    #[test]
    fn start_turn_stream_emits_first_token_latency_on_reasoning_delta() {
        let server = MockHttpServer::start(vec![sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "先想一下。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "最终答案。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 6,
                    "total_tokens": 26
                }
            }),
        ])]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-reasoning-latency".to_string(),
            TurnInput {
                message: "请直接回答。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-reasoning-latency".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:delta" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("first delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("completed event");

        assert_eq!(first_delta.reasoning_content.as_deref(), Some("先想一下。"));
        assert_eq!(first_delta.text, None);
        assert!(first_delta.first_token_latency_ms.is_some());
        assert_eq!(
            completed.first_token_latency_ms,
            first_delta.first_token_latency_ms
        );
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        assert_eq!(decision_request.get("stream").and_then(Value::as_bool), Some(true));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_decision_stream")
        );
        let provider_call = completed
            .provider_call_records
            .as_ref()
            .and_then(|records| records.first())
            .expect("initial provider call record");
        assert_eq!(provider_call.latency_kind, ProviderLatencyKind::ProviderStream);
        assert!(provider_call.first_token_latency_ms.is_some());
        assert!(provider_call.turn_duration_ms.is_some());
        assert!(
            provider_call.first_token_latency_ms.unwrap()
                <= provider_call.turn_duration_ms.unwrap()
        );
        assert!(
            completed.first_token_latency_ms.unwrap()
                > provider_call.first_token_latency_ms.unwrap()
        );
    }

    #[test]
    fn start_turn_stream_sync_fallback_for_initial_decision_does_not_emit_fake_ttft() {
        let server = MockHttpServer::start(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "最终答案。",
                            "reasoning_content": "先想一下。"
                        }
                    }
                ]
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "最终答案。",
                            "reasoning_content": "先想一下。"
                        }
                    }
                ]
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-sync-fallback-no-fake-ttft".to_string(),
            TurnInput {
                message: "请直接回答。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-sync-fallback-no-fake-ttft".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .expect("first delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(first_delta.first_token_latency_ms, None);
        assert_eq!(completed.first_token_latency_ms, None);
        let streamed_decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let fallback_decision_request: Value =
            serde_json::from_str(&requests[1]).expect("fallback decision request should be json");
        assert_eq!(
            streamed_decision_request
                .get("stream")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            fallback_decision_request
                .get("stream")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(completed.provider_source.as_deref(), Some("provider_decision"));
    }

    #[test]
    fn start_turn_stream_measures_first_token_latency_from_turn_start_across_tool_hops() {
        let final_text = "工具调用后返回最终答案。";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 40,
                        "completion_tokens": 20,
                        "total_tokens": 60
                    }
                }),
            ]),
        ]);
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(SlowToolExecutor { delay_ms: 40 }),
        );
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-tool-hop-latency".to_string(),
            TurnInput {
                message: "先调用工具再回答".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-tool-hop-latency".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let _ = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .expect("first delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert!(first_delta.first_token_latency_ms.unwrap_or_default() >= 40);
        assert_eq!(
            completed.first_token_latency_ms,
            first_delta.first_token_latency_ms
        );
    }

    #[test]
    fn start_turn_stream_uses_live_stream_for_deepseek_tool_followup() {
        let final_text = "deepseek follow-up completed";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "need workspace listing before answering"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "tool output is sufficient"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 24,
                        "total_tokens": 84
                    }
                }),
            ]),
        ]);
        let mut runtime = build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "read Cargo.toml then answer".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.as_deref() == Some(final_text))
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(decision_request.get("stream").and_then(Value::as_bool), Some(true));
        assert_eq!(followup_request.get("stream").and_then(Value::as_bool), Some(true));
        assert!(followup_request.get("stream_options").is_some());
        let replayed_assistant_message = followup_request
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .expect("follow-up request should replay assistant tool call message");
        assert_eq!(
            replayed_assistant_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("need workspace listing before answering")
        );
        assert_eq!(text_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("ready"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream")
        );
        assert_eq!(completed.fallback_reason, None);
        let provider_calls = completed
            .provider_call_records
            .as_ref()
            .expect("provider call records");
        assert_eq!(provider_calls.len(), 2);
        assert!(provider_calls
            .iter()
            .all(|record| record.latency_kind == ProviderLatencyKind::ProviderStream));
        assert!(provider_calls
            .iter()
            .all(|record| record.first_token_latency_ms.is_some()));
    }

    #[test]
    fn start_turn_stream_accumulates_token_usage_across_tool_followups() {
        let final_text = "流式回合已累计整轮 token usage。";
        let server = MockHttpServer::start(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "先调用 workspace_list_files。",
                            "reasoning_content": "需要先执行 workspace_list_files。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": json!({"path": ".", "limit": 40}).to_string()
                                    }
                                }
                            ]
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 100,
                    "prompt_cache_hit_tokens": 30,
                    "prompt_cache_miss_tokens": 70,
                    "completion_tokens": 20,
                    "total_tokens": 120,
                    "completion_tokens_details": {
                        "reasoning_tokens": 7
                    }
                }
            })),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "目录已找到。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "继续读取 tauri.conf.json。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_read_file",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_read_file",
                                            "arguments": "{\"path\":\"tauri.conf.json\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 80,
                        "prompt_cache_hit_tokens": 25,
                        "prompt_cache_miss_tokens": 55,
                        "completion_tokens": 10,
                        "total_tokens": 90,
                        "completion_tokens_details": {
                            "reasoning_tokens": 3
                        }
                    }
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "已经整理完最终结果。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "prompt_cache_hit_tokens": 15,
                        "prompt_cache_miss_tokens": 45,
                        "completion_tokens": 40,
                        "total_tokens": 100,
                        "completion_tokens_details": {
                            "reasoning_tokens": 2
                        }
                    }
                }),
            ]),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-usage-accumulated".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json 第三行".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-usage-accumulated".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let events = sink.events.borrow();
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("stream completed event");

        assert_eq!(completed.text.as_deref(), Some(final_text));
        assert_eq!(completed.input_tokens, Some(240));
        assert_eq!(completed.cache_hit_input_tokens, Some(70));
        assert_eq!(completed.reasoning_tokens, Some(12));
        assert_eq!(completed.output_tokens, Some(70));
        assert_eq!(completed.total_tokens, Some(310));

        let snapshot = runtime.load_session_snapshot(Some("stream-usage-accumulated"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("stream accumulated trace");
        assert_eq!(trace.input_tokens, Some(240));
        assert_eq!(trace.cache_hit_input_tokens, Some(70));
        assert_eq!(trace.reasoning_tokens, Some(12));
        assert_eq!(trace.output_tokens, Some(70));
        assert_eq!(trace.total_tokens, Some(310));
        assert_eq!(trace.provider_call_records.len(), 3);
        assert_eq!(
            trace.provider_call_records[0].request_kind,
            ProviderRequestKind::InitialRequest
        );
        assert_eq!(
            trace.provider_call_records[1].request_kind,
            ProviderRequestKind::ToolFollowup
        );
        assert_eq!(
            trace.provider_call_records[2].cache_miss_input_tokens,
            Some(45)
        );

        let _ = server.finish();
    }

    #[test]
    fn start_turn_stream_repairs_blank_tool_name_in_followup_stream() {
        let final_text = "tauri.conf.json 已在流式回合中成功读取。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "继续读取文件内容。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_blank_name_stream",
                                        "type": "function",
                                        "function": {
                                            "name": "",
                                            "arguments": "{\"path\":\"tauri.conf.json\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            })]),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-repair-blank-stream".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("repair-blank-tool-stream".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        let events = sink.events.borrow();
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("completed event");

        assert_eq!(completed.text.as_deref(), Some(final_text));
        assert!(events.iter().any(|(name, payload)| {
            name == "turn:tool"
                && payload
                    .tool_activities
                    .as_ref()
                    .map(|activities| {
                        activities
                            .iter()
                            .any(|activity| activity.name == "workspace_read_file")
                    })
                    .unwrap_or(false)
        }));
        assert_eq!(request_bodies.len(), 3);
    }

    #[test]
    fn runtime_can_rebuild_session_snapshot_and_retrieved_context_from_history_node() {
        let server =
            MockHttpServer::start(vec![json_completion("第一答"), json_completion("第二答")]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let first = runtime.run_turn(TurnInput {
            message: "第一问".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("runtime-history".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });
        let second = runtime.run_turn(TurnInput {
            message: "第二问".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("runtime-history".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });
        assert_eq!(first.assistant_message, "第一答");
        assert_eq!(second.assistant_message, "第二答");

        let (history_nodes, _, _) = runtime.load_history_graph(Some("runtime-history"));
        let historical_node_id = history_nodes
            .first()
            .map(|node| node.node_id.clone())
            .expect("historical node should exist");

        let snapshot = runtime
            .load_session_snapshot_at(Some("runtime-history"), Some(historical_node_id.as_str()));
        assert_eq!(
            snapshot.resolved_node_id.as_deref(),
            Some(historical_node_id.as_str())
        );
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].content, "第一问");
        assert_eq!(
            snapshot.history_cursor.mode,
            crate::agent::session::HistoryCursorMode::Historical
        );

        let retrieved = runtime.inspect_retrieved_context_at(
            Some("runtime-history"),
            Some(historical_node_id.as_str()),
            None,
            None,
        );
        assert_eq!(retrieved.session_context.turn_count, 1);
        assert_eq!(retrieved.session_context.recent_history.len(), 2);

        let _ = server.finish();
    }

    #[test]
    fn persisted_trace_timeline_uses_canonical_monitor_semantics() {
        let provider_meta = ProviderEventMeta {
            requested_name: "OpenAI".to_string(),
            provider_name: "OpenAI".to_string(),
            protocol: "openai".to_string(),
            model: "gpt-5".to_string(),
        };
        let build_context_observation = BuildContextObservation {
            request_format: "responses".to_string(),
            message_count: 4,
            image_count: 0,
            tool_count: 1,
            temperature: 0.0,
            max_output_tokens: 1024,
            stable_prefix_text: "system: stable".to_string(),
            semi_stable_context_text: "developer: retrieval summary".to_string(),
            volatile_input_text: "user: request".to_string(),
            prefix_mutation_reasons: vec![
                crate::agent::provider::PrefixMutationReason::HistoryBoundaryShifted,
            ],
            request_messages_text: "system: stable\nuser: request".to_string(),
            tool_definitions_text: "workspace.read_file(path)".to_string(),
        };
        let tool_activities = vec![crate::agent::telemetry::TurnToolActivity {
            id: "tool-read-file".to_string(),
            name: "workspace.read_file".to_string(),
            status: "done".to_string(),
            summary: "read file done".to_string(),
            arguments_text: Some("{\"path\":\"src/main.ts\"}".to_string()),
            result_text: Some("{\"content\":\"ok\"}".to_string()),
            duration_seconds: Some(0.2),
            capability_invocation: None,
        }];

        let timeline = build_persisted_trace_timeline(
            "读取文件",
            "completed",
            Some(&provider_meta),
            Some("primary"),
            Some("standard"),
            Some(&build_context_observation),
            &tool_activities,
            Some("final answer"),
            None,
            None,
            None,
            Some(11),
            Some(3),
            Some(0),
            Some(7),
            Some(18),
            Some(99),
            Some(900),
        );

        let kinds = timeline
            .iter()
            .map(|entry| entry.kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "input",
                "prepare_retrieval",
                "build_context",
                "call_model",
                "call_tool",
                "call_model",
            ]
        );
        assert_eq!(timeline[1].label, "PREPARE RETRIEVAL");
        assert_eq!(timeline[4].label, "CALL TOOL #1 · workspace.read_file");
        assert_eq!(timeline[5].label, "CALL MODEL #2");
    }

    #[test]
    fn deepseek_tool_followup_uses_live_stream() {
        let final_text = "deepseek follow-up completed";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "need workspace listing before answering"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "tool output is sufficient"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 24,
                        "total_tokens": 84
                    }
                }),
            ]),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "read Cargo.toml then answer".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.as_deref() == Some(final_text))
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            followup_request.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert!(followup_request.get("stream_options").is_some());
        let replayed_assistant_message = followup_request
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .expect("follow-up request should replay assistant tool call message");
        assert_eq!(
            replayed_assistant_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("need workspace listing before answering")
        );
        assert_eq!(text_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("ready"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream")
        );
        assert_eq!(completed.fallback_reason, None);
        let provider_calls = completed
            .provider_call_records
            .as_ref()
            .expect("provider call records");
        assert_eq!(provider_calls.len(), 2);
        assert!(provider_calls
            .iter()
            .all(|record| record.first_token_latency_ms.is_some()));
    }

    #[test]
    fn deepseek_multi_hop_followup_preserves_structured_reasoning_content() {
        let final_text = "deepseek structured follow-up completed";
        let first_reasoning = json!([
            { "type": "reasoning", "text": "need workspace listing before answering" }
        ]);
        let second_reasoning = json!([
            { "type": "reasoning", "text": "need Cargo.toml content before answering" }
        ]);
        let final_reasoning = json!([
            { "type": "reasoning", "text": "tool output is sufficient" }
        ]);
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": first_reasoning.clone()
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": second_reasoning.clone()
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "read Cargo.toml next",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_read_file",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_read_file",
                                            "arguments": "{\"path\":\"Cargo.toml\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": final_reasoning.clone()
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 120,
                        "completion_tokens": 36,
                        "total_tokens": 156
                    }
                }),
            ]),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-structured-followup".to_string(),
            TurnInput {
                message: "inspect workspace then read Cargo.toml".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-structured-followup".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        let first_followup: Value =
            serde_json::from_str(&requests[1]).expect("first followup request should be json");
        let second_followup: Value =
            serde_json::from_str(&requests[2]).expect("second followup request should be json");

        let first_replayed = first_followup
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .and_then(|message| message.get("reasoning_content"))
            .cloned()
            .expect("first followup should replay structured reasoning");
        let second_replayed = second_followup
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| messages.iter().rev().find(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .map(|calls| !calls.is_empty())
                        .unwrap_or(false)
            }))
            .and_then(|message| message.get("reasoning_content"))
            .cloned()
            .expect("second followup should replay structured reasoning");

        assert_eq!(
            first_replayed,
            json!([{ "type": "reasoning", "text": "need workspace listing before answering" }])
        );
        assert_eq!(
            second_replayed,
            json!([{ "type": "reasoning", "text": "need Cargo.toml content before answering" }])
        );
    }
}
