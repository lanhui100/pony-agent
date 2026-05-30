use crate::agent::config::{
    ProviderReasoningEffort, ProviderRegistryStore, ProviderSelectionResolver,
};
use crate::agent::context::{DefaultTurnContextBuilder, RetrievedContextState, TurnContextBuilder};
use crate::agent::execution_control::ExecutionCheckpoint;
use crate::agent::execution_control::ExecutionControlRegistry;
use crate::agent::graph::{GraphDecision, GraphEngine, GraphRun, GraphTurnHandoff};
use crate::agent::input::TurnInputImage;
use crate::agent::planner::{GraphPlanner, LocalTurnPlanner, TurnPlanner};
use crate::agent::provider::{
    build_context_observation, provider_native_assistant_message_with_reasoning,
    provider_native_assistant_tool_call_message, provider_native_tool_result_message,
    provider_native_user_message, BuildContextObservation, ProviderDecision, ProviderManager,
    ProviderRequest, ProviderResponse, ProviderStreamChunk, TokenUsage,
};
use crate::agent::session::{
    SessionAttachment, SessionOverview, SessionSnapshot, SessionStore, TurnHistoryMessage,
    TurnTraceRecord,
};
use crate::agent::telemetry::{
    DefaultTurnTelemetryBuilder, TurnTelemetryBuilder, TurnToolActivity, TurnTraceStep,
};
use crate::agent::tools::{builtin_tools, ToolCall, ToolDefinition, ToolExecutor, ToolRouter};
use crate::agent::turn_flow::{
    build_failed_turn_result, emit_stream_cancelled, emit_stream_event, emit_stream_failed,
    emit_turn_failed, normalize_user_message, preview_text, provider_decision, provider_event_meta,
    provider_failure_message, provider_followup, provider_followup_stream, runtime_log,
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
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub user_message: String,
    pub assistant_message: String,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
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
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub trace_steps: Option<Vec<TurnTraceStep>>,
    pub tool_activities: Option<Vec<TurnToolActivity>>,
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
        self.sessions.snapshot(session_id, &[])
    }

    pub fn inspect_retrieved_context(
        &mut self,
        session_id: Option<&str>,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> RetrievedContextState {
        let snapshot = self.load_session_snapshot(session_id);
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
            .build_turn_handoff(turn_id, session_id, result, &retrieved, checkpoint)
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

        let session = self
            .sessions
            .snapshot(input.session_id.as_deref(), &input.history);
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

        let mut first_decision = if prepared.provider.requires_provider_native_tool_flow() {
            match preflight_decision {
                Some(decision) if planner_decision_can_override_native_tool_flow(&decision) => {
                    decision
                }
                _ => provider_decision(
                    &prepared.provider,
                    &prepared.planning_request,
                    &prepared.tools,
                )?,
            }
        } else {
            match preflight_decision {
                Some(decision) => decision,
                None => provider_decision(
                    &prepared.provider,
                    &prepared.planning_request,
                    &prepared.tools,
                )?,
            }
        };

        if let Some(tool_call) = first_decision.tool_call.take() {
            let normalized = normalize_tool_directive(
                tool_call,
                first_decision.assistant_message.take(),
                &first_decision.output_text,
                first_decision.reasoning_content.as_deref(),
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
        let (input_tokens, output_tokens, total_tokens) = token_usage_parts(token_usage);

        PersistedTurnOutcome {
            session_summary,
            input_tokens,
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
        fallback_reason: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        first_token_latency_ms: Option<u64>,
        session_summary: Option<String>,
        error: Option<String>,
    ) {
        self.sessions.record_turn_trace(
            session_id,
            TurnTraceRecord {
                turn_id: turn_id.to_string(),
                title: build_turn_trace_title(user_message),
                phase: phase.to_string(),
                trace_steps,
                tool_activities,
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
                output_tokens,
                total_tokens,
                first_token_latency_ms,
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
            first_token_latency_ms,
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
            build_context_observation,
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
        turn_started_at: &Instant,
    ) {
        let context_observation = build_context_observation(planning_request, tools);
        let mut hop_records = Vec::new();
        let mut tool_activities = Vec::new();
        let mut current_tool_call = tool_call;
        let mut current_assistant_message = first_decision.assistant_message.clone();
        let mut current_assistant_output_text = first_decision.output_text.clone();
        let mut current_assistant_reasoning = first_decision.reasoning_content.clone();
        let mut all_tools_ok = true;
        let mut completed_hops = 0usize;
        let mut accumulated_fallback_reason = first_decision.fallback_reason.clone();
        let first_token_latency = Rc::new(Cell::new(None));

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
                Some(trace_steps),
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
                Some(running_tool_activities),
                None,
            );

            let tool_result = self.tool_executor.execute(&current_tool_call);
            all_tools_ok &= tool_result.status == "ok";
            tool_activities.extend(
                self.telemetry_builder
                    .tool_activities_after_result(&current_tool_call, &tool_result),
            );
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
                Some(tool_activities.clone()),
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
                Some(return_trace_steps),
                None,
                None,
            );

            let delta_turn_id = turn_id.to_string();
            let first_token_latency_for_emit = Rc::clone(&first_token_latency);
            let context_observation_for_delta = context_observation.clone();
            let response = match provider_followup_stream(
                provider,
                planning_request,
                tools,
                current_assistant_message.as_ref(),
                &current_tool_call,
                &tool_result,
                move |delta| {
                    let latency = if first_token_latency_for_emit.get().is_none() {
                        let value = turn_started_at.elapsed().as_millis() as u64;
                        first_token_latency_for_emit.set(Some(value));
                        Some(value)
                    } else {
                        None
                    };

                    let (text, reasoning_content) = match delta {
                        ProviderStreamChunk::Text(text) => (Some(text), None),
                        ProviderStreamChunk::Reasoning(reasoning) => (None, Some(reasoning)),
                    };

                    emit_stream_event(
                        sink,
                        "turn:delta",
                        delta_turn_id.clone(),
                        "delta",
                        Some("calling_model"),
                        text,
                        reasoning_content,
                        None,
                        None,
                        None,
                        None,
                        Some(context_observation_for_delta.clone()),
                        None,
                        None,
                        None,
                        latency,
                        None,
                        None,
                        None,
                    );
                },
            ) {
                Ok(response) => response,
                Err(error) => {
                    let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                    self.persist_turn_trace(
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        "failed",
                        trace_steps.clone(),
                        tool_activities.clone(),
                        Some(provider_meta),
                        None,
                        None,
                        Some(context_observation.clone()),
                        accumulated_fallback_reason.clone(),
                        None,
                        None,
                        None,
                        first_token_latency.get(),
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
                        Some(context_observation.clone()),
                        error,
                    );
                    return;
                }
            };
            let mut response = response;
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
                    Some(context_observation.clone()),
                );
                return;
            }

            if let Some(error) = provider_failure_message(
                &response.provider_mode,
                response.fallback_reason.as_deref(),
            ) {
                let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                self.persist_turn_trace(
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    "failed",
                    trace_steps.clone(),
                    tool_activities.clone(),
                    Some(provider_meta),
                    None,
                    Some(response.provider_mode.clone()),
                    Some(context_observation.clone()),
                    accumulated_fallback_reason.clone(),
                    None,
                    None,
                    None,
                    first_token_latency.get(),
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
                    Some(context_observation.clone()),
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
                ) {
                    Ok(normalized) => normalized,
                    Err(error) => {
                        let trace_steps =
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                        self.persist_turn_trace(
                            input.session_id.as_deref(),
                            turn_id,
                            display_message,
                            "failed",
                            trace_steps.clone(),
                            tool_activities.clone(),
                            Some(provider_meta),
                            Some(response.provider_source.clone()),
                            Some(response.provider_mode.clone()),
                            Some(context_observation.clone()),
                            accumulated_fallback_reason.clone(),
                            None,
                            None,
                            None,
                            first_token_latency.get(),
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
                            Some(context_observation.clone()),
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
                    self.persist_turn_trace(
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        "failed",
                        trace_steps.clone(),
                        tool_activities.clone(),
                        Some(provider_meta),
                        Some(response.provider_source.clone()),
                        Some(response.provider_mode.clone()),
                        Some(context_observation.clone()),
                        accumulated_fallback_reason.clone(),
                        None,
                        None,
                        None,
                        first_token_latency.get(),
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
                        Some(context_observation.clone()),
                        error,
                    );
                    return;
                }

                current_assistant_message = response.assistant_message.clone();
                current_assistant_output_text = response.output_text.clone();
                current_assistant_reasoning = response.reasoning_content.clone();
                current_tool_call = next_tool_call;
                continue;
            }

            let completed_text = response.output_text.clone();
            let completed_mode = response.provider_mode.clone();
            let attachments = match self.save_input_attachments(input) {
                Ok(attachments) => attachments,
                Err(error) => {
                    let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                    self.persist_turn_trace(
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        "failed",
                        trace_steps.clone(),
                        tool_activities.clone(),
                        Some(provider_meta),
                        Some(response.provider_source.clone()),
                        Some(response.provider_mode.clone()),
                        Some(context_observation.clone()),
                        accumulated_fallback_reason.clone(),
                        None,
                        None,
                        None,
                        first_token_latency.get(),
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
                        Some(context_observation.clone()),
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
                response.token_usage.as_ref(),
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
            self.persist_turn_trace(
                input.session_id.as_deref(),
                turn_id,
                display_message,
                "completed",
                trace_steps.clone(),
                tool_activities.clone(),
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                Some(context_observation.clone()),
                accumulated_fallback_reason.clone(),
                persisted.input_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                Some(persisted.session_summary.clone()),
                None,
            );

            emit_stream_event(
                sink,
                "turn:completed",
                turn_id.to_string(),
                "completed",
                Some("ready"),
                Some(response.output_text),
                response.reasoning_content.clone(),
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason,
                Some(context_observation.clone()),
                persisted.input_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                Some(trace_steps),
                Some(tool_activities),
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
    ) -> Result<SyncToolTurnOutcome, TurnResult> {
        let mut hop_records = Vec::new();
        let mut tool_activities = Vec::new();
        let mut current_tool_call = tool_call;
        let mut current_assistant_message = first_decision.assistant_message.clone();
        let mut current_assistant_output_text = first_decision.output_text.clone();
        let mut current_assistant_reasoning = first_decision.reasoning_content.clone();
        let mut all_tools_ok = true;
        let mut completed_hops = 0usize;
        let mut accumulated_fallback_reason = first_decision.fallback_reason.clone();

        loop {
            completed_hops += 1;
            runtime_log(format!(
                "turn:tool-execute hop={} name={} args={}",
                completed_hops, current_tool_call.name, current_tool_call.arguments
            ));
            let tool_result = self.tool_executor.execute(&current_tool_call);
            runtime_log(format!(
                "turn:tool-result hop={} name={} status={} output_preview={}",
                completed_hops,
                tool_result.tool_name,
                tool_result.status,
                preview_text(&tool_result.output, 160)
            ));
            all_tools_ok &= tool_result.status == "ok";
            tool_activities.extend(
                self.telemetry_builder
                    .tool_activities_after_result(&current_tool_call, &tool_result),
            );
            hop_records.push(ToolTurnHopRecord {
                assistant_output_text: current_assistant_output_text.clone(),
                assistant_reasoning_content: current_assistant_reasoning.clone(),
                tool_call: current_tool_call.clone(),
                tool_result: tool_result.clone(),
            });

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
                token_usage: response.token_usage,
                trace_steps: self
                    .telemetry_builder
                    .completed_trace_with_tool(all_tools_ok),
                tool_activities,
            });
        }
    }

    pub fn run_turn(&mut self, input: TurnInput) -> TurnResult {
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
        ) = if let Some(tool_call) = resolved_tool_call {
            match self.handle_sync_tool_turn(
                user_message.clone(),
                display_message.clone(),
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                &first_decision,
                tool_call,
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
            output_tokens: persisted.output_tokens,
            total_tokens: persisted.total_tokens,
            first_token_latency_ms: None,
            user_message: display_message,
            assistant_message,
            trace_steps,
            tool_activities,
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
        let turn_started_at = Instant::now();

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
            Some(start_trace_steps.clone()),
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
                Some(prepared.build_context_observation.clone()),
            );
            return;
        }

        let planned = match self.plan_turn(&prepared) {
            Ok(planned) => planned,
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
        let PlannedTurn {
            first_decision,
            resolved_tool_call,
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
            self.persist_turn_trace(
                input.session_id.as_deref(),
                &turn_id,
                &display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                Some(&provider_meta),
                Some(first_decision.provider_source.clone()),
                Some(first_decision.provider_mode.clone()),
                Some(build_context_observation.clone()),
                first_decision.fallback_reason.clone(),
                None,
                None,
                None,
                None,
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
                Some(build_context_observation.clone()),
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
                &turn_started_at,
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
            Some(self.telemetry_builder.trace_return_active_without_tool()),
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
                Some(build_context_observation.clone()),
            );
            return;
        }

        let first_token_latency_ms =
            if let Some(reasoning_content) = first_decision.reasoning_content.as_deref() {
                stream_reasoning_chunks(
                    sink,
                    &turn_id,
                    "calling_model",
                    reasoning_content,
                    &turn_started_at,
                    None,
                )
            } else {
                None
            };
        let first_token_latency_ms = stream_text_chunks(
            sink,
            &turn_id,
            "calling_model",
            &first_decision.output_text,
            &turn_started_at,
            first_token_latency_ms,
        );
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
                self.persist_turn_trace(
                    input.session_id.as_deref(),
                    &turn_id,
                    &display_message,
                    "failed",
                    trace_steps.clone(),
                    Vec::new(),
                    Some(&provider_meta),
                    Some(first_decision.provider_source.clone()),
                    Some(first_decision.provider_mode.clone()),
                    Some(build_context_observation.clone()),
                    first_decision.fallback_reason.clone(),
                    None,
                    None,
                    None,
                    first_token_latency_ms,
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
                    Some(build_context_observation.clone()),
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
        self.persist_turn_trace(
            input.session_id.as_deref(),
            &turn_id,
            &display_message,
            "completed",
            trace_steps.clone(),
            Vec::new(),
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            Some(build_context_observation.clone()),
            first_decision.fallback_reason.clone(),
            persisted.input_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            Some(persisted.session_summary.clone()),
            None,
        );

        emit_stream_event(
            sink,
            "turn:completed",
            turn_id,
            "completed",
            Some("ready"),
            Some(first_decision.output_text),
            first_decision.reasoning_content.clone(),
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
            Some(build_context_observation.clone()),
            Some(persisted.input_tokens).flatten(),
            Some(persisted.output_tokens).flatten(),
            Some(persisted.total_tokens).flatten(),
            first_token_latency_ms,
            Some(trace_steps),
            Some(Vec::new()),
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
}

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

    Ok(NormalizedToolDirective {
        assistant_message: Some(provider_native_assistant_tool_call_message(
            non_empty_text(output_text),
            reasoning_content,
            &tool_call,
        )),
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
        provider_native_assistant_message_with_reasoning(
            &decision.output_text,
            decision.reasoning_content.as_deref(),
        )
    });

    Some(vec![
        provider_native_user_message(user_message),
        assistant_message,
    ])
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
    provider_native_assistant_tool_call_message(
        text_if_present(&hop.assistant_output_text),
        hop.assistant_reasoning_content.as_deref(),
        &hop.tool_call,
    )
}

fn final_assistant_message(response: &ProviderResponse) -> Value {
    response.assistant_message.clone().unwrap_or_else(|| {
        provider_native_assistant_message_with_reasoning(
            &response.output_text,
            response.reasoning_content.as_deref(),
        )
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
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            user_message: "请继续处理".to_string(),
            assistant_message: "当前轮已收口。".to_string(),
            trace_steps: Vec::new(),
            tool_activities: Vec::new(),
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
        };

        let retrieved =
            builder.retrieve_context_state("继续看这张图里有什么？", &[], &session, None, None);

        assert!(should_recall_recent_images(&retrieved));
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
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
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
}
