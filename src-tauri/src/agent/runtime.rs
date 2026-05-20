use crate::agent::config::{ProviderRegistryStore, ProviderSelectionResolver};
use crate::agent::context::{DefaultTurnContextBuilder, TurnContextBuilder};
use crate::agent::graph::GraphEngine;
use crate::agent::planner::{LocalTurnPlanner, TurnPlanner};
use crate::agent::provider::{
    ProviderDecision, ProviderManager, ProviderRequest, TokenUsage,
};
use crate::agent::session::{SessionOverview, SessionSnapshot, SessionStore, TurnHistoryMessage};
use crate::agent::telemetry::{
    DefaultTurnTelemetryBuilder, TurnTelemetryBuilder, TurnToolActivity, TurnTraceStep,
};
use crate::agent::turn_flow::{
    build_failed_turn_result, emit_stream_event, emit_stream_failed, emit_turn_failed,
    normalize_user_message, preview_text, provider_decision, provider_event_meta,
    provider_failure_message, provider_followup, provider_followup_stream, runtime_log,
    stream_text_chunks, token_usage_parts, PersistedTurnOutcome, PlannedTurn, PreparedTurn,
    ProviderEventMeta, SyncToolTurnOutcome,
};
use crate::agent::tools::{
    builtin_tools, ToolCall, ToolDefinition, ToolExecutor, ToolRouter,
};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;
use tauri::AppHandle;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub message: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub history: Vec<TurnHistoryMessage>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    pub phase: String,
    pub provider_requested_name: String,
    pub provider_name: String,
    pub provider_protocol: String,
    pub provider_model: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStreamEvent {
    pub turn_id: String,
    pub kind: String,
    pub phase: Option<String>,
    pub text: Option<String>,
    pub error: Option<String>,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub trace_steps: Option<Vec<TurnTraceStep>>,
    pub tool_activities: Option<Vec<TurnToolActivity>>,
    pub session_summary: Option<String>,
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

    fn with_dependencies(
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

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        self.sessions.list_sessions()
    }

    pub fn load_session_snapshot(&mut self, session_id: Option<&str>) -> SessionSnapshot {
        self.sessions.snapshot(session_id, &[])
    }

    pub fn remove_session(&mut self, session_id: &str) -> Vec<SessionOverview> {
        self.sessions.remove_session(session_id)
    }

    fn prepare_turn(&mut self, input: &TurnInput, reject_empty: bool) -> Result<PreparedTurn, String> {
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
        let tools = builtin_tools();
        let planning_request = self
            .context_builder
            .build_request(self.graph.name(), &provider, &user_message, &session);

        Ok(PreparedTurn {
            user_message,
            session,
            provider,
            tools,
            planning_request,
        })
    }

    fn plan_turn(&self, prepared: &PreparedTurn) -> Result<PlannedTurn, String> {
        let first_decision = match self
            .planner
            .preflight_decision(&prepared.user_message, &prepared.session.history)
        {
            Some(decision) => decision,
            None => provider_decision(&prepared.provider, &prepared.planning_request, &prepared.tools)?,
        };

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            return Err(error);
        }

        let resolved_tool_call = self.resolve_tool_call(
            &prepared.user_message,
            &prepared.session.history,
            first_decision.tool_call.clone(),
        );

        Ok(PlannedTurn {
            first_decision,
            resolved_tool_call,
        })
    }

    fn persist_turn_outcome(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_name: &str,
        provider_mode: &str,
        token_usage: Option<&TokenUsage>,
    ) -> PersistedTurnOutcome {
        let updated_session = self
            .sessions
            .append_turn(session_id, user_message, assistant_message);
        let session_summary = self.context_builder.build_session_summary(
            self.graph.name(),
            &updated_session,
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
    fn handle_stream_tool_turn(
        &mut self,
        app: &AppHandle,
        turn_id: &str,
        input: &TurnInput,
        user_message: &str,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_call: ToolCall,
        turn_started_at: &Instant,
    ) {
        emit_stream_event(
            app,
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
            None,
            None,
            Some(self.telemetry_builder.trace_tool_active()),
            None,
            None,
        );

        emit_stream_event(
            app,
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
            None,
            None,
            None,
            Some(self.telemetry_builder.tool_activities_running(&tool_call)),
            None,
        );

        let tool_result = self.tool_executor.execute(&tool_call);

        emit_stream_event(
            app,
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
            None,
            None,
            None,
            Some(self.telemetry_builder.tool_activities_after_result(&tool_call, &tool_result)),
            None,
        );

        emit_stream_event(
            app,
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
            None,
            None,
            Some(self.telemetry_builder.trace_return_active(tool_result.status == "ok")),
            None,
            None,
        );

        let delta_app = app.clone();
        let delta_turn_id = turn_id.to_string();
        let first_token_latency = Rc::new(Cell::new(None));
        let first_token_latency_for_emit = Rc::clone(&first_token_latency);

        let final_response = match provider_followup_stream(
            provider,
            planning_request,
            tools,
            &tool_call,
            &tool_result,
            move |delta| {
                let latency = if first_token_latency_for_emit.get().is_none() {
                    let value = turn_started_at.elapsed().as_millis() as u64;
                    first_token_latency_for_emit.set(Some(value));
                    Some(value)
                } else {
                    None
                };

                emit_stream_event(
                    &delta_app,
                    "turn:delta",
                    delta_turn_id.clone(),
                    "delta",
                    Some("calling_model"),
                    Some(delta),
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
                );
            },
        ) {
            Ok(response) => response,
            Err(error) => {
                emit_stream_failed(
                    app,
                    turn_id.to_string(),
                    Some(provider_meta),
                    self.telemetry_builder
                        .failed_trace_after_tool(tool_result.status == "ok"),
                    Some(
                        self.telemetry_builder
                            .tool_activities_after_result(&tool_call, &tool_result),
                    ),
                    first_token_latency.get(),
                    error,
                );
                return;
            }
        };

        if let Some(error) = provider_failure_message(
            &final_response.provider_mode,
            final_response.fallback_reason.as_deref(),
        ) {
            emit_stream_failed(
                app,
                turn_id.to_string(),
                Some(provider_meta),
                self.telemetry_builder
                    .failed_trace_after_tool(tool_result.status == "ok"),
                Some(
                    self.telemetry_builder
                        .tool_activities_after_result(&tool_call, &tool_result),
                ),
                first_token_latency.get(),
                error,
            );
            return;
        }

        let completed_text = final_response.output_text.clone();
        let completed_mode = final_response.provider_mode.clone();
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            user_message,
            &completed_text,
            provider.name(),
            &completed_mode,
            final_response.token_usage.as_ref(),
        );

        emit_stream_event(
            app,
            "turn:completed",
            turn_id.to_string(),
            "completed",
            Some("ready"),
            Some(final_response.output_text),
            Some(provider_meta),
            Some(final_response.provider_mode.clone()),
            final_response
                .fallback_reason
                .or(first_decision.fallback_reason.clone()),
            persisted.input_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency.get(),
            Some(
                self.telemetry_builder
                    .completed_trace_with_tool(tool_result.status == "ok"),
            ),
            Some(
                self.telemetry_builder
                    .tool_activities_after_result(&tool_call, &tool_result),
            ),
            Some(persisted.session_summary),
        );
    }

    fn handle_sync_tool_turn(
        &mut self,
        user_message: String,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        tool_call: ToolCall,
    ) -> Result<SyncToolTurnOutcome, TurnResult> {
        runtime_log(format!(
            "turn:tool-execute name={} args={}",
            tool_call.name, tool_call.arguments
        ));
        let tool_result = self.tool_executor.execute(&tool_call);
        runtime_log(format!(
            "turn:tool-result name={} status={} output_preview={}",
            tool_result.tool_name,
            tool_result.status,
            preview_text(&tool_result.output, 160)
        ));

        let final_response = match provider_followup(
            provider,
            planning_request,
            tools,
            &tool_call,
            &tool_result,
        ) {
            Ok(response) => response,
            Err(error) => {
                return Err(build_failed_turn_result(
                    Some(provider_meta),
                    user_message,
                    error,
                    self.telemetry_builder
                        .failed_trace_after_tool(tool_result.status == "ok"),
                    self.telemetry_builder
                        .tool_activities_after_result(&tool_call, &tool_result),
                ));
            }
        };

        if let Some(error) = provider_failure_message(
            &final_response.provider_mode,
            final_response.fallback_reason.as_deref(),
        ) {
            return Err(build_failed_turn_result(
                Some(provider_meta),
                user_message,
                error,
                self.telemetry_builder
                    .failed_trace_after_tool(tool_result.status == "ok"),
                self.telemetry_builder
                    .tool_activities_after_result(&tool_call, &tool_result),
            ));
        }

        Ok(SyncToolTurnOutcome {
            assistant_message: final_response.output_text,
            provider_mode: final_response.provider_mode,
            fallback_reason: final_response.fallback_reason,
            token_usage: final_response.token_usage,
            trace_steps: self
                .telemetry_builder
                .completed_trace_with_tool(tool_result.status == "ok"),
            tool_activities: self
                .telemetry_builder
                .tool_activities_after_result(&tool_call, &tool_result),
        })
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
                    prepared.user_message,
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
            provider,
            tools,
            planning_request,
            ..
        } = prepared;
        let provider_meta = provider_event_meta(&provider);

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            return build_failed_turn_result(
                Some(&provider_meta),
                user_message,
                error,
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
            );
        }
        let (
            assistant_message,
            provider_mode,
            fallback_reason,
            token_usage,
            trace_steps,
            tool_activities,
        ) = if let Some(tool_call) = resolved_tool_call {
            match self.handle_sync_tool_turn(
                user_message.clone(),
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                tool_call,
            ) {
                Ok(outcome) => (
                    outcome.assistant_message,
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
                first_decision.provider_mode.clone(),
                first_decision.fallback_reason.clone(),
                first_decision.token_usage.clone(),
                self.telemetry_builder.completed_trace_without_tool(),
                Vec::new(),
            )
        };
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            &user_message,
            &assistant_message,
            provider.name(),
            &provider_mode,
            token_usage.as_ref(),
        );

        TurnResult {
            phase: "ready".to_string(),
            provider_requested_name: provider.requested_name().to_string(),
            provider_name: provider.name().to_string(),
            provider_protocol: provider.protocol_label().to_string(),
            provider_model: provider.model().to_string(),
            provider_mode,
            fallback_reason,
            input_tokens: persisted.input_tokens,
            output_tokens: persisted.output_tokens,
            total_tokens: persisted.total_tokens,
            first_token_latency_ms: None,
            user_message,
            assistant_message,
            trace_steps,
            tool_activities,
            session_summary: persisted.session_summary,
        }
    }

    pub fn start_turn_stream(&mut self, app: AppHandle, turn_id: String, input: TurnInput) {
        let prepared = match self.prepare_turn(&input, true) {
            Ok(prepared) => prepared,
            Err(error) => {
                emit_turn_failed(
                    &app,
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

        emit_stream_event(
            &app,
            "turn:started",
            turn_id.clone(),
            "started",
            Some("calling_model"),
            Some(prepared.user_message.clone()),
            Some(&prepared_provider_meta),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(self.telemetry_builder.start_trace_steps()),
            None,
            None,
        );

        let planned = match self.plan_turn(&prepared) {
            Ok(planned) => planned,
            Err(error) => {
                emit_turn_failed(
                    &app,
                    turn_id,
                    Some(prepared.provider.requested_name().to_string()),
                    Some(prepared.provider.name().to_string()),
                    Some(prepared.provider.protocol_label().to_string()),
                    Some(prepared.provider.model().to_string()),
                    self.telemetry_builder.failed_trace_before_tool(),
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
            provider,
            tools,
            planning_request,
            ..
        } = prepared;
        let provider_meta = provider_event_meta(&provider);

        if let Some(tool_call) = resolved_tool_call {
            self.handle_stream_tool_turn(
                &app,
                &turn_id,
                &input,
                &user_message,
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
            &app,
            "turn:trace",
            turn_id.clone(),
            "trace",
            Some("calling_model"),
            None,
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
        );

        let first_token_latency_ms =
            stream_text_chunks(&app, &turn_id, "calling_model", &first_decision.output_text, &turn_started_at);
        let completed_text = first_decision.output_text.clone();
        let completed_mode = first_decision.provider_mode.clone();
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            &user_message,
            &completed_text,
            provider.name(),
            &completed_mode,
            first_decision.token_usage.as_ref(),
        );

        emit_stream_event(
            &app,
            "turn:completed",
            turn_id,
            "completed",
            Some("ready"),
            Some(first_decision.output_text),
            Some(&provider_meta),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
            Some(persisted.input_tokens).flatten(),
            Some(persisted.output_tokens).flatten(),
            Some(persisted.total_tokens).flatten(),
            first_token_latency_ms,
            Some(self.telemetry_builder.completed_trace_without_tool()),
            Some(Vec::new()),
            Some(persisted.session_summary),
        );
    }

    fn resolve_provider(&self, input: &TurnInput) -> ProviderManager {
        ProviderManager::new(self.provider_resolver.resolve_provider_selection(
            input.provider_id.as_deref(),
            input.model_id.as_deref(),
        ))
    }

    fn resolve_tool_call(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        provider_tool_call: Option<ToolCall>,
    ) -> Option<ToolCall> {
        self.planner
            .select_tool_call(user_message, history, provider_tool_call)
    }
}
