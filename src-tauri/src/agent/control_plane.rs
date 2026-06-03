use crate::agent::capability_bridge::{
    CapabilityInvocationMode, CapabilityKind, CapabilityRegistry, CapabilitySourceKind,
    CapabilitySourceView, CapabilityView, McpSourceSnapshot,
};
use crate::agent::context::RetrievedContextState;
use crate::agent::execution_control::{
    ExecutionCheckpoint, ExecutionControlRegistry, StopTurnResponse,
};
use crate::agent::graph::{
    GraphDecision, GraphRun, GraphRunCheckpoint, GraphRunEvent, GraphRunPhase, GraphRunStopReason,
    GraphRunStore, GraphRunner, GraphTurnHandoff,
};
use crate::agent::planner::{DefaultGraphPlanner, GraphPlanner};
use crate::agent::runtime::{AgentRuntime, TurnInput, TurnResult, TurnStreamEvent};
use crate::agent::session::{
    HistoryBranch, HistoryCheckoutMode as SessionHistoryCheckoutMode, HistoryCursor, HistoryNode,
    SessionOverview, SessionSnapshot,
};
use crate::agent::turn_flow::TurnEventSink;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
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
    pub node_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct RetrievedContextQuery {
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub node_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct ModelMonitorSummaryQuery {
    pub session_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct CapabilityListQuery {
    pub source_id: Option<String>,
    pub kind: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ApplyMcpSourceSnapshotCommand {
    pub snapshot: McpSourceSnapshot,
}

#[derive(Clone)]
pub struct CapabilityInspectionQuery {
    pub capability_id: String,
}

#[derive(Clone)]
pub struct CapabilitySourceInspectionQuery {
    pub source_id: String,
}

#[derive(Clone)]
pub struct ModelMonitorSessionDrilldownQuery {
    pub session_id: String,
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

#[derive(Clone, Default)]
pub struct HistoryGraphQuery {
    pub session_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct HistoryCursorQuery {
    pub session_id: Option<String>,
}

#[derive(Clone)]
pub struct CheckoutHistoryNodeCommand {
    pub session_id: Option<String>,
    pub node_id: String,
    pub mode: HistoryCheckoutMode,
}

#[derive(Clone)]
pub struct RestoreBranchHeadCommand {
    pub session_id: Option<String>,
    pub branch_id: Option<String>,
}

#[derive(Clone)]
pub struct ForkFromHistoryNodeCommand {
    pub session_id: Option<String>,
    pub node_id: String,
}

#[derive(Clone)]
pub struct SwitchHistoryBranchCommand {
    pub session_id: Option<String>,
    pub branch_id: String,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeView {
    pub session: SessionSnapshot,
    pub retrieved: RetrievedContextState,
    pub checkpoint: Option<ExecutionCheckpoint>,
    pub history_nodes: Option<Vec<HistoryNodeView>>,
    pub history_branches: Option<Vec<HistoryBranchView>>,
    pub history_cursor: Option<HistoryCursorState>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorOverview {
    pub session_count: u64,
    pub request_count: u64,
    pub model_call_count: u64,
    pub tool_call_count: u64,
    pub failed_request_count: u64,
    pub retrieval_participation_count: u64,
    pub input_tokens: u64,
    pub cache_hit_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub avg_first_token_latency_ms: Option<u64>,
    pub avg_turn_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorDimensionRow {
    pub key: String,
    pub label: String,
    pub request_count: u64,
    pub model_call_count: u64,
    pub failed_request_count: u64,
    pub retrieval_participation_count: u64,
    pub input_tokens: u64,
    pub cache_hit_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub avg_first_token_latency_ms: Option<u64>,
    pub avg_turn_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorToolRow {
    pub key: String,
    pub label: String,
    pub call_count: u64,
    pub failed_call_count: u64,
    pub avg_duration_ms: Option<u64>,
    pub total_duration_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorActivityRow {
    pub key: String,
    pub label: String,
    pub call_count: u64,
    pub failed_call_count: u64,
    pub avg_duration_ms: Option<u64>,
    pub total_duration_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorSessionRow {
    pub session_id: String,
    pub title: String,
    pub summary: String,
    pub updated_at_ms: u64,
    pub request_count: u64,
    pub model_call_count: u64,
    pub tool_call_count: u64,
    pub failed_request_count: u64,
    pub retrieval_participation_count: u64,
    pub input_tokens: u64,
    pub cache_hit_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub avg_first_token_latency_ms: Option<u64>,
    pub avg_turn_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorSummaryView {
    pub overview: ModelMonitorOverview,
    pub providers: Vec<ModelMonitorDimensionRow>,
    pub models: Vec<ModelMonitorDimensionRow>,
    pub tools: Vec<ModelMonitorToolRow>,
    pub capability_sources: Vec<ModelMonitorActivityRow>,
    pub capability_invocation_modes: Vec<ModelMonitorActivityRow>,
    pub capability_failure_classes: Vec<ModelMonitorActivityRow>,
    pub sessions: Vec<ModelMonitorSessionRow>,
    pub generated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorSessionDrilldownView {
    pub session_id: String,
    pub metrics: ModelMonitorSessionRow,
    pub runtime_view: SessionRuntimeView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCheckoutMode {
    TranscriptOnly,
    TranscriptAndWorkspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCursorMode {
    Live,
    Historical,
    HistoricalDirty,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryNodeView {
    pub node_id: String,
    pub session_id: String,
    pub parent_node_id: Option<String>,
    pub branch_id: String,
    pub forked_from_node_id: Option<String>,
    pub kind: String,
    pub summary: Option<String>,
    pub created_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryBranchView {
    pub branch_id: String,
    pub session_id: String,
    pub base_node_id: Option<String>,
    pub head_node_id: Option<String>,
    pub forked_from_branch_id: Option<String>,
    pub forked_from_node_id: Option<String>,
    pub label: String,
    pub created_at_ms: Option<u64>,
    pub updated_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCursorState {
    pub session_id: String,
    pub visible_node_id: Option<String>,
    pub active_branch_id: Option<String>,
    pub branch_head_node_id: Option<String>,
    pub workspace_node_id: Option<String>,
    pub mode: HistoryCursorMode,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryGraphView {
    pub session_id: String,
    pub nodes: Vec<HistoryNodeView>,
    pub branches: Vec<HistoryBranchView>,
    pub cursor: HistoryCursorState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCheckoutResponse {
    pub session_id: String,
    pub node_id: String,
    pub requested_mode: HistoryCheckoutMode,
    pub applied_mode: HistoryCheckoutMode,
    pub workspace_rollback_capable: bool,
    pub workspace_rollback_applied: bool,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub cursor: HistoryCursorState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBranchHeadResponse {
    pub session_id: String,
    pub branch_id: Option<String>,
    pub restored_node_id: Option<String>,
    pub cursor: HistoryCursorState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkFromHistoryNodeResponse {
    pub session_id: String,
    pub node_id: String,
    pub branch: HistoryBranchView,
    pub cursor: HistoryCursorState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchHistoryBranchResponse {
    pub session_id: String,
    pub branch_id: String,
    pub node_id: Option<String>,
    pub cursor: HistoryCursorState,
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
    cache_hit_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    trace_steps: Option<Vec<crate::agent::telemetry::TurnTraceStep>>,
    trace_timeline: Option<Vec<crate::agent::session::TraceTimelineEntry>>,
    tool_activities: Option<Vec<crate::agent::telemetry::TurnToolActivity>>,
    provider_call_records: Option<Vec<crate::agent::telemetry::ProviderCallCacheRecord>>,
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
            cache_hit_input_tokens: terminal.cache_hit_input_tokens,
            reasoning_tokens: terminal.reasoning_tokens,
            output_tokens: terminal.output_tokens,
            total_tokens: terminal.total_tokens,
            first_token_latency_ms: terminal.first_token_latency_ms,
            turn_duration_ms: None,
            user_message,
            assistant_message: assistant_message.clone(),
            trace_steps: terminal.trace_steps.clone().unwrap_or_default(),
            trace_timeline: terminal.trace_timeline.clone().unwrap_or_default(),
            tool_activities: terminal.tool_activities.clone().unwrap_or_default(),
            provider_call_records: terminal.provider_call_records.clone().unwrap_or_default(),
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
        terminal.cache_hit_input_tokens = payload.cache_hit_input_tokens;
        terminal.reasoning_tokens = payload.reasoning_tokens;
        terminal.output_tokens = payload.output_tokens;
        terminal.total_tokens = payload.total_tokens;
        terminal.first_token_latency_ms = payload.first_token_latency_ms;
        terminal.trace_steps = payload.trace_steps.clone();
        terminal.trace_timeline = payload.trace_timeline.clone();
        terminal.tool_activities = payload.tool_activities.clone();
        terminal.provider_call_records = payload.provider_call_records.clone();
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
    capability_registry: Mutex<CapabilityRegistry>,
}

impl HostControlPlane {
    pub fn new() -> Self {
        Self {
            runtime: Mutex::new(AgentRuntime::new()),
            execution_control: ExecutionControlRegistry::new(),
            graph_runs: Mutex::new(default_graph_run_store()),
            graph_runner: GraphRunner::new(),
            graph_planner: Box::new(DefaultGraphPlanner),
            capability_registry: Mutex::new(CapabilityRegistry::new()),
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
            capability_registry: Mutex::new(CapabilityRegistry::new()),
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

    pub fn list_capability_sources(&self) -> Vec<CapabilitySourceView> {
        self.capability_registry
            .lock()
            .expect("capability registry lock poisoned")
            .list_sources()
    }

    pub fn list_capabilities(&self, query: CapabilityListQuery) -> Vec<CapabilityView> {
        self.capability_registry
            .lock()
            .expect("capability registry lock poisoned")
            .list_capabilities(query.source_id.as_deref(), query.kind.as_deref())
    }

    pub fn inspect_capability(&self, query: CapabilityInspectionQuery) -> Option<CapabilityView> {
        self.capability_registry
            .lock()
            .expect("capability registry lock poisoned")
            .inspect_capability(&query.capability_id)
    }

    pub fn inspect_capability_source(
        &self,
        query: CapabilitySourceInspectionQuery,
    ) -> Option<CapabilitySourceView> {
        self.capability_registry
            .lock()
            .expect("capability registry lock poisoned")
            .inspect_source(&query.source_id)
    }

    pub fn apply_mcp_source_snapshot(
        &self,
        command: ApplyMcpSourceSnapshotCommand,
    ) -> Result<CapabilitySourceView, String> {
        validate_mcp_source_snapshot(&command.snapshot)?;

        {
            let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
            runtime.apply_mcp_source_snapshot(command.snapshot.clone());
        }

        let mut registry = self
            .capability_registry
            .lock()
            .expect("capability registry lock poisoned");
        registry.replace_mcp_source_snapshot(command.snapshot.clone());

        Ok(command.snapshot.source)
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

    pub fn load_model_monitor_summary(
        &self,
        query: ModelMonitorSummaryQuery,
    ) -> ModelMonitorSummaryView {
        let target_session_id = query
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let session_overviews = runtime.list_sessions();
        let selected_overviews = session_overviews
            .into_iter()
            .filter(|overview| {
                target_session_id
                    .as_deref()
                    .map(|session_id| overview.conversation_id == session_id)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();

        let mut overview = ModelMonitorOverview {
            session_count: selected_overviews.len() as u64,
            ..ModelMonitorOverview::default()
        };
        let mut overview_first_token_latency = AverageAccumulator::default();
        let mut overview_turn_duration = AverageAccumulator::default();
        let mut providers = std::collections::BTreeMap::<String, MonitorAggregate>::new();
        let mut models = std::collections::BTreeMap::<String, MonitorAggregate>::new();
        let mut tools = std::collections::BTreeMap::<String, ToolAggregate>::new();
        let mut capability_sources =
            std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut capability_invocation_modes =
            std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut capability_failure_classes =
            std::collections::BTreeMap::<String, ActivityAggregate>::new();
        let mut sessions = Vec::with_capacity(selected_overviews.len());

        for session_overview in selected_overviews {
            let snapshot =
                runtime.load_session_snapshot(Some(session_overview.conversation_id.as_str()));
            let session_metrics = aggregate_session_metrics(&snapshot);
            merge_monitor_overview(
                &mut overview,
                &session_metrics,
                &mut overview_first_token_latency,
                &mut overview_turn_duration,
            );

            for trace in &snapshot.turn_trace_history {
                let (provider_key, provider_label) = provider_dimension_key_and_label(trace);
                providers
                    .entry(provider_key.clone())
                    .or_insert_with(|| {
                        MonitorAggregate::new(provider_key.clone(), provider_label.clone())
                    })
                    .add_trace(trace);

                let (model_key, model_label) = model_dimension_key_and_label(trace);
                models
                    .entry(model_key.clone())
                    .or_insert_with(|| MonitorAggregate::new(model_key.clone(), model_label))
                    .add_trace(trace);

                append_tool_aggregates(trace, &mut tools);
                append_capability_aggregates(
                    trace,
                    &mut capability_sources,
                    &mut capability_invocation_modes,
                    &mut capability_failure_classes,
                );
            }

            sessions.push(ModelMonitorSessionRow {
                session_id: snapshot.conversation_id.clone(),
                title: snapshot.title.clone(),
                summary: snapshot.summary.clone(),
                updated_at_ms: snapshot.updated_at_ms,
                request_count: session_metrics.request_count,
                model_call_count: session_metrics.model_call_count,
                tool_call_count: session_metrics.tool_call_count,
                failed_request_count: session_metrics.failed_request_count,
                retrieval_participation_count: session_metrics.retrieval_participation_count,
                input_tokens: session_metrics.input_tokens,
                cache_hit_input_tokens: session_metrics.cache_hit_input_tokens,
                output_tokens: session_metrics.output_tokens,
                total_tokens: session_metrics.total_tokens,
                avg_first_token_latency_ms: session_metrics.first_token_latency.average(),
                avg_turn_duration_ms: session_metrics.turn_duration.average(),
            });
        }

        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });

        overview.avg_first_token_latency_ms = overview_first_token_latency.average();
        overview.avg_turn_duration_ms = overview_turn_duration.average();

        let mut providers = providers
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        providers.sort_by(|left, right| {
            right
                .request_count
                .cmp(&left.request_count)
                .then_with(|| left.label.cmp(&right.label))
        });

        let mut models = models
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            right
                .request_count
                .cmp(&left.request_count)
                .then_with(|| left.label.cmp(&right.label))
        });

        let mut tools = tools
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| {
            right
                .call_count
                .cmp(&left.call_count)
                .then_with(|| left.label.cmp(&right.label))
        });

        let mut capability_sources = capability_sources
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        capability_sources.sort_by(activity_row_sort);

        let mut capability_invocation_modes = capability_invocation_modes
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        capability_invocation_modes.sort_by(activity_row_sort);

        let mut capability_failure_classes = capability_failure_classes
            .into_values()
            .map(|aggregate| aggregate.into_row())
            .collect::<Vec<_>>();
        capability_failure_classes.sort_by(activity_row_sort);

        ModelMonitorSummaryView {
            overview,
            providers,
            models,
            tools,
            capability_sources,
            capability_invocation_modes,
            capability_failure_classes,
            sessions,
            generated_at_ms: now_timestamp_ms(),
        }
    }

    pub fn load_model_monitor_session_drilldown(
        &self,
        query: ModelMonitorSessionDrilldownQuery,
    ) -> ModelMonitorSessionDrilldownView {
        let session_id = self.normalize_history_session_id(Some(query.session_id));
        let runtime_view = self.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some(session_id.clone()),
            ..SessionRuntimeViewQuery::default()
        });
        let metrics = aggregate_session_metrics(&runtime_view.session);
        ModelMonitorSessionDrilldownView {
            session_id: session_id.clone(),
            metrics: ModelMonitorSessionRow {
                session_id,
                title: runtime_view.session.title.clone(),
                summary: runtime_view.session.summary.clone(),
                updated_at_ms: runtime_view.session.updated_at_ms,
                request_count: metrics.request_count,
                model_call_count: metrics.model_call_count,
                tool_call_count: metrics.tool_call_count,
                failed_request_count: metrics.failed_request_count,
                retrieval_participation_count: metrics.retrieval_participation_count,
                input_tokens: metrics.input_tokens,
                cache_hit_input_tokens: metrics.cache_hit_input_tokens,
                output_tokens: metrics.output_tokens,
                total_tokens: metrics.total_tokens,
                avg_first_token_latency_ms: metrics.first_token_latency.average(),
                avg_turn_duration_ms: metrics.turn_duration.average(),
            },
            runtime_view,
        }
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
        let session_id = session_id
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())?;
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

    fn normalize_history_session_id(&self, session_id: Option<String>) -> String {
        session_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
                runtime.load_session_snapshot(None).conversation_id
            })
    }

    fn history_cursor_mode_from_session(cursor: &HistoryCursor) -> HistoryCursorMode {
        match cursor.mode {
            crate::agent::session::HistoryCursorMode::Live => HistoryCursorMode::Live,
            crate::agent::session::HistoryCursorMode::Historical => HistoryCursorMode::Historical,
            crate::agent::session::HistoryCursorMode::HistoricalDirty => {
                HistoryCursorMode::HistoricalDirty
            }
        }
    }

    fn history_checkout_mode_to_session(mode: HistoryCheckoutMode) -> SessionHistoryCheckoutMode {
        match mode {
            HistoryCheckoutMode::TranscriptOnly => SessionHistoryCheckoutMode::TranscriptOnly,
            HistoryCheckoutMode::TranscriptAndWorkspace => {
                SessionHistoryCheckoutMode::TranscriptAndWorkspace
            }
        }
    }

    fn history_checkout_mode_from_session(
        mode: crate::agent::session::HistoryCheckoutMode,
    ) -> HistoryCheckoutMode {
        match mode {
            crate::agent::session::HistoryCheckoutMode::TranscriptOnly => {
                HistoryCheckoutMode::TranscriptOnly
            }
            crate::agent::session::HistoryCheckoutMode::TranscriptAndWorkspace => {
                HistoryCheckoutMode::TranscriptAndWorkspace
            }
        }
    }

    fn history_node_view(node: &HistoryNode) -> HistoryNodeView {
        HistoryNodeView {
            node_id: node.node_id.clone(),
            session_id: node.session_id.clone(),
            parent_node_id: node.parent_node_id.clone(),
            branch_id: node.branch_id.clone(),
            forked_from_node_id: node.forked_from_node_id.clone(),
            kind: serde_json::to_value(&node.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "turn_committed".to_string()),
            summary: Some(node.summary.clone()),
            created_at_ms: Some(node.created_at_ms),
        }
    }

    fn history_branch_view(branch: &HistoryBranch) -> HistoryBranchView {
        HistoryBranchView {
            branch_id: branch.branch_id.clone(),
            session_id: branch.session_id.clone(),
            base_node_id: branch.base_node_id.clone(),
            head_node_id: branch.head_node_id.clone(),
            forked_from_branch_id: branch.forked_from_branch_id.clone(),
            forked_from_node_id: branch.forked_from_node_id.clone(),
            label: branch.label.clone(),
            created_at_ms: Some(branch.created_at_ms),
            updated_at_ms: Some(branch.updated_at_ms),
        }
    }

    fn history_cursor_state(cursor: &HistoryCursor) -> HistoryCursorState {
        HistoryCursorState {
            session_id: cursor.session_id.clone(),
            visible_node_id: cursor.visible_node_id.clone(),
            active_branch_id: cursor.active_branch_id.clone(),
            branch_head_node_id: cursor.branch_head_node_id.clone(),
            workspace_node_id: cursor.workspace_node_id.clone(),
            mode: Self::history_cursor_mode_from_session(cursor),
        }
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

    pub fn load_history_graph(&self, query: HistoryGraphQuery) -> HistoryGraphView {
        let session_id = self.normalize_history_session_id(query.session_id);
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let (nodes, branches, cursor) = runtime.load_history_graph(Some(session_id.as_str()));
        HistoryGraphView {
            session_id,
            nodes: nodes.iter().map(Self::history_node_view).collect(),
            branches: branches.iter().map(Self::history_branch_view).collect(),
            cursor: Self::history_cursor_state(&cursor),
        }
    }

    pub fn load_history_cursor(&self, query: HistoryCursorQuery) -> HistoryCursorState {
        let session_id = self.normalize_history_session_id(query.session_id);
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let cursor = runtime.load_history_cursor(Some(session_id.as_str()));
        Self::history_cursor_state(&cursor)
    }

    pub fn checkout_history_node(
        &self,
        command: CheckoutHistoryNodeCommand,
    ) -> Result<HistoryCheckoutResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let requested_mode = command.mode;
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let snapshot = runtime.checkout_history_node(
            Some(session_id.as_str()),
            &command.node_id,
            Self::history_checkout_mode_to_session(requested_mode),
        )?;
        let cursor = Self::history_cursor_state(&snapshot.history_cursor);
        let degraded = matches!(
            snapshot.history_cursor.checkout_status,
            crate::agent::session::HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
        Ok(HistoryCheckoutResponse {
            session_id,
            node_id: command.node_id,
            requested_mode,
            applied_mode: Self::history_checkout_mode_from_session(
                snapshot.history_cursor.checkout_mode,
            ),
            workspace_rollback_capable: snapshot
                .history_nodes
                .iter()
                .find(|node| Some(node.node_id.as_str()) == snapshot.resolved_node_id.as_deref())
                .map(|node| node.workspace_ref.rollback_capable)
                .unwrap_or(false),
            workspace_rollback_applied: requested_mode
                == HistoryCheckoutMode::TranscriptAndWorkspace
                && !degraded,
            degraded,
            degradation_reason: degraded.then(|| "workspace_rollback_unsupported".to_string()),
            cursor,
        })
    }

    pub fn restore_branch_head(
        &self,
        command: RestoreBranchHeadCommand,
    ) -> Result<RestoreBranchHeadResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let snapshot =
            runtime.restore_branch_head(Some(session_id.as_str()), command.branch_id.as_deref())?;
        Ok(RestoreBranchHeadResponse {
            session_id,
            branch_id: snapshot.history_cursor.active_branch_id.clone(),
            restored_node_id: snapshot.resolved_node_id.clone(),
            cursor: Self::history_cursor_state(&snapshot.history_cursor),
        })
    }

    pub fn fork_from_history_node(
        &self,
        command: ForkFromHistoryNodeCommand,
    ) -> Result<ForkFromHistoryNodeResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let before = runtime.load_history_cursor(Some(session_id.as_str()));
        let snapshot =
            runtime.fork_from_history_node(Some(session_id.as_str()), &command.node_id)?;
        let created_branch_id = snapshot
            .history_cursor
            .active_branch_id
            .clone()
            .ok_or_else(|| "fork did not produce an active branch".to_string())?;
        let branch = snapshot
            .history_branches
            .iter()
            .find(|item| item.branch_id == created_branch_id)
            .cloned()
            .ok_or_else(|| "forked branch not found in snapshot".to_string())?;
        let mut branch_view = Self::history_branch_view(&branch);
        if branch_view.forked_from_branch_id.is_none() {
            branch_view.forked_from_branch_id = before.active_branch_id;
        }
        Ok(ForkFromHistoryNodeResponse {
            session_id,
            node_id: command.node_id,
            branch: branch_view,
            cursor: Self::history_cursor_state(&snapshot.history_cursor),
        })
    }

    pub fn switch_history_branch(
        &self,
        command: SwitchHistoryBranchCommand,
    ) -> Result<SwitchHistoryBranchResponse, String> {
        let session_id = self.normalize_history_session_id(command.session_id);
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let snapshot =
            runtime.switch_history_branch(Some(session_id.as_str()), &command.branch_id)?;
        Ok(SwitchHistoryBranchResponse {
            session_id,
            branch_id: command.branch_id,
            node_id: snapshot.resolved_node_id.clone(),
            cursor: Self::history_cursor_state(&snapshot.history_cursor),
        })
    }

    pub fn load_session_runtime_view(&self, query: SessionRuntimeViewQuery) -> SessionRuntimeView {
        let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: query.turn_id,
            session_id: query.session_id.clone(),
        });
        let resolved_session_id =
            self.normalize_history_session_id(query.session_id.or_else(|| {
                checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.session_id.clone())
            }));
        let run = self.resolve_graph_run_for_retrieval(
            query.run_id.as_deref(),
            Some(resolved_session_id.as_str()),
        );
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        let session = runtime
            .load_session_snapshot_at(Some(resolved_session_id.as_str()), query.node_id.as_deref());
        let retrieved = runtime.inspect_retrieved_context_at(
            Some(resolved_session_id.as_str()),
            query.node_id.as_deref(),
            run.as_ref(),
            checkpoint.as_ref(),
        );

        SessionRuntimeView {
            history_nodes: Some(
                session
                    .history_nodes
                    .iter()
                    .map(Self::history_node_view)
                    .collect(),
            ),
            history_branches: Some(
                session
                    .history_branches
                    .iter()
                    .map(Self::history_branch_view)
                    .collect(),
            ),
            history_cursor: Some(Self::history_cursor_state(&session.history_cursor)),
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
        let resolved_session_id =
            self.normalize_history_session_id(query.session_id.or_else(|| {
                checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.session_id.clone())
            }));
        let run = self.resolve_graph_run_for_retrieval(
            query.run_id.as_deref(),
            Some(resolved_session_id.as_str()),
        );
        let mut runtime = self.runtime.lock().expect("runtime lock poisoned");
        runtime.inspect_retrieved_context_at(
            Some(resolved_session_id.as_str()),
            query.node_id.as_deref(),
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

fn validate_mcp_source_view(source: &CapabilitySourceView) -> Result<(), String> {
    if source.source_kind != CapabilitySourceKind::Mcp {
        return Err("Only MCP-backed sources may be registered through this command.".to_string());
    }
    if source.source_id.trim().is_empty() {
        return Err("Capability source id is empty.".to_string());
    }
    if source.display_name.trim().is_empty() {
        return Err("Capability source display name is empty.".to_string());
    }
    if source.transport_kind.trim().is_empty() {
        return Err("Capability source transport kind is empty.".to_string());
    }
    if source.server_identity.trim().is_empty() {
        return Err("Capability source server identity is empty.".to_string());
    }
    Ok(())
}

fn validate_mcp_capability_view(capability: &CapabilityView) -> Result<(), String> {
    if capability.source_kind != CapabilitySourceKind::Mcp {
        return Err(
            "Only MCP-backed capabilities may be registered through this command.".to_string(),
        );
    }
    if capability.capability_id.trim().is_empty() {
        return Err("Capability id is empty.".to_string());
    }
    if capability.source_id.trim().is_empty() {
        return Err("Capability source id is empty.".to_string());
    }
    if capability.label.trim().is_empty() {
        return Err("Capability label is empty.".to_string());
    }

    let expected_mode = match capability.kind {
        CapabilityKind::Tool => CapabilityInvocationMode::DirectToolCall,
        CapabilityKind::Resource => CapabilityInvocationMode::ReadOnlyFetch,
        CapabilityKind::PromptTemplate => CapabilityInvocationMode::PromptExpansion,
    };

    if capability.invocation_mode != expected_mode {
        return Err(format!(
            "Capability `{}` has incompatible invocation mode `{}` for kind `{}`.",
            capability.capability_id,
            capability.invocation_mode.as_str(),
            capability.kind.as_str()
        ));
    }

    Ok(())
}

fn validate_mcp_source_snapshot(snapshot: &McpSourceSnapshot) -> Result<(), String> {
    validate_mcp_source_view(&snapshot.source)?;
    for capability in &snapshot.capabilities {
        validate_mcp_capability_view(capability)?;
        if capability.source_id != snapshot.source.source_id {
            return Err(format!(
                "Capability `{}` does not belong to source `{}`.",
                capability.capability_id, snapshot.source.source_id
            ));
        }
    }
    Ok(())
}

fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, Default)]
struct AverageAccumulator {
    sum: u128,
    count: u64,
}

impl AverageAccumulator {
    fn push(&mut self, value: Option<u64>) {
        if let Some(value) = value {
            self.sum += u128::from(value);
            self.count += 1;
        }
    }

    fn average(&self) -> Option<u64> {
        if self.count == 0 {
            None
        } else {
            Some((self.sum / u128::from(self.count)) as u64)
        }
    }
}

#[derive(Clone, Debug, Default)]
struct SessionMetricsAggregate {
    request_count: u64,
    model_call_count: u64,
    tool_call_count: u64,
    failed_request_count: u64,
    retrieval_participation_count: u64,
    input_tokens: u64,
    cache_hit_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    first_token_latency: AverageAccumulator,
    turn_duration: AverageAccumulator,
}

#[derive(Clone, Debug, Default)]
struct MonitorAggregate {
    key: String,
    label: String,
    request_count: u64,
    model_call_count: u64,
    failed_request_count: u64,
    retrieval_participation_count: u64,
    input_tokens: u64,
    cache_hit_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    first_token_latency: AverageAccumulator,
    turn_duration: AverageAccumulator,
}

impl MonitorAggregate {
    fn new(key: String, label: String) -> Self {
        Self {
            key,
            label,
            ..Self::default()
        }
    }

    fn add_trace(&mut self, trace: &crate::agent::session::TurnTraceRecord) {
        self.request_count += 1;
        self.model_call_count += count_model_calls(trace);
        self.failed_request_count += u64::from(trace.error.is_some());
        self.retrieval_participation_count += u64::from(trace_uses_retrieval(trace));
        self.input_tokens += trace.input_tokens.unwrap_or(0);
        self.cache_hit_input_tokens += trace.cache_hit_input_tokens.unwrap_or(0);
        self.output_tokens += trace.output_tokens.unwrap_or(0);
        self.total_tokens += trace.total_tokens.unwrap_or(0);
        self.first_token_latency.push(trace.first_token_latency_ms);
        self.turn_duration.push(trace.turn_duration_ms);
    }

    fn into_row(self) -> ModelMonitorDimensionRow {
        ModelMonitorDimensionRow {
            key: self.key,
            label: self.label,
            request_count: self.request_count,
            model_call_count: self.model_call_count,
            failed_request_count: self.failed_request_count,
            retrieval_participation_count: self.retrieval_participation_count,
            input_tokens: self.input_tokens,
            cache_hit_input_tokens: self.cache_hit_input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.total_tokens,
            avg_first_token_latency_ms: self.first_token_latency.average(),
            avg_turn_duration_ms: self.turn_duration.average(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ToolAggregate {
    key: String,
    label: String,
    call_count: u64,
    failed_call_count: u64,
    total_duration_ms: u64,
    duration: AverageAccumulator,
}

impl ToolAggregate {
    fn new(key: String, label: String) -> Self {
        Self {
            key,
            label,
            ..Self::default()
        }
    }

    fn add_activity(&mut self, activity: &crate::agent::telemetry::TurnToolActivity) {
        self.call_count += 1;
        self.failed_call_count += u64::from(activity.status != "completed");
        let duration_ms = activity
            .duration_seconds
            .map(|value| (value * 1000.0).round().max(0.0) as u64);
        self.duration.push(duration_ms);
        self.total_duration_ms += duration_ms.unwrap_or(0);
    }

    fn into_row(self) -> ModelMonitorToolRow {
        ModelMonitorToolRow {
            key: self.key,
            label: self.label,
            call_count: self.call_count,
            failed_call_count: self.failed_call_count,
            avg_duration_ms: self.duration.average(),
            total_duration_ms: self.total_duration_ms,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ActivityAggregate {
    key: String,
    label: String,
    call_count: u64,
    failed_call_count: u64,
    total_duration_ms: u64,
    duration: AverageAccumulator,
}

impl ActivityAggregate {
    fn new(key: String, label: String) -> Self {
        Self {
            key,
            label,
            ..Self::default()
        }
    }

    fn add_activity(&mut self, activity: &crate::agent::telemetry::TurnToolActivity) {
        self.call_count += 1;
        self.failed_call_count += u64::from(activity.status != "completed");
        let duration_ms = activity
            .duration_seconds
            .map(|value| (value * 1000.0).round().max(0.0) as u64);
        self.duration.push(duration_ms);
        self.total_duration_ms += duration_ms.unwrap_or(0);
    }

    fn into_row(self) -> ModelMonitorActivityRow {
        ModelMonitorActivityRow {
            key: self.key,
            label: self.label,
            call_count: self.call_count,
            failed_call_count: self.failed_call_count,
            avg_duration_ms: self.duration.average(),
            total_duration_ms: self.total_duration_ms,
        }
    }
}

fn aggregate_session_metrics(session: &SessionSnapshot) -> SessionMetricsAggregate {
    let mut aggregate = SessionMetricsAggregate::default();
    for trace in &session.turn_trace_history {
        aggregate.request_count += 1;
        aggregate.model_call_count += count_model_calls(trace);
        aggregate.tool_call_count += count_tool_calls(trace);
        aggregate.failed_request_count += u64::from(trace.error.is_some());
        aggregate.retrieval_participation_count += u64::from(trace_uses_retrieval(trace));
        aggregate.input_tokens += trace.input_tokens.unwrap_or(0);
        aggregate.cache_hit_input_tokens += trace.cache_hit_input_tokens.unwrap_or(0);
        aggregate.output_tokens += trace.output_tokens.unwrap_or(0);
        aggregate.total_tokens += trace.total_tokens.unwrap_or(0);
        aggregate
            .first_token_latency
            .push(trace.first_token_latency_ms);
        aggregate.turn_duration.push(trace.turn_duration_ms);
    }
    aggregate
}

fn merge_monitor_overview(
    overview: &mut ModelMonitorOverview,
    metrics: &SessionMetricsAggregate,
    first_token_latency: &mut AverageAccumulator,
    turn_duration: &mut AverageAccumulator,
) {
    overview.request_count += metrics.request_count;
    overview.model_call_count += metrics.model_call_count;
    overview.tool_call_count += metrics.tool_call_count;
    overview.failed_request_count += metrics.failed_request_count;
    overview.retrieval_participation_count += metrics.retrieval_participation_count;
    overview.input_tokens += metrics.input_tokens;
    overview.cache_hit_input_tokens += metrics.cache_hit_input_tokens;
    overview.output_tokens += metrics.output_tokens;
    overview.total_tokens += metrics.total_tokens;
    first_token_latency.sum += metrics.first_token_latency.sum;
    first_token_latency.count += metrics.first_token_latency.count;
    turn_duration.sum += metrics.turn_duration.sum;
    turn_duration.count += metrics.turn_duration.count;
}

fn provider_dimension_key_and_label(
    trace: &crate::agent::session::TurnTraceRecord,
) -> (String, String) {
    let provider = trace
        .provider_name
        .as_deref()
        .or(trace.provider_requested_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "unknown-provider".to_string());
    (provider.clone(), provider)
}

fn model_dimension_key_and_label(
    trace: &crate::agent::session::TurnTraceRecord,
) -> (String, String) {
    let provider = trace
        .provider_name
        .as_deref()
        .or(trace.provider_requested_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let model = trace
        .provider_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    match (provider, model) {
        (Some(provider), Some(model)) => {
            let label = format!("{provider}/{model}");
            (label.clone(), label)
        }
        (None, Some(model)) => (model.clone(), model),
        (Some(provider), None) => (provider.clone(), provider),
        (None, None) => ("unknown-model".to_string(), "unknown-model".to_string()),
    }
}

fn count_model_calls(trace: &crate::agent::session::TurnTraceRecord) -> u64 {
    trace.provider_call_records.len().max(1) as u64
}

fn count_tool_calls(trace: &crate::agent::session::TurnTraceRecord) -> u64 {
    trace.tool_activities.len() as u64
}

fn trace_uses_retrieval(trace: &crate::agent::session::TurnTraceRecord) -> bool {
    trace
        .build_context_observation
        .as_ref()
        .map(|observation| {
            observation.message_count > 2
                || !observation.prefix_mutation_reasons.is_empty()
                || !observation.semi_stable_context_text.trim().is_empty()
        })
        .unwrap_or(false)
}

fn append_tool_aggregates(
    trace: &crate::agent::session::TurnTraceRecord,
    tools: &mut std::collections::BTreeMap<String, ToolAggregate>,
) {
    for activity in &trace.tool_activities {
        let key = activity.name.clone();
        tools
            .entry(key.clone())
            .or_insert_with(|| ToolAggregate::new(key.clone(), activity.name.clone()))
            .add_activity(activity);
    }
}

fn append_capability_aggregates(
    trace: &crate::agent::session::TurnTraceRecord,
    sources: &mut std::collections::BTreeMap<String, ActivityAggregate>,
    invocation_modes: &mut std::collections::BTreeMap<String, ActivityAggregate>,
    failure_classes: &mut std::collections::BTreeMap<String, ActivityAggregate>,
) {
    for activity in &trace.tool_activities {
        let Some(invocation) = activity.capability_invocation.as_ref() else {
            continue;
        };

        let source_key = invocation
            .source_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown-source")
            .to_string();
        sources
            .entry(source_key.clone())
            .or_insert_with(|| ActivityAggregate::new(source_key.clone(), source_key.clone()))
            .add_activity(activity);

        let invocation_mode_key = invocation
            .invocation_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown-mode")
            .to_string();
        invocation_modes
            .entry(invocation_mode_key.clone())
            .or_insert_with(|| {
                ActivityAggregate::new(invocation_mode_key.clone(), invocation_mode_key.clone())
            })
            .add_activity(activity);

        let failure_key = invocation
            .failure_kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("ok")
            .to_string();
        failure_classes
            .entry(failure_key.clone())
            .or_insert_with(|| ActivityAggregate::new(failure_key.clone(), failure_key.clone()))
            .add_activity(activity);
    }
}

fn activity_row_sort(left: &ModelMonitorActivityRow, right: &ModelMonitorActivityRow) -> Ordering {
    right
        .call_count
        .cmp(&left.call_count)
        .then_with(|| left.label.cmp(&right.label))
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
                cache_hit_input_tokens: Some(8),
                reasoning_tokens: Some(5),
                output_tokens: Some(34),
                total_tokens: Some(55),
                first_token_latency_ms: Some(180),
                turn_duration_ms: Some(960),
                trace_steps: Some(Vec::new()),
                trace_timeline: Some(Vec::new()),
                tool_activities: Some(Vec::new()),
                provider_call_records: None,
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
                    node_id: None,
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
                node_id: None,
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
        let (control_plane, server) =
            build_test_control_plane(vec![json_completion("runtime-view")]);

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请继续推进 PA-018。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-runtime-view".to_string()),
                node_id: None,
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
                node_id: None,
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
                    node_id: None,
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
        assert_eq!(
            retrieved.run_state.run_id.as_deref(),
            Some("run-session-aware")
        );
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
                    node_id: None,
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
                    node_id: None,
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
                    node_id: None,
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
                    node_id: None,
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
                    node_id: None,
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
                    node_id: None,
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
                    node_id: None,
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
                    node_id: None,
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
                    node_id: None,
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
                node_id: None,
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
                node_id: None,
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

    #[test]
    fn history_commands_and_runtime_view_follow_persisted_history_graph() {
        let (control_plane, server) = build_test_control_plane(vec![
            json_completion("第一答"),
            json_completion("第二答"),
            json_completion("分叉回答"),
        ]);

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "第一问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("history-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            });
            let _ = runtime.run_turn(TurnInput {
                message: "第二问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("history-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            });
        }

        let graph = control_plane.load_history_graph(HistoryGraphQuery {
            session_id: Some("history-control".to_string()),
        });
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.branches.len(), 1);

        let first_node_id = graph.nodes[0].node_id.clone();
        let latest_node_id = graph.nodes[1].node_id.clone();
        let checkout = control_plane
            .checkout_history_node(CheckoutHistoryNodeCommand {
                session_id: Some("history-control".to_string()),
                node_id: first_node_id.clone(),
                mode: HistoryCheckoutMode::TranscriptAndWorkspace,
            })
            .expect("checkout should succeed");
        assert!(checkout.degraded);
        assert_eq!(checkout.cursor.mode, HistoryCursorMode::Historical);

        let historical_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("history-control".to_string()),
            node_id: Some(first_node_id.clone()),
            ..SessionRuntimeViewQuery::default()
        });
        assert_eq!(
            historical_view.session.resolved_node_id.as_deref(),
            Some(first_node_id.as_str())
        );
        assert_eq!(historical_view.session.history.len(), 2);
        assert_eq!(historical_view.session.history[0].content, "第一问");

        let fork = control_plane
            .fork_from_history_node(ForkFromHistoryNodeCommand {
                session_id: Some("history-control".to_string()),
                node_id: first_node_id.clone(),
            })
            .expect("fork should succeed");
        assert_ne!(fork.branch.branch_id.as_str(), "branch-main");
        assert_eq!(
            fork.branch.base_node_id.as_deref(),
            Some(first_node_id.as_str())
        );
        assert_eq!(
            fork.cursor.active_branch_id.as_deref(),
            Some(fork.branch.branch_id.as_str())
        );

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "在分叉上继续".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("history-control".to_string()),
                node_id: fork.cursor.visible_node_id.clone(),
                history: Vec::new(),
                images: Vec::new(),
            });
        }

        let post_fork_graph = control_plane.load_history_graph(HistoryGraphQuery {
            session_id: Some("history-control".to_string()),
        });
        let fork_branch = post_fork_graph
            .branches
            .iter()
            .find(|branch| branch.branch_id == fork.branch.branch_id)
            .expect("fork branch should exist after append");
        assert_eq!(post_fork_graph.nodes.len(), 3);
        assert_ne!(
            fork_branch.head_node_id.as_deref(),
            Some(latest_node_id.as_str())
        );

        let switched = control_plane
            .switch_history_branch(SwitchHistoryBranchCommand {
                session_id: Some("history-control".to_string()),
                branch_id: "branch-main".to_string(),
            })
            .expect("switch back to main should succeed");
        assert_eq!(switched.node_id.as_deref(), Some(latest_node_id.as_str()));

        let restored = control_plane
            .restore_branch_head(RestoreBranchHeadCommand {
                session_id: Some("history-control".to_string()),
                branch_id: Some(fork.branch.branch_id.clone()),
            })
            .expect("restore fork head should succeed");
        assert_eq!(
            restored.branch_id.as_deref(),
            Some(fork.branch.branch_id.as_str())
        );
        assert_eq!(restored.cursor.mode, HistoryCursorMode::Live);
        assert_ne!(
            restored.restored_node_id.as_deref(),
            Some(latest_node_id.as_str())
        );

        server.finish();
    }

    #[test]
    fn load_model_monitor_summary_aggregates_existing_session_traces() {
        let (control_plane, server) = build_test_control_plane(vec![
            json_completion("第一轮总结"),
            json_completion("第二轮总结"),
        ]);

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "先看第一轮".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("monitor-summary".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            });
            let _ = runtime.run_turn(TurnInput {
                message: "再继续第二轮".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("monitor-summary".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            });
        }

        let summary = control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery {
            session_id: Some("monitor-summary".to_string()),
        });

        assert_eq!(summary.overview.session_count, 1);
        assert_eq!(summary.overview.request_count, 2);
        assert_eq!(summary.overview.model_call_count, 2);
        assert_eq!(summary.sessions.len(), 1);
        assert_eq!(summary.sessions[0].session_id, "monitor-summary");
        assert_eq!(summary.providers.len(), 1);
        assert_eq!(summary.models.len(), 1);
        server.finish();
    }

    #[test]
    fn load_model_monitor_summary_aggregates_capability_usage_dimensions() {
        let mut sessions = SessionStore::memory_only();
        sessions.record_turn_trace(
            Some("monitor-capability"),
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                title: "Capability summary".to_string(),
                phase: "completed".to_string(),
                tool_activities: vec![
                    crate::agent::telemetry::TurnToolActivity {
                        id: "activity-1".to_string(),
                        name: "workspace_search".to_string(),
                        status: "completed".to_string(),
                        summary: "tool ok".to_string(),
                        arguments_text: None,
                        result_text: None,
                        duration_seconds: Some(0.2),
                        capability_invocation: Some(
                            crate::agent::telemetry::CapabilityInvocationRecord {
                                tool_name: "workspace_search".to_string(),
                                capability_id: Some("mcp:tool:workspace_search".to_string()),
                                source_id: Some("mcp-local".to_string()),
                                source_kind: Some("mcp".to_string()),
                                capability_kind: Some("tool".to_string()),
                                invocation_mode: Some("direct_tool_call".to_string()),
                                failure_kind: None,
                                requires_approval: Some(false),
                                host_mediated: Some(true),
                                permission_scope: Some("workspace.read".to_string()),
                            },
                        ),
                    },
                    crate::agent::telemetry::TurnToolActivity {
                        id: "activity-2".to_string(),
                        name: "workspace_read".to_string(),
                        status: "error".to_string(),
                        summary: "resource denied".to_string(),
                        arguments_text: None,
                        result_text: None,
                        duration_seconds: Some(0.4),
                        capability_invocation: Some(
                            crate::agent::telemetry::CapabilityInvocationRecord {
                                tool_name: "workspace_read".to_string(),
                                capability_id: Some("mcp:resource:workspace_read".to_string()),
                                source_id: Some("mcp-local".to_string()),
                                source_kind: Some("mcp".to_string()),
                                capability_kind: Some("resource".to_string()),
                                invocation_mode: Some("read_only_fetch".to_string()),
                                failure_kind: Some("permission_denied".to_string()),
                                requires_approval: Some(true),
                                host_mediated: Some(false),
                                permission_scope: Some("workspace.read".to_string()),
                            },
                        ),
                    },
                    crate::agent::telemetry::TurnToolActivity {
                        id: "activity-3".to_string(),
                        name: "local_tool".to_string(),
                        status: "completed".to_string(),
                        summary: "local only".to_string(),
                        arguments_text: None,
                        result_text: None,
                        duration_seconds: Some(0.1),
                        capability_invocation: None,
                    },
                ],
                provider_name: Some("test-openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_source: Some("test".to_string()),
                provider_mode: Some("chat".to_string()),
                session_summary: Some("capability summary".to_string()),
                input_tokens: Some(10),
                output_tokens: Some(5),
                total_tokens: Some(15),
                ..crate::agent::session::TurnTraceRecord::default()
            },
        );

        let runtime = AgentRuntime::with_dependencies(
            sessions,
            Box::new(StaticResolver {
                selection: test_provider_selection("http://localhost".to_string()),
            }),
            Box::new(ToolRouter::new()),
            Box::new(LocalTurnPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        let control_plane = HostControlPlane::with_runtime(runtime);

        let summary = control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery {
            session_id: Some("monitor-capability".to_string()),
        });

        assert_eq!(summary.capability_sources.len(), 1);
        assert_eq!(summary.capability_sources[0].key, "mcp-local");
        assert_eq!(summary.capability_sources[0].call_count, 2);
        assert_eq!(summary.capability_sources[0].failed_call_count, 1);

        assert_eq!(summary.capability_invocation_modes.len(), 2);
        assert!(summary
            .capability_invocation_modes
            .iter()
            .any(|row| row.key == "direct_tool_call" && row.call_count == 1));
        assert!(summary
            .capability_invocation_modes
            .iter()
            .any(|row| row.key == "read_only_fetch" && row.failed_call_count == 1));

        assert_eq!(summary.capability_failure_classes.len(), 2);
        assert!(summary
            .capability_failure_classes
            .iter()
            .any(|row| row.key == "ok" && row.call_count == 1));
        assert!(summary
            .capability_failure_classes
            .iter()
            .any(|row| row.key == "permission_denied" && row.failed_call_count == 1));
    }

    #[test]
    fn load_model_monitor_summary_reads_capability_activity_from_runtime_generated_trace() {
        let (control_plane, server) = build_test_control_plane(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "先列出文件。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": json!({"path": ".", "limit": 20}).to_string()
                                    }
                                }
                            ]
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            })),
            json_completion("文件已列出。"),
        ]);

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let result = runtime.run_turn(TurnInput {
                message: "列出当前目录文件".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("monitor-runtime-capability".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            });
            assert_eq!(result.phase, "ready");
            assert_eq!(result.tool_activities.len(), 1);

            let invocation = result.tool_activities[0]
                .capability_invocation
                .as_ref()
                .expect("runtime should annotate capability invocation");
            assert_eq!(invocation.tool_name, "workspace_list_files");
            assert_eq!(invocation.source_id.as_deref(), Some("builtin-tools"));
            assert_eq!(invocation.capability_kind.as_deref(), Some("tool"));
            assert_eq!(
                invocation.invocation_mode.as_deref(),
                Some("direct_tool_call")
            );
            assert_eq!(invocation.failure_kind, None);

            let snapshot = runtime.load_session_snapshot(Some("monitor-runtime-capability"));
            let trace = snapshot
                .turn_trace_history
                .last()
                .expect("trace should be persisted");
            let persisted_invocation = trace.tool_activities[0]
                .capability_invocation
                .as_ref()
                .expect("persisted trace should keep capability invocation");
            assert_eq!(
                persisted_invocation.source_id.as_deref(),
                Some("builtin-tools")
            );
        }

        let summary = control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery {
            session_id: Some("monitor-runtime-capability".to_string()),
        });
        assert!(summary
            .capability_sources
            .iter()
            .any(|row| row.key == "builtin-tools" && row.call_count == 1));
        assert!(summary
            .capability_invocation_modes
            .iter()
            .any(|row| row.key == "direct_tool_call" && row.call_count == 1));
        assert!(summary
            .capability_failure_classes
            .iter()
            .any(|row| row.key == "ok" && row.call_count == 1));

        server.finish();
    }

    #[test]
    fn list_capabilities_exposes_builtin_tools_through_unified_registry() {
        let control_plane = HostControlPlane::new();

        let sources = control_plane.list_capability_sources();
        assert!(sources.iter().any(|source| source.source_id == "builtin-tools"));

        let capabilities = control_plane.list_capabilities(CapabilityListQuery {
            source_id: Some("builtin-tools".to_string()),
            kind: Some("tool".to_string()),
        });
        assert!(!capabilities.is_empty());
        assert!(capabilities
            .iter()
            .any(|capability| capability.capability_id == "builtin:time_now"));
    }

    #[test]
    fn apply_mcp_source_snapshot_updates_read_plane_and_runtime_registry() {
        let control_plane = HostControlPlane::new();

        let snapshot = crate::agent::capability_bridge::McpSourceSnapshot {
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
                capability_id: "mcp:tool:workspace-search".to_string(),
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "workspace.search".to_string(),
                description: "Search workspace files".to_string(),
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
        };

        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: snapshot.clone(),
            })
            .expect("snapshot should apply");

        let source = control_plane
            .inspect_capability_source(CapabilitySourceInspectionQuery {
                source_id: "mcp-local".to_string(),
            })
            .expect("source should be visible");
        assert_eq!(source.transport_kind, "stdio");

        let capabilities = control_plane.list_capabilities(CapabilityListQuery {
            source_id: Some("mcp-local".to_string()),
            kind: None,
        });
        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].capability_id, "mcp:tool:workspace-search");

        let runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
        let runtime_capability = runtime
            .inspect_capability("mcp:tool:workspace-search")
            .expect("runtime registry should be synchronized");
        assert_eq!(runtime_capability.source_id, "mcp-local");
    }

    #[test]
    fn apply_mcp_source_snapshot_replaces_stale_capabilities_for_same_source() {
        let control_plane = HostControlPlane::new();

        let old_snapshot = crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Local MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://local".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
            },
            capabilities: vec![
                crate::agent::capability_bridge::CapabilityView {
                    capability_id: "mcp:tool:a".to_string(),
                    source_id: "mcp-local".to_string(),
                    source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                    kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                    label: "tool.a".to_string(),
                    description: "A".to_string(),
                    invocation_mode:
                        crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                    input_schema_summary: "{}".to_string(),
                    safety_class: "host_tool".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec!["mcp".to_string()],
                    requires_approval: false,
                    host_mediated: true,
                    permission_scope: "workspace.read".to_string(),
                },
                crate::agent::capability_bridge::CapabilityView {
                    capability_id: "mcp:tool:b".to_string(),
                    source_id: "mcp-local".to_string(),
                    source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                    kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                    label: "tool.b".to_string(),
                    description: "B".to_string(),
                    invocation_mode:
                        crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                    input_schema_summary: "{}".to_string(),
                    safety_class: "host_tool".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec!["mcp".to_string()],
                    requires_approval: false,
                    host_mediated: true,
                    permission_scope: "workspace.read".to_string(),
                },
            ],
        };
        let new_snapshot = crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                updated_at_ms: 2,
                ..old_snapshot.source.clone()
            },
            capabilities: vec![
                crate::agent::capability_bridge::CapabilityView {
                    capability_id: "mcp:tool:b".to_string(),
                    source_id: "mcp-local".to_string(),
                    source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                    kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                    label: "tool.b".to_string(),
                    description: "B".to_string(),
                    invocation_mode:
                        crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                    input_schema_summary: "{}".to_string(),
                    safety_class: "host_tool".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec!["mcp".to_string()],
                    requires_approval: false,
                    host_mediated: true,
                    permission_scope: "workspace.read".to_string(),
                },
                crate::agent::capability_bridge::CapabilityView {
                    capability_id: "mcp:tool:c".to_string(),
                    source_id: "mcp-local".to_string(),
                    source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                    kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                    label: "tool.c".to_string(),
                    description: "C".to_string(),
                    invocation_mode:
                        crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                    input_schema_summary: "{}".to_string(),
                    safety_class: "host_tool".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec!["mcp".to_string()],
                    requires_approval: false,
                    host_mediated: true,
                    permission_scope: "workspace.read".to_string(),
                },
            ],
        };

        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: old_snapshot,
            })
            .expect("old snapshot should apply");
        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: new_snapshot,
            })
            .expect("new snapshot should replace prior entries");

        let capabilities = control_plane.list_capabilities(CapabilityListQuery {
            source_id: Some("mcp-local".to_string()),
            kind: Some("tool".to_string()),
        });
        assert_eq!(capabilities.len(), 2);
        assert!(capabilities
            .iter()
            .all(|capability| capability.capability_id != "mcp:tool:a"));
        assert!(capabilities
            .iter()
            .any(|capability| capability.capability_id == "mcp:tool:b"));
        assert!(capabilities
            .iter()
            .any(|capability| capability.capability_id == "mcp:tool:c"));
    }

    #[test]
    fn apply_mcp_source_snapshot_keeps_unreachable_source_visible_without_capabilities() {
        let control_plane = HostControlPlane::new();

        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::McpSourceSnapshot {
                    source: crate::agent::capability_bridge::CapabilitySourceView {
                        source_id: "mcp-offline".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        display_name: "Offline MCP".to_string(),
                        transport_kind: "stdio".to_string(),
                        server_identity: "mcp://offline".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Unreachable,
                        declared_capabilities: vec![
                            crate::agent::capability_bridge::CapabilityKind::Tool,
                            crate::agent::capability_bridge::CapabilityKind::Resource,
                        ],
                        permission_profile: "host-mediated".to_string(),
                        updated_at_ms: 3,
                    },
                    capabilities: vec![],
                },
            })
            .expect("offline snapshot should apply");

        let source = control_plane
            .inspect_capability_source(CapabilitySourceInspectionQuery {
                source_id: "mcp-offline".to_string(),
            })
            .expect("source should remain visible");
        assert_eq!(
            source.availability,
            crate::agent::capability_bridge::CapabilityAvailability::Unreachable
        );

        let capabilities = control_plane.list_capabilities(CapabilityListQuery {
            source_id: Some("mcp-offline".to_string()),
            kind: None,
        });
        assert!(capabilities.is_empty());
    }

    #[test]
    fn inspect_capability_returns_normalized_builtin_capability_view() {
        let control_plane = HostControlPlane::new();

        let capability = control_plane
            .inspect_capability(CapabilityInspectionQuery {
                capability_id: "builtin:time_now".to_string(),
            })
            .expect("builtin capability should exist");

        assert_eq!(capability.source_id, "builtin-tools");
        assert_eq!(capability.kind.as_str(), "tool");
        assert_eq!(
            capability.invocation_mode,
            crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall
        );
    }

    #[test]
    fn load_model_monitor_session_drilldown_returns_metrics_and_runtime_view() {
        let (control_plane, server) = build_test_control_plane(vec![json_completion("下钻摘要")]);

        {
            let mut runtime = control_plane.runtime.lock().expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请准备一个下钻样本".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("monitor-drilldown".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            });
        }

        let drilldown =
            control_plane.load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery {
                session_id: "monitor-drilldown".to_string(),
            });

        assert_eq!(drilldown.session_id, "monitor-drilldown");
        assert_eq!(drilldown.metrics.session_id, "monitor-drilldown");
        assert_eq!(drilldown.metrics.request_count, 1);
        assert_eq!(
            drilldown.runtime_view.session.conversation_id,
            "monitor-drilldown"
        );
        assert_eq!(drilldown.runtime_view.session.turn_trace_history.len(), 1);
        server.finish();
    }

    #[test]
    fn model_dimension_key_keeps_same_model_names_separate_across_providers() {
        let openai_trace = crate::agent::session::TurnTraceRecord {
            provider_name: Some("openai".to_string()),
            provider_model: Some("gpt-5".to_string()),
            ..crate::agent::session::TurnTraceRecord::default()
        };
        let anthropic_trace = crate::agent::session::TurnTraceRecord {
            provider_name: Some("anthropic".to_string()),
            provider_model: Some("gpt-5".to_string()),
            ..crate::agent::session::TurnTraceRecord::default()
        };

        let (openai_key, openai_label) = model_dimension_key_and_label(&openai_trace);
        let (anthropic_key, anthropic_label) = model_dimension_key_and_label(&anthropic_trace);

        assert_eq!(openai_key, "openai/gpt-5");
        assert_eq!(openai_label, "openai/gpt-5");
        assert_eq!(anthropic_key, "anthropic/gpt-5");
        assert_eq!(anthropic_label, "anthropic/gpt-5");
        assert_ne!(openai_key, anthropic_key);
    }
}
