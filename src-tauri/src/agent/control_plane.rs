use crate::agent::context::RetrievedContextState;
use crate::agent::execution_control::{
    ExecutionCheckpoint, ExecutionControlRegistry, StopTurnResponse,
};
use crate::agent::graph::{
    GraphDecision, GraphRun, GraphRunCheckpoint, GraphRunEvent, GraphRunPhase,
    GraphRunStopReason, GraphRunStore, GraphRunner, GraphTurnHandoff,
};
use crate::agent::planner::{DefaultGraphPlanner, GraphPlanner};
use crate::agent::runtime::{AgentRuntime, TurnInput, TurnResult, TurnStreamEvent};
use crate::agent::session::{SessionOverview, SessionSnapshot};
use crate::agent::turn_flow::TurnEventSink;
use serde::Serialize;
use std::sync::Mutex;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostHealthSnapshot {
    pub app_name: String,
    pub app_version: String,
    pub runtime: String,
    pub graph_engine: String,
    pub graph_contract_version: String,
}

#[derive(Clone)]
pub struct RunTurnCommand {
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct StartTurnStreamCommand {
    pub turn_id: String,
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct StartGraphRunCommand {
    pub run_id: Option<String>,
    pub goal: String,
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct StartGraphRunStreamCommand {
    pub turn_id: String,
    pub run_id: Option<String>,
    pub goal: String,
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct ContinueGraphRunCommand {
    pub run_id: String,
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct ContinueGraphRunStreamCommand {
    pub turn_id: String,
    pub run_id: String,
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct ResumeGraphRunCommand {
    pub run_id: String,
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct ResumeGraphRunStreamCommand {
    pub turn_id: String,
    pub run_id: String,
    pub input: TurnInput,
}

#[derive(Clone)]
pub struct StopTurnCommand {
    pub turn_id: String,
}

#[derive(Clone)]
pub struct StopGraphRunCommand {
    pub run_id: String,
}

#[derive(Clone, Default)]
pub struct ExecutionCheckpointQuery {
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct SessionSnapshotQuery {
    pub session_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct SessionRuntimeViewQuery {
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct RetrievedContextQuery {
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Clone)]
pub struct DeleteSessionCommand {
    pub session_id: String,
}

#[derive(Clone, Default)]
pub struct GraphRunQuery {
    pub run_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct GraphRunCheckpointQuery {
    pub run_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct HostInspectionQuery {
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub include_session: bool,
    pub include_retrieved: bool,
    pub include_sessions: bool,
    pub include_run: bool,
    pub include_runs: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInspectionSnapshot {
    pub surface: String,
    pub turn: Option<ExecutionCheckpoint>,
    pub session: Option<SessionSnapshot>,
    pub retrieved: Option<RetrievedContextState>,
    pub sessions: Option<Vec<SessionOverview>>,
    pub run: Option<GraphRun>,
    pub runs: Option<Vec<GraphRun>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeView {
    pub session: SessionSnapshot,
    pub retrieved: RetrievedContextState,
    pub checkpoint: Option<ExecutionCheckpoint>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunTurnResponse {
    pub run: GraphRun,
    pub handoff: GraphTurnHandoff,
    pub decision: GraphDecision,
    pub event: GraphRunEvent,
    pub turn_result: TurnResult,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunControlResponse {
    pub run: GraphRun,
    pub event: GraphRunEvent,
    pub turn_stop: Option<StopTurnResponse>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunStreamStartResponse {
    pub run: GraphRun,
    pub event: GraphRunEvent,
    pub turn_id: String,
}

#[derive(Clone)]
pub struct PreparedGraphRunStream {
    pub run_id: String,
    pub turn_id: String,
    pub input: TurnInput,
}

#[derive(Default)]
struct RecordedTurnTerminal {
    phase: Option<String>,
    assistant_message: Option<String>,
    provider_requested_name: Option<String>,
    provider_name: Option<String>,
    provider_protocol: Option<String>,
    provider_model: Option<String>,
    provider_source: Option<String>,
    provider_mode: Option<String>,
    fallback_reason: Option<String>,
    build_context_observation: Option<crate::agent::provider::BuildContextObservation>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    trace_steps: Option<Vec<crate::agent::telemetry::TurnTraceStep>>,
    tool_activities: Option<Vec<crate::agent::telemetry::TurnToolActivity>>,
    session_summary: Option<String>,
}

struct RecordingTurnEventSink<'a, S> {
    inner: &'a S,
    terminal: Mutex<RecordedTurnTerminal>,
}

impl<'a, S> RecordingTurnEventSink<'a, S> {
    fn new(inner: &'a S) -> Self {
        Self {
            inner,
            terminal: Mutex::new(RecordedTurnTerminal::default()),
        }
    }

    fn build_turn_result(
        &self,
        input: &TurnInput,
        fallback_session_summary: String,
    ) -> Option<TurnResult> {
        let terminal = self.terminal.lock().expect("recording sink lock poisoned");
        let phase = terminal.phase.clone()?;
        let user_message = input
            .display_message
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(input.message.as_str())
            .to_string();
        let assistant_message = terminal.assistant_message.clone().unwrap_or_default();

        Some(TurnResult {
            phase,
            provider_requested_name: terminal.provider_requested_name.clone().unwrap_or_default(),
            provider_name: terminal.provider_name.clone().unwrap_or_default(),
            provider_protocol: terminal.provider_protocol.clone().unwrap_or_default(),
            provider_model: terminal.provider_model.clone().unwrap_or_default(),
            provider_source: terminal.provider_source.clone().unwrap_or_default(),
            provider_mode: terminal.provider_mode.clone().unwrap_or_default(),
            fallback_reason: terminal.fallback_reason.clone(),
            build_context_observation: terminal.build_context_observation.clone(),
            input_tokens: terminal.input_tokens,
            output_tokens: terminal.output_tokens,
            total_tokens: terminal.total_tokens,
            first_token_latency_ms: terminal.first_token_latency_ms,
            user_message,
            assistant_message: assistant_message.clone(),
            trace_steps: terminal.trace_steps.clone().unwrap_or_default(),
            tool_activities: terminal.tool_activities.clone().unwrap_or_default(),
            session_summary: terminal
                .session_summary
                .clone()
                .unwrap_or_else(|| fallback_session_summary.clone()),
        })
    }

    fn record_terminal_payload(&self, payload: &TurnStreamEvent) {
        let mut terminal = self.terminal.lock().expect("recording sink lock poisoned");
        terminal.phase = payload.phase.clone();
        terminal.assistant_message = payload.text.clone();
        terminal.provider_requested_name = payload.provider_requested_name.clone();
        terminal.provider_name = payload.provider_name.clone();
        terminal.provider_protocol = payload.provider_protocol.clone();
        terminal.provider_model = payload.provider_model.clone();
        terminal.provider_source = payload.provider_source.clone();
        terminal.provider_mode = payload.provider_mode.clone();
        terminal.fallback_reason = payload.fallback_reason.clone();
        terminal.build_context_observation = payload.build_context_observation.clone();
        terminal.input_tokens = payload.input_tokens;
        terminal.output_tokens = payload.output_tokens;
        terminal.total_tokens = payload.total_tokens;
        terminal.first_token_latency_ms = payload.first_token_latency_ms;
        terminal.trace_steps = payload.trace_steps.clone();
        terminal.tool_activities = payload.tool_activities.clone();
        terminal.session_summary = payload.session_summary.clone();
    }
}

impl<'a, S: TurnEventSink> TurnEventSink for RecordingTurnEventSink<'a, S> {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.inner.emit(name, payload.clone());
        if matches!(name, "turn:completed" | "turn:failed" | "turn:cancelled") {
            self.record_terminal_payload(&payload);
        }
    }
}

pub struct HostControlPlane {
    runtime: Mutex<AgentRuntime>,
    execution_control: ExecutionControlRegistry,
    graph_runs: Mutex<GraphRunStore>,
    graph_runner: GraphRunner,
    graph_planner: Box<dyn GraphPlanner>,
}

impl HostControlPlane {
    pub fn new() -> Self {
        Self {
            runtime: Mutex::new(AgentRuntime::new()),
            execution_control: ExecutionControlRegistry::new(),
            graph_runs: Mutex::new(default_graph_run_store()),
            graph_runner: GraphRunner::new(),
            graph_planner: Box::new(DefaultGraphPlanner),
        }
    }

    #[cfg(test)]
    fn with_runtime(runtime: AgentRuntime) -> Self {
        Self {
            runtime: Mutex::new(runtime),
            execution_control: ExecutionControlRegistry::new(),
            graph_runs: Mutex::new(default_graph_run_store()),
            graph_runner: GraphRunner::new(),
            graph_planner: Box::new(DefaultGraphPlanner),
        }
    }

    pub fn health_snapshot(&self) -> HostHealthSnapshot {
        let runtime = self.runtime.lock().expect("runtime lock poisoned");

        HostHealthSnapshot {
            app_name: "Pony Agent".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            runtime: runtime.name().to_string(),
            graph_engine: runtime.graph_engine().to_string(),
            graph_contract_version: runtime.graph_contract_version().to_string(),
        }
    }

    pub fn run_turn(&self, command: RunTurnCommand) -> TurnResult {
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        runtime.run_turn(command.input)
    }

    pub fn start_graph_run(
        &self,
        command: StartGraphRunCommand,
    ) -> Result<GraphRunTurnResponse, String> {
        let goal = command.goal.trim();
        if goal.is_empty() {
            return Err("Goal is empty.".to_string());
        }

        let run_id = command
            .run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(next_graph_run_id);

        let run = {
            let runtime = self.runtime.lock().expect("runtime lock poisoned");
            runtime.start_graph_run(
                run_id.clone(),
                goal.to_string(),
                command.input.session_id.as_deref(),
            )
        };

        {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            if graph_runs.load_run(&run_id).is_some() {
                return Err(format!("Graph run `{run_id}` already exists."));
            }
            self.graph_runner.start_run(&mut graph_runs, run);
        }

        self.advance_graph_run(run_id, command.input)
    }

    pub fn continue_graph_run(
        &self,
        command: ContinueGraphRunCommand,
    ) -> Result<GraphRunTurnResponse, String> {
        let input = {
            let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            let run = graph_runs
                .load_run(&command.run_id)
                .ok_or_else(|| format!("Graph run `{}` not found.", command.run_id))?;
            if matches!(
                run.phase,
                crate::agent::graph::GraphRunPhase::Completed
                    | crate::agent::graph::GraphRunPhase::Failed
                    | crate::agent::graph::GraphRunPhase::Cancelled
            ) {
                return Err(format!(
                    "Graph run `{}` is already terminal and cannot continue.",
                    command.run_id
                ));
            }

            let mut input = command.input;
            match (run.session_id.as_deref(), input.session_id.as_deref()) {
                (Some(expected), Some(actual)) if expected != actual => {
                    return Err(format!(
                        "Graph run `{}` is bound to session `{expected}`, but received `{actual}`.",
                        command.run_id
                    ));
                }
                (Some(expected), None) => {
                    input.session_id = Some(expected.to_string());
                }
                _ => {}
            }
            input
        };

        self.advance_graph_run(command.run_id, input)
    }

    pub fn resume_graph_run(
        &self,
        command: ResumeGraphRunCommand,
    ) -> Result<GraphRunTurnResponse, String> {
        let input = {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            let lifecycle = self
                .graph_runner
                .resume_run(
                    &mut graph_runs,
                    &command.run_id,
                    "Graph run resumed and ready for the next turn.",
                )
                .ok_or_else(|| format!("Graph run `{}` is not resumable.", command.run_id))?;
            let mut input = command.input;
            match (
                lifecycle.run.session_id.as_deref(),
                input.session_id.as_deref(),
            ) {
                (Some(expected), Some(actual)) if expected != actual => {
                    return Err(format!(
                        "Graph run `{}` is bound to session `{expected}`, but received `{actual}`.",
                        command.run_id
                    ));
                }
                (Some(expected), None) => {
                    input.session_id = Some(expected.to_string());
                }
                _ => {}
            }
            input
        };

        self.advance_graph_run(command.run_id, input)
    }

    pub fn prepare_start_graph_run_stream(
        &self,
        command: StartGraphRunStreamCommand,
    ) -> Result<(GraphRunStreamStartResponse, PreparedGraphRunStream), String> {
        let goal = command.goal.trim();
        if goal.is_empty() {
            return Err("Goal is empty.".to_string());
        }

        let run_id = command
            .run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(next_graph_run_id);

        let run = {
            let runtime = self.runtime.lock().expect("runtime lock poisoned");
            runtime.start_graph_run(
                run_id.clone(),
                goal.to_string(),
                command.input.session_id.as_deref(),
            )
        };

        {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            if graph_runs.load_run(&run_id).is_some() {
                return Err(format!("Graph run `{run_id}` already exists."));
            }
            self.graph_runner.start_run(&mut graph_runs, run);
        }

        self.begin_graph_run_stream(run_id, command.turn_id, command.input)
    }

    pub fn prepare_continue_graph_run_stream(
        &self,
        command: ContinueGraphRunStreamCommand,
    ) -> Result<(GraphRunStreamStartResponse, PreparedGraphRunStream), String> {
        let input = {
            let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            let run = graph_runs
                .load_run(&command.run_id)
                .ok_or_else(|| format!("Graph run `{}` not found.", command.run_id))?;
            if matches!(
                run.phase,
                crate::agent::graph::GraphRunPhase::Completed
                    | crate::agent::graph::GraphRunPhase::Failed
                    | crate::agent::graph::GraphRunPhase::Cancelled
            ) {
                return Err(format!(
                    "Graph run `{}` is already terminal and cannot continue.",
                    command.run_id
                ));
            }
            if run.active_turn_id.is_some() {
                return Err(format!(
                    "Graph run `{}` already has an active turn and cannot continue.",
                    command.run_id
                ));
            }

            let mut input = command.input;
            match (run.session_id.as_deref(), input.session_id.as_deref()) {
                (Some(expected), Some(actual)) if expected != actual => {
                    return Err(format!(
                        "Graph run `{}` is bound to session `{expected}`, but received `{actual}`.",
                        command.run_id
                    ));
                }
                (Some(expected), None) => {
                    input.session_id = Some(expected.to_string());
                }
                _ => {}
            }
            input
        };

        self.begin_graph_run_stream(command.run_id, command.turn_id, input)
    }

    pub fn prepare_resume_graph_run_stream(
        &self,
        command: ResumeGraphRunStreamCommand,
    ) -> Result<(GraphRunStreamStartResponse, PreparedGraphRunStream), String> {
        let input = {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            let lifecycle = self
                .graph_runner
                .resume_run(
                    &mut graph_runs,
                    &command.run_id,
                    "Graph run resumed and ready for the next turn.",
                )
                .ok_or_else(|| format!("Graph run `{}` is not resumable.", command.run_id))?;
            let mut input = command.input;
            match (
                lifecycle.run.session_id.as_deref(),
                input.session_id.as_deref(),
            ) {
                (Some(expected), Some(actual)) if expected != actual => {
                    return Err(format!(
                        "Graph run `{}` is bound to session `{expected}`, but received `{actual}`.",
                        command.run_id
                    ));
                }
                (Some(expected), None) => {
                    input.session_id = Some(expected.to_string());
                }
                _ => {}
            }
            input
        };

        self.begin_graph_run_stream(command.run_id, command.turn_id, input)
    }

    pub fn execute_graph_run_stream<S: TurnEventSink>(
        &self,
        sink: &S,
        prepared: PreparedGraphRunStream,
    ) -> Result<GraphRunTurnResponse, String> {
        let (turn_result, handoff, decision) = {
            let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
            let recording_sink = RecordingTurnEventSink::new(sink);
            runtime.start_turn_stream_with_control(
                &recording_sink,
                &self.execution_control,
                prepared.turn_id.clone(),
                prepared.input.clone(),
            );
            let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: Some(prepared.turn_id.clone()),
                session_id: None,
            });
            let run = {
                let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
                graph_runs.load_run(&prepared.run_id).ok_or_else(|| {
                    format!(
                        "Graph run `{}` failed to load planner state.",
                        prepared.run_id
                    )
                })?
            };
            let session_summary_fallback = runtime
                .inspect_retrieved_context(
                    prepared.input.session_id.as_deref(),
                    Some(&run),
                    checkpoint.as_ref(),
                )
                .session_context
                .summary;
            let turn_result = recording_sink
                .build_turn_result(&prepared.input, session_summary_fallback)
                .ok_or_else(|| {
                    format!(
                        "Graph run `{}` finished without a terminal turn event.",
                        prepared.run_id
                    )
                })?;
            let handoff = runtime.build_graph_turn_handoff(
                Some(&run),
                Some(&prepared.turn_id),
                prepared.input.session_id.as_deref(),
                &turn_result,
                checkpoint.as_ref(),
            );
            let decision = runtime.decide_graph_after_turn_with_planner(
                &run,
                Some(&prepared.turn_id),
                prepared.input.session_id.as_deref(),
                &turn_result,
                checkpoint.as_ref(),
                self.graph_planner.as_ref(),
            );
            (turn_result, handoff, decision)
        };

        let advance = {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            self.graph_runner
                .apply_turn_result(&mut graph_runs, &prepared.run_id, handoff, decision)
                .ok_or_else(|| {
                    format!(
                        "Graph run `{}` failed to record streamed turn result.",
                        prepared.run_id
                    )
                })?
        };

        Ok(GraphRunTurnResponse {
            run: advance.run,
            handoff: advance.handoff,
            decision: advance.decision,
            event: advance.event,
            turn_result,
        })
    }

    pub fn start_turn_stream<S: TurnEventSink>(&self, sink: &S, command: StartTurnStreamCommand) {
        self.execution_control
            .register_turn(&command.turn_id, command.input.session_id.as_deref());

        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        runtime.start_turn_stream_with_control(
            sink,
            &self.execution_control,
            command.turn_id,
            command.input,
        );
    }

    pub fn stop_turn(&self, command: StopTurnCommand) -> StopTurnResponse {
        self.execution_control.request_stop(&command.turn_id)
    }

    pub fn stop_graph_run(
        &self,
        command: StopGraphRunCommand,
    ) -> Result<GraphRunControlResponse, String> {
        let turn_stop = {
            let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            let run = graph_runs
                .load_run(&command.run_id)
                .ok_or_else(|| format!("Graph run `{}` not found.", command.run_id))?;
            if matches!(
                run.phase,
                crate::agent::graph::GraphRunPhase::Completed
                    | crate::agent::graph::GraphRunPhase::Failed
                    | crate::agent::graph::GraphRunPhase::Cancelled
            ) {
                return Err(format!(
                    "Graph run `{}` is already terminal and cannot stop.",
                    command.run_id
                ));
            }
            run.active_turn_id.as_deref().and_then(|turn_id| {
                self.load_execution_checkpoint(ExecutionCheckpointQuery {
                    turn_id: Some(turn_id.to_string()),
                    session_id: None,
                })
                .map(|_| self.execution_control.request_stop(turn_id))
            })
        };

        let lifecycle = {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            self.graph_runner
                .request_stop(
                    &mut graph_runs,
                    &command.run_id,
                    GraphRunStopReason::UserStop,
                    "Graph run stopped and waiting to resume.",
                )
                .ok_or_else(|| format!("Graph run `{}` cannot be stopped.", command.run_id))?
        };

        Ok(GraphRunControlResponse {
            run: lifecycle.run,
            event: lifecycle.event,
            turn_stop,
        })
    }

    pub fn load_execution_checkpoint(
        &self,
        query: ExecutionCheckpointQuery,
    ) -> Option<ExecutionCheckpoint> {
        self.execution_control
            .load_checkpoint(query.turn_id.as_deref(), query.session_id.as_deref())
    }

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        let runtime = self.runtime.lock().expect("runtime lock poisoned");
        runtime.list_sessions()
    }

    pub fn load_graph_run(&self, query: GraphRunQuery) -> Option<GraphRun> {
        let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
        let run_id = query.run_id?;
        graph_runs.load_run(&run_id)
    }

    pub fn list_graph_runs(&self) -> Vec<GraphRun> {
        let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
        graph_runs.list_runs()
    }

    fn load_active_graph_run_for_session(&self, session_id: Option<&str>) -> Option<GraphRun> {
        let session_id = session_id.map(str::trim).filter(|session_id| !session_id.is_empty())?;
        let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
        graph_runs.list_runs().into_iter().find(|run| {
            run.session_id.as_deref() == Some(session_id)
                && !matches!(
                    run.phase,
                    GraphRunPhase::Completed | GraphRunPhase::Failed | GraphRunPhase::Cancelled
                )
        })
    }

    fn resolve_graph_run_for_retrieval(
        &self,
        run_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Option<GraphRun> {
        run_id
            .and_then(|run_id| {
                self.load_graph_run(GraphRunQuery {
                    run_id: Some(run_id.to_string()),
                })
            })
            .or_else(|| self.load_active_graph_run_for_session(session_id))
    }

    pub fn load_graph_run_checkpoint(
        &self,
        query: GraphRunCheckpointQuery,
    ) -> Option<GraphRunCheckpoint> {
        let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
        let run_id = query.run_id?;
        let run = graph_runs.load_run(&run_id)?;
        Some(self.graph_runner.build_checkpoint(&run))
    }

    pub fn load_session_snapshot(&self, query: SessionSnapshotQuery) -> SessionSnapshot {
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        runtime.load_session_snapshot(query.session_id.as_deref())
    }

    pub fn load_session_runtime_view(&self, query: SessionRuntimeViewQuery) -> SessionRuntimeView {
        let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: query.turn_id,
            session_id: query.session_id.clone(),
        });
        let resolved_session_id = query.session_id.or_else(|| {
            checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.session_id.clone())
        });
        let run = self
            .resolve_graph_run_for_retrieval(query.run_id.as_deref(), resolved_session_id.as_deref());
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let session = runtime.load_session_snapshot(resolved_session_id.as_deref());
        let retrieved =
            runtime.inspect_retrieved_context(resolved_session_id.as_deref(), run.as_ref(), checkpoint.as_ref());

        SessionRuntimeView {
            session,
            retrieved,
            checkpoint,
        }
    }

    pub fn load_retrieved_context(&self, query: RetrievedContextQuery) -> RetrievedContextState {
        let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: query.turn_id,
            session_id: query.session_id.clone(),
        });
        let resolved_session_id = query.session_id.or_else(|| {
            checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.session_id.clone())
        });
        let run = self
            .resolve_graph_run_for_retrieval(query.run_id.as_deref(), resolved_session_id.as_deref());
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        runtime.inspect_retrieved_context(
            resolved_session_id.as_deref(),
            run.as_ref(),
            checkpoint.as_ref(),
        )
    }

    pub fn delete_session(&self, command: DeleteSessionCommand) -> Vec<SessionOverview> {
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        runtime.remove_session(&command.session_id)
    }

    pub fn inspect(&self, query: HostInspectionQuery) -> HostInspectionSnapshot {
        let turn = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: query.turn_id.clone(),
            session_id: query.session_id.clone(),
        });
        let resolved_session_id = query.session_id.clone().or_else(|| {
            turn.as_ref()
                .and_then(|checkpoint| checkpoint.session_id.clone())
        });
        let run = (query.include_run || query.include_retrieved)
            .then(|| {
                self.resolve_graph_run_for_retrieval(
                    query.run_id.as_deref(),
                    resolved_session_id.as_deref(),
                )
            })
            .flatten();
        let retrieved = if query.include_retrieved {
            let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
            Some(runtime.inspect_retrieved_context(
                resolved_session_id.as_deref(),
                run.as_ref(),
                turn.as_ref(),
            ))
        } else {
            None
        };

        HostInspectionSnapshot {
            surface: "host-control-plane/v1".to_string(),
            turn,
            session: query.include_session.then(|| {
                self.load_session_snapshot(SessionSnapshotQuery {
                    session_id: resolved_session_id,
                })
            }),
            retrieved,
            sessions: query.include_sessions.then(|| self.list_sessions()),
            run: query.include_run.then_some(run).flatten(),
            runs: query.include_runs.then(|| self.list_graph_runs()),
        }
    }

    fn begin_graph_run_stream(
        &self,
        run_id: String,
        turn_id: String,
        input: TurnInput,
    ) -> Result<(GraphRunStreamStartResponse, PreparedGraphRunStream), String> {
        self.execution_control
            .register_turn(&turn_id, input.session_id.as_deref());

        let lifecycle = {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            self.graph_runner
                .begin_turn(
                    &mut graph_runs,
                    &run_id,
                    &turn_id,
                    input.session_id.as_deref(),
                )
                .ok_or_else(|| format!("Graph run `{run_id}` cannot accept a new turn."))?
        };

        let response = GraphRunStreamStartResponse {
            run: lifecycle.run,
            event: lifecycle.event,
            turn_id: turn_id.clone(),
        };
        let prepared = PreparedGraphRunStream {
            run_id,
            turn_id,
            input,
        };

        Ok((response, prepared))
    }

    fn advance_graph_run(
        &self,
        run_id: String,
        input: TurnInput,
    ) -> Result<GraphRunTurnResponse, String> {
        let turn_id = {
            let next_turn_id = {
                let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
                let run = graph_runs
                    .load_run(&run_id)
                    .ok_or_else(|| format!("Graph run `{run_id}` cannot accept a new turn."))?;
                format!("{}-turn-{}", run.id, run.steps.len() + 1)
            };
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            let lifecycle = self
                .graph_runner
                .begin_turn(
                    &mut graph_runs,
                    &run_id,
                    &next_turn_id,
                    input.session_id.as_deref(),
                )
                .ok_or_else(|| format!("Graph run `{run_id}` cannot accept a new turn."))?;
            lifecycle.run.active_turn_id.clone().unwrap_or(next_turn_id)
        };

        let (turn_result, handoff, decision) = {
            let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
            let turn_result = runtime.run_turn(input.clone());
            let run = {
                let graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
                graph_runs
                    .load_run(&run_id)
                    .ok_or_else(|| format!("Graph run `{run_id}` failed to load planner state."))?
            };
            let handoff = runtime.build_graph_turn_handoff(
                Some(&run),
                Some(&turn_id),
                input.session_id.as_deref(),
                &turn_result,
                None,
            );
            let decision = runtime.decide_graph_after_turn_with_planner(
                &run,
                Some(&turn_id),
                input.session_id.as_deref(),
                &turn_result,
                None,
                self.graph_planner.as_ref(),
            );
            (turn_result, handoff, decision)
        };

        let advance = {
            let mut graph_runs = self.graph_runs.lock().expect("graph run lock poisoned");
            self.graph_runner
                .apply_turn_result(&mut graph_runs, &run_id, handoff, decision)
                .ok_or_else(|| format!("Graph run `{run_id}` failed to record turn result."))?
        };

        Ok(GraphRunTurnResponse {
            run: advance.run,
            handoff: advance.handoff,
            decision: advance.decision,
            event: advance.event,
            turn_result,
        })
    }
}

impl Default for HostControlPlane {
    fn default() -> Self {
        Self::new()
    }
}

fn next_graph_run_id() -> String {
    format!("run-{}", now_timestamp_ms())
}

fn default_graph_run_store() -> GraphRunStore {
    #[cfg(test)]
    {
        GraphRunStore::new()
    }

    #[cfg(not(test))]
    {
        GraphRunStore::persistent(crate::agent::graph::default_graph_run_store_path())
    }
}

fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{
        ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    };
    use crate::agent::context::DefaultTurnContextBuilder;
    use crate::agent::graph::{GraphDecisionKind, GraphRunEventKind, GraphRunPhase};
    use crate::agent::planner::LocalTurnPlanner;
    use crate::agent::provider::ProviderProtocol;
    use crate::agent::runtime::TurnStreamEvent;
    use crate::agent::session::SessionStore;
    use crate::agent::telemetry::DefaultTurnTelemetryBuilder;
    use crate::agent::tools::ToolRouter;
    use crate::agent::turn_flow::TurnEventSink;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct NoopSink;

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

    struct MockHttpResponse {
        content_type: &'static str,
        body: String,
    }

    struct MockHttpServer {
        base_url: String,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl MockHttpServer {
        fn start(responses: Vec<MockHttpResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let address = listener.local_addr().expect("mock server addr");
            let pending = Arc::new(Mutex::new(responses));
            let pending_for_thread = Arc::clone(&pending);
            let handle = thread::spawn(move || loop {
                let response = {
                    let mut responses = pending_for_thread.lock().expect("mock response lock");
                    if responses.is_empty() {
                        break;
                    }
                    responses.remove(0)
                };
                let (mut stream, _) = listener.accept().expect("accept mock request");
                let _ = read_http_request_body(&mut stream);
                write_http_response(&mut stream, response);
            });

            Self {
                base_url: format!("http://{}/v1", address),
                handle: Some(handle),
            }
        }

        fn finish(mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join().expect("join mock server");
            }
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
                if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = Some(index + 4);
                    let headers = String::from_utf8_lossy(&buffer[..index + 4]);
                    for line in headers.lines() {
                        let lowercase = line.to_ascii_lowercase();
                        if let Some(value) = lowercase.strip_prefix("content-length:") {
                            content_length = value.trim().parse::<usize>().unwrap_or(0);
                        }
                    }
                }
            }

            if let Some(header_end) = header_end {
                let body_len = buffer.len().saturating_sub(header_end);
                if body_len >= content_length {
                    break;
                }
            }
        }

        let body_start = header_end.unwrap_or(buffer.len());
        String::from_utf8_lossy(&buffer[body_start..]).to_string()
    }

    fn write_http_response(stream: &mut TcpStream, response: MockHttpResponse) {
        let body_len = response.body.as_bytes().len();
        let payload = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.content_type, body_len, response.body
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
            ],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2
            }
        }))
    }

    fn test_provider_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "test-openai".to_string(),
            provider_name: "test-openai".to_string(),
            protocol: ProviderProtocol::OpenAi,
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

    fn build_test_control_plane(
        responses: Vec<MockHttpResponse>,
    ) -> (HostControlPlane, MockHttpServer) {
        let server = MockHttpServer::start(responses);
        let runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver {
                selection: test_provider_selection(server.base_url.clone()),
            }),
            Box::new(ToolRouter::new()),
            Box::new(LocalTurnPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        (HostControlPlane::with_runtime(runtime), server)
    }

    impl TurnEventSink for NoopSink {
        fn emit(&self, _name: &str, _payload: TurnStreamEvent) {}
    }

    #[test]
    fn recording_turn_event_sink_uses_fallback_summary_when_terminal_event_has_none() {
        let sink = NoopSink;
        let recording_sink = RecordingTurnEventSink::new(&sink);

        recording_sink.emit(
            "turn:completed",
            TurnStreamEvent {
                turn_id: "turn-fallback".to_string(),
                kind: "completed".to_string(),
                phase: Some("ready".to_string()),
                text: Some("streamed reply".to_string()),
                reasoning_content: None,
                error: None,
                provider_requested_name: Some("test-openai".to_string()),
                provider_name: Some("test-openai".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                fallback_reason: None,
                build_context_observation: None,
                input_tokens: Some(21),
                output_tokens: Some(34),
                total_tokens: Some(55),
                first_token_latency_ms: Some(180),
                trace_steps: Some(Vec::new()),
                tool_activities: Some(Vec::new()),
                session_summary: None,
            },
        );

        let result = recording_sink
            .build_turn_result(
                &TurnInput {
                    message: "continue".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("session-fallback".to_string()),
                    history: Vec::new(),
                    images: Vec::new(),
                },
                "retrieval summary fallback".to_string(),
            )
            .expect("turn result should be reconstructed");

        assert_eq!(result.session_summary, "retrieval summary fallback");
        assert_eq!(result.assistant_message, "streamed reply");
        assert_eq!(result.phase, "ready");
    }

    #[test]
    fn inspection_can_join_turn_and_session_views() {
        let (control_plane, server) = build_test_control_plane(vec![json_completion("inspected")]);
        control_plane
            .execution_control
            .register_turn("turn-inspect", Some("session-inspect"));

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "check Cargo.toml".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-inspect".to_string()),
                history: Vec::new(),
                images: Vec::new(),
            });
        }

        let snapshot = control_plane.inspect(HostInspectionQuery {
            turn_id: Some("turn-inspect".to_string()),
            session_id: None,
            run_id: None,
            include_session: true,
            include_retrieved: true,
            include_sessions: false,
            include_run: false,
            include_runs: false,
        });

        assert_eq!(snapshot.surface, "host-control-plane/v1");
        assert_eq!(
            snapshot
                .turn
                .as_ref()
                .map(|checkpoint| checkpoint.turn_id.as_str()),
            Some("turn-inspect")
        );
        assert_eq!(
            snapshot
                .session
                .as_ref()
                .map(|session| session.conversation_id.as_str()),
            Some("session-inspect")
        );
        assert_eq!(
            snapshot
                .retrieved
                .as_ref()
                .map(|retrieved| retrieved.session_context.conversation_id.as_str()),
            Some("session-inspect")
        );
        assert!(snapshot.sessions.is_none());
        server.finish();
    }

    #[test]
    fn session_snapshot_queries_flow_through_control_plane() {
        let control_plane = HostControlPlane::new();

        let snapshot = control_plane.load_session_snapshot(SessionSnapshotQuery {
            session_id: Some("alpha".to_string()),
        });

        assert_eq!(snapshot.conversation_id, "alpha");
        assert_eq!(
            control_plane
                .list_sessions()
                .into_iter()
                .filter(|session| session.conversation_id == "alpha")
                .count(),
            0
        );
    }

    #[test]
    fn session_runtime_view_queries_flow_through_control_plane() {
        let (control_plane, server) = build_test_control_plane(vec![json_completion("runtime-view")]);

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请继续推进 PA-018。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-runtime-view".to_string()),
                history: Vec::new(),
                images: Vec::new(),
            });
        }

        let view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("session-runtime-view".to_string()),
            ..SessionRuntimeViewQuery::default()
        });

        assert_eq!(view.session.conversation_id, "session-runtime-view");
        assert_eq!(
            view.retrieved.session_context.conversation_id,
            "session-runtime-view"
        );
        assert!(view.checkpoint.is_none());
        server.finish();
    }

    #[test]
    fn retrieved_context_queries_flow_through_control_plane() {
        let (control_plane, server) = build_test_control_plane(vec![json_completion("retrieved")]);

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请记住这个项目优先推进 PA-018。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-retrieved".to_string()),
                history: Vec::new(),
                images: Vec::new(),
            });
        }

        let retrieved = control_plane.load_retrieved_context(RetrievedContextQuery {
            session_id: Some("session-retrieved".to_string()),
            ..RetrievedContextQuery::default()
        });

        assert_eq!(
            retrieved.session_context.conversation_id,
            "session-retrieved"
        );
        assert_eq!(retrieved.long_term_memory.status, "available");
        assert!(retrieved
            .long_term_memory
            .entries
            .iter()
            .any(|entry| entry.kind == "user_memory.explicit_note"));
        server.finish();
    }

    #[test]
    fn retrieved_context_can_infer_active_graph_run_from_session_id() {
        let (control_plane, server) =
            build_test_control_plane(vec![json_completion("session-aware retrieval")]);
        let started = control_plane
            .start_graph_run(StartGraphRunCommand {
                run_id: Some("run-session-aware".to_string()),
                goal: "resume by retrieval boundary".to_string(),
                input: TurnInput {
                    message: "continue the active session run".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("session-aware".to_string()),
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph run should start");

        let retrieved = control_plane.load_retrieved_context(RetrievedContextQuery {
            session_id: Some("session-aware".to_string()),
            ..RetrievedContextQuery::default()
        });

        assert_eq!(started.run.id, "run-session-aware");
        assert_eq!(retrieved.run_state.run_id.as_deref(), Some("run-session-aware"));
        assert_eq!(retrieved.run_state.phase.as_deref(), Some("waiting_user"));
        server.finish();
    }

    #[test]
    fn stop_turn_and_checkpoint_queries_share_same_registry_surface() {
        let control_plane = HostControlPlane::new();
        let sink = NoopSink;

        control_plane.start_turn_stream(
            &sink,
            StartTurnStreamCommand {
                turn_id: "turn-stream".to_string(),
                input: TurnInput {
                    message: "   ".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("stream-session".to_string()),
                    history: Vec::new(),
                    images: Vec::new(),
                },
            },
        );

        let checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: Some("turn-stream".to_string()),
                session_id: None,
            })
            .expect("checkpoint should exist");
        let stop = control_plane.stop_turn(StopTurnCommand {
            turn_id: "turn-stream".to_string(),
        });

        assert_eq!(checkpoint.turn_id, "turn-stream");
        assert_eq!(checkpoint.session_id.as_deref(), Some("stream-session"));
        assert!(stop.accepted);
        assert_eq!(stop.state, "running");
    }

    #[test]
    fn graph_run_can_start_and_wait_for_next_user_turn() {
        let (control_plane, server) =
            build_test_control_plane(vec![json_completion("first response")]);
        let response = control_plane
            .start_graph_run(StartGraphRunCommand {
                run_id: Some("run-alpha".to_string()),
                goal: "audit provider config and continue".to_string(),
                input: TurnInput {
                    message: "run the first streamed turn".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("graph-session".to_string()),
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph run should start");

        assert_eq!(response.run.id, "run-alpha");
        assert_eq!(response.run.steps.len(), 1);
        assert_eq!(response.run.phase, GraphRunPhase::Ready);
        assert_eq!(response.event.kind, GraphRunEventKind::Updated);
        assert_eq!(response.decision.kind, GraphDecisionKind::Continue);
        assert_eq!(
            response.handoff.turn_id.as_deref(),
            Some("run-alpha-turn-1")
        );
        server.finish();
    }

    #[test]
    fn graph_run_can_continue_across_multiple_turns() {
        let (control_plane, server) = build_test_control_plane(vec![
            json_completion("first response"),
            json_completion("second response"),
        ]);
        let first = control_plane
            .start_graph_run(StartGraphRunCommand {
                run_id: Some("run-beta".to_string()),
                goal: "review config step by step".to_string(),
                input: TurnInput {
                    message: "what is tauri.conf.json?".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("graph-continue".to_string()),
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph run should start");
        let second = control_plane
            .continue_graph_run(ContinueGraphRunCommand {
                run_id: "run-beta".to_string(),
                input: TurnInput {
                    message: "keep reading the fourth line".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph run should continue");

        assert_eq!(first.run.steps.len(), 1);
        assert_eq!(second.run.steps.len(), 2);
        assert_eq!(second.run.session_id.as_deref(), Some("graph-continue"));
        assert_eq!(second.handoff.turn_id.as_deref(), Some("run-beta-turn-2"));
        assert_eq!(first.run.phase, GraphRunPhase::Ready);
        assert_eq!(second.run.phase, GraphRunPhase::Ready);
        server.finish();
    }

    #[test]
    fn graph_run_can_stop_resume_and_expose_checkpoint() {
        let (control_plane, server) = build_test_control_plane(vec![
            json_completion("first response"),
            json_completion("second response"),
        ]);
        let _ = control_plane
            .start_graph_run(StartGraphRunCommand {
                run_id: Some("run-pause".to_string()),
                goal: "pause then resume".to_string(),
                input: TurnInput {
                    message: "execute the first turn".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("graph-pause".to_string()),
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph run should start");

        let stopped = control_plane
            .stop_graph_run(StopGraphRunCommand {
                run_id: "run-pause".to_string(),
            })
            .expect("graph run should stop");
        assert_eq!(stopped.run.phase, GraphRunPhase::Paused);
        assert!(stopped.turn_stop.is_none());

        let checkpoint = control_plane
            .load_graph_run_checkpoint(GraphRunCheckpointQuery {
                run_id: Some("run-pause".to_string()),
            })
            .expect("checkpoint should exist");
        assert_eq!(checkpoint.phase, GraphRunPhase::Paused);
        assert!(checkpoint.resumable);

        let resumed = control_plane
            .resume_graph_run(ResumeGraphRunCommand {
                run_id: "run-pause".to_string(),
                input: TurnInput {
                    message: "continue with the second turn".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph run should resume");
        assert_eq!(resumed.run.phase, GraphRunPhase::WaitingUser);
        assert_eq!(resumed.run.resume_count, 1);
        assert_eq!(resumed.run.steps.len(), 2);
        server.finish();
    }

    #[test]
    fn graph_run_stream_can_start_continue_and_resume() {
        let (control_plane, server) = build_test_control_plane(vec![
            json_completion("stream response one"),
            json_completion("stream response two"),
            json_completion("stream response three"),
        ]);

        let (started, prepared_start) = control_plane
            .prepare_start_graph_run_stream(StartGraphRunStreamCommand {
                turn_id: "run-stream-turn-1".to_string(),
                run_id: Some("run-stream".to_string()),
                goal: "stream config review".to_string(),
                input: TurnInput {
                    message: "start the streamed run".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: Some("graph-stream".to_string()),
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph stream run should prepare");
        assert_eq!(started.run.phase, GraphRunPhase::Running);
        assert_eq!(started.turn_id, "run-stream-turn-1");

        let first = control_plane
            .execute_graph_run_stream(&NoopSink, prepared_start)
            .expect("graph stream run should execute");
        assert_eq!(first.run.phase, GraphRunPhase::WaitingUser);
        assert_eq!(first.run.steps.len(), 1);
        assert_eq!(first.handoff.turn_id.as_deref(), Some("run-stream-turn-1"));

        let (continued, prepared_continue) = control_plane
            .prepare_continue_graph_run_stream(ContinueGraphRunStreamCommand {
                turn_id: "run-stream-turn-2".to_string(),
                run_id: "run-stream".to_string(),
                input: TurnInput {
                    message: "continue with the next streamed turn".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph stream run should continue");
        assert_eq!(continued.run.phase, GraphRunPhase::Running);
        assert_eq!(continued.turn_id, "run-stream-turn-2");

        let second = control_plane
            .execute_graph_run_stream(&NoopSink, prepared_continue)
            .expect("graph stream continue should execute");
        assert_eq!(second.run.steps.len(), 2);
        assert_eq!(second.handoff.turn_id.as_deref(), Some("run-stream-turn-2"));

        let stopped = control_plane
            .stop_graph_run(StopGraphRunCommand {
                run_id: "run-stream".to_string(),
            })
            .expect("graph run should stop");
        assert_eq!(stopped.run.phase, GraphRunPhase::Paused);

        let (resumed, prepared_resume) = control_plane
            .prepare_resume_graph_run_stream(ResumeGraphRunStreamCommand {
                turn_id: "run-stream-turn-3".to_string(),
                run_id: "run-stream".to_string(),
                input: TurnInput {
                    message: "resume after the pause".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    session_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                },
            })
            .expect("graph stream run should resume");
        assert_eq!(resumed.run.phase, GraphRunPhase::Running);
        assert_eq!(resumed.turn_id, "run-stream-turn-3");

        let third = control_plane
            .execute_graph_run_stream(&NoopSink, prepared_resume)
            .expect("graph stream resume should execute");
        assert_eq!(third.run.steps.len(), 3);
        assert_eq!(third.run.resume_count, 1);
        assert_eq!(third.handoff.turn_id.as_deref(), Some("run-stream-turn-3"));
        server.finish();
    }

    #[test]
    fn inspection_can_include_graph_run_views() {
        let (control_plane, server) =
            build_test_control_plane(vec![json_completion("summary response")]);
        let _ = control_plane.start_graph_run(StartGraphRunCommand {
            run_id: Some("run-gamma".to_string()),
            goal: "summarize recent conversation".to_string(),
            input: TurnInput {
                message: "summarize the current project status".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("graph-inspect".to_string()),
                history: Vec::new(),
                images: Vec::new(),
            },
        });

        let snapshot = control_plane.inspect(HostInspectionQuery {
            turn_id: None,
            session_id: None,
            run_id: Some("run-gamma".to_string()),
            include_session: false,
            include_retrieved: true,
            include_sessions: false,
            include_run: true,
            include_runs: true,
        });

        assert_eq!(
            snapshot.run.as_ref().map(|run| run.id.as_str()),
            Some("run-gamma")
        );
        assert_eq!(
            snapshot
                .retrieved
                .as_ref()
                .and_then(|retrieved| retrieved.run_state.run_id.as_deref()),
            Some("run-gamma")
        );
        assert_eq!(snapshot.runs.as_ref().map(|runs| runs.len()), Some(1));
        server.finish();
    }

    #[test]
    fn inspection_can_infer_session_run_without_explicit_run_id() {
        let (control_plane, server) =
            build_test_control_plane(vec![json_completion("summary response")]);
        let _ = control_plane.start_graph_run(StartGraphRunCommand {
            run_id: Some("run-delta".to_string()),
            goal: "infer graph run from session".to_string(),
            input: TurnInput {
                message: "summarize the inferred session run".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("graph-infer".to_string()),
                history: Vec::new(),
                images: Vec::new(),
            },
        });

        let snapshot = control_plane.inspect(HostInspectionQuery {
            turn_id: None,
            session_id: Some("graph-infer".to_string()),
            run_id: None,
            include_session: false,
            include_retrieved: true,
            include_sessions: false,
            include_run: true,
            include_runs: false,
        });

        assert_eq!(
            snapshot.run.as_ref().map(|run| run.id.as_str()),
            Some("run-delta")
        );
        assert_eq!(
            snapshot
                .retrieved
                .as_ref()
                .and_then(|retrieved| retrieved.run_state.run_id.as_deref()),
            Some("run-delta")
        );
        server.finish();
    }
}
