use crate::agent::capability_bridge::{
    CapabilityInvocationMode, CapabilityKind, CapabilityRegistry, CapabilitySourceKind,
    CapabilitySourceView, CapabilityView, McpSourceSnapshot, SkillDescriptor, SkillSourceSnapshot,
    SkillSourceView,
};
use crate::agent::context::RetrievedContextState;
use crate::agent::dispatcher::GovernedDispatcher;
use crate::agent::execution_control::{
    execution_checkpoint_contract_version, refresh_execution_checkpoint_projection,
    ExecutionCheckpoint, ExecutionControlRegistry, StopTurnResponse,
};
use crate::agent::frontend_diagnostics::{
    default_frontend_diagnostics_sqlite_path, FrontendDiagnosticsStore, FrontendStallSnapshot,
    FrontendTraceAppendCommand, FrontendTraceExportPayload, FrontendTraceQuery,
    FrontendTraceQueryResult,
};
use crate::agent::graph::{
    GraphDecision, GraphDecisionKind, GraphDecisionReason, GraphRun, GraphRunCheckpoint,
    GraphRunControlBoundaryEvidence, GraphRunEvent, GraphRunEventKind, GraphRunPhase,
    GraphRunStopReason, GraphRunStore, GraphRunner, GraphTurnHandoff,
};
use crate::agent::hooks::HistoryStateHookEvidence;
use crate::agent::hooks::{
    build_submission_plan_run_control_hook_envelope, CanonicalGraphRunEventType,
    RunControlCheckpointContext, RunControlHookEnvelope,
};
use crate::agent::planner::{DefaultGraphPlanner, GraphPlanner};
use crate::agent::plan_state::PlanStore;
use crate::agent::runtime::{
    AgentRuntime, AgentRuntimeBuilder, RunTurnFacts, SUSPENDED_TURN_PHASE, TurnInput, TurnResult,
    TurnStreamEvent,
};
use crate::agent::tool_runtime::{RuntimeClock, SystemClock};
use crate::agent::tools::ToolRegistrySnapshot;
use crate::agent::session::{
    build_missing_run_control_audit_summary, HistoryBranch,
    HistoryCheckoutMode as SessionHistoryCheckoutMode, HistoryCursor, HistoryNode,
    HistoryStateAuditSummary, RunControlAuditActionSummary, RunControlAuditCurrentContext,
    RunControlAuditSummary, SessionOverview, SessionSnapshot, SessionStore, TurnTraceRecord,
    WorkspaceRef,
};
use crate::agent::turn_flow::TurnEventSink;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
mod ask_plan_commands;
mod capability_commands;
mod graph_projection;
mod history_commands;
mod message_projection;
mod path_authorization_commands;
mod query_commands;
mod trace_commands;
mod workspace_commands;

/// 默认 workspace root：进程当前目录（与 `ToolRouter::new()` 默认一致）。
fn default_workspace_root() -> String {
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

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
pub struct GraphRunSubmissionPlanQuery {
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

#[derive(Clone, Default)]
pub struct SkillListQuery {
    pub source_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ApplyMcpSourceSnapshotCommand {
    pub snapshot: McpSourceSnapshot,
}

#[derive(Clone, Debug)]
pub struct ApplySkillSourceSnapshotCommand {
    pub snapshot: SkillSourceSnapshot,
}

#[derive(Clone)]
pub struct CapabilityInspectionQuery {
    pub capability_id: String,
}

#[derive(Clone)]
pub struct SkillInspectionQuery {
    pub skill_id: String,
}

#[derive(Clone)]
pub struct CapabilitySourceInspectionQuery {
    pub source_id: String,
}

#[derive(Clone)]
pub struct SkillSourceInspectionQuery {
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
    pub expected_cursor_version: Option<u64>,
}

#[derive(Clone)]
pub struct RestoreBranchHeadCommand {
    pub session_id: Option<String>,
    pub branch_id: Option<String>,
    pub expected_cursor_version: Option<u64>,
}

#[derive(Clone)]
pub struct ForkFromHistoryNodeCommand {
    pub session_id: Option<String>,
    pub node_id: String,
    pub expected_cursor_version: Option<u64>,
}

#[derive(Clone)]
pub struct SwitchHistoryBranchCommand {
    pub session_id: Option<String>,
    pub branch_id: String,
    pub expected_cursor_version: Option<u64>,
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
    pub message_state: MessageStateSnapshot,
    pub history_state_evidence: Option<Vec<HistoryStateHookEvidence>>,
    pub history_state_audit_summary: HistoryStateAuditSummary,
    pub run_control_audit_summary: RunControlAuditSummary,
    pub retrieved: RetrievedContextState,
    pub checkpoint: Option<ExecutionCheckpoint>,
    pub submission_plan: Option<GraphRunSubmissionPlan>,
    pub control_boundary_evidence: Option<Vec<GraphRunControlBoundaryEvidence>>,
    pub history_nodes: Option<Vec<HistoryNodeView>>,
    pub history_branches: Option<Vec<HistoryBranchView>>,
    pub history_cursor: Option<HistoryCursorState>,
    pub authority_mode: String,
    pub resolved_visible_node_id: Option<String>,
    pub active_branch_head_node_id: Option<String>,
    pub is_at_branch_head: bool,
    pub cursor_version: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStateSnapshot {
    pub session_id: String,
    pub revision: String,
    pub messages: Vec<MessageStateEntry>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageStateEntry {
    pub message_id: String,
    pub turn_id: String,
    pub role: String,
    pub content: String,
    pub attachments: Vec<crate::agent::session::AttachmentReference>,
    pub status: Option<String>,
    pub reasoning_content: Option<String>,
    pub model_name: Option<String>,
    pub token_count: Option<u64>,
    pub tool_name: Option<String>,
    pub canonical_tool_name: Option<String>,
    pub display_name_zh: Option<String>,
    pub detail: Option<String>,
    pub duration_seconds: Option<f64>,
    pub error_detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStateDelta {
    pub session_id: String,
    pub base_revision: String,
    pub target_revision: String,
    pub ops: Vec<MessageDeltaOp>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MessageDeltaOp {
    TruncateAfter { message_id: Option<String> },
    Append { messages: Vec<MessageStateEntry> },
    ReplaceAll { messages: Vec<MessageStateEntry> },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunSubmissionPlan {
    pub command: String,
    pub run_id: Option<String>,
    pub source: String,
    pub hook_point: String,
    pub canonical_event_type: String,
    pub canonical_phase: String,
    pub hook_envelope: Option<RunControlHookEnvelope>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorOverview {
    pub session_count: u64,
    pub request_count: u64,
    pub model_call_count: u64,
    pub tool_call_count: u64,
    pub hook_call_count: u64,
    pub blocked_hook_count: u64,
    pub failed_request_count: u64,
    pub retrieval_participation_count: u64,
    pub input_tokens: u64,
    pub cache_hit_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub avg_first_token_latency_ms: Option<u64>,
    pub avg_turn_duration_ms: Option<u64>,
    pub avg_hook_duration_ms: Option<u64>,
    pub total_hook_duration_ms: u64,
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
pub struct ModelMonitorHookRow {
    pub key: String,
    pub label: String,
    pub call_count: u64,
    pub blocked_call_count: u64,
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
    pub hook_call_count: u64,
    pub blocked_hook_count: u64,
    pub failed_request_count: u64,
    pub retrieval_participation_count: u64,
    pub input_tokens: u64,
    pub cache_hit_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub avg_first_token_latency_ms: Option<u64>,
    pub avg_turn_duration_ms: Option<u64>,
    pub avg_hook_duration_ms: Option<u64>,
    pub total_hook_duration_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMonitorSummaryView {
    pub overview: ModelMonitorOverview,
    pub providers: Vec<ModelMonitorDimensionRow>,
    pub models: Vec<ModelMonitorDimensionRow>,
    pub tools: Vec<ModelMonitorToolRow>,
    pub hook_classes: Vec<ModelMonitorHookRow>,
    pub hooks: Vec<ModelMonitorHookRow>,
    pub capability_sources: Vec<ModelMonitorActivityRow>,
    pub capability_invocation_modes: Vec<ModelMonitorActivityRow>,
    pub capability_failure_classes: Vec<ModelMonitorActivityRow>,
    pub skill_selections: Vec<ModelMonitorActivityRow>,
    pub skill_sources: Vec<ModelMonitorActivityRow>,
    pub skill_failure_layers: Vec<ModelMonitorActivityRow>,
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
    pub turn_id: Option<String>,
    pub workspace_ref: WorkspaceRef,
    pub summary: Option<String>,
    pub title: Option<String>,
    pub turn_count: Option<usize>,
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
    pub authority_mode: String,
    pub cursor_version: Option<u64>,
    pub is_at_branch_head: bool,
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
    pub transcript_restore_applied: bool,
    pub workspace_rollback_capable: bool,
    pub workspace_rollback_applied: bool,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub history_state_evidence: Option<Vec<HistoryStateHookEvidence>>,
    pub history_state_audit_summary: HistoryStateAuditSummary,
    pub message_delta: MessageStateDelta,
    pub message_revision: String,
    pub cursor: HistoryCursorState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBranchHeadResponse {
    pub session_id: String,
    pub branch_id: Option<String>,
    pub restored_node_id: Option<String>,
    pub transcript_restore_applied: bool,
    pub workspace_rollback_capable: bool,
    pub workspace_rollback_applied: bool,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub history_state_evidence: Option<Vec<HistoryStateHookEvidence>>,
    pub history_state_audit_summary: HistoryStateAuditSummary,
    pub message_delta: MessageStateDelta,
    pub message_revision: String,
    pub cursor: HistoryCursorState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkFromHistoryNodeResponse {
    pub session_id: String,
    pub node_id: String,
    pub branch: HistoryBranchView,
    pub history_state_evidence: Option<Vec<HistoryStateHookEvidence>>,
    pub history_state_audit_summary: HistoryStateAuditSummary,
    pub message_delta: MessageStateDelta,
    pub message_revision: String,
    pub cursor: HistoryCursorState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchHistoryBranchResponse {
    pub session_id: String,
    pub branch_id: String,
    pub node_id: Option<String>,
    pub history_state_evidence: Option<Vec<HistoryStateHookEvidence>>,
    pub history_state_audit_summary: HistoryStateAuditSummary,
    pub message_delta: MessageStateDelta,
    pub message_revision: String,
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
    pub control_boundary_evidence: Option<GraphRunControlBoundaryEvidence>,
    pub run_control_audit_summary: RunControlAuditSummary,
    pub turn_stop: Option<StopTurnResponse>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunStreamStartResponse {
    pub run: GraphRun,
    pub event: GraphRunEvent,
    pub control_boundary_evidence: Option<GraphRunControlBoundaryEvidence>,
    pub run_control_audit_summary: RunControlAuditSummary,
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
    event_id: Option<String>,
    event_type: Option<String>,
    event_version: Option<String>,
    sequence: Option<u64>,
    emitted_at_ms: Option<u64>,
    session_id: Option<String>,
    turn_id: Option<String>,
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
    hook_trace_records: Option<Vec<crate::agent::hooks::HookTraceRecord>>,
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
        let terminal = self.terminal.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] recording sink lock poisoned: {e}, recovering");
            e.into_inner()
        });
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
            event_id: terminal.event_id.clone(),
            event_type: terminal.event_type.clone(),
            event_version: terminal.event_version.clone(),
            sequence: terminal.sequence,
            emitted_at_ms: terminal.emitted_at_ms,
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
            hook_trace_records: terminal.hook_trace_records.clone().unwrap_or_default(),
            session_summary: terminal
                .session_summary
                .clone()
                .unwrap_or_else(|| fallback_session_summary.clone()),
        })
    }

    fn record_terminal_payload(&self, payload: &TurnStreamEvent) {
        let mut terminal = self.terminal.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] recording sink lock poisoned: {e}, recovering");
            e.into_inner()
        });
        terminal.event_id = payload.event_id.clone();
        terminal.event_type = payload.event_type.clone();
        terminal.event_version = payload.event_version.clone();
        terminal.sequence = payload.sequence;
        terminal.emitted_at_ms = payload.emitted_at_ms;
        terminal.session_id = payload.session_id.clone();
        terminal.turn_id = Some(payload.turn_id.clone());
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
        terminal.hook_trace_records = payload.hook_trace_records.clone();
        terminal.session_summary = payload.session_summary.clone();
    }

    fn terminal_trace_annotation(
        &self,
    ) -> Option<(
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<u64>,
        Option<u64>,
    )> {
        let terminal = self.terminal.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] recording sink lock poisoned: {e}, recovering");
            e.into_inner()
        });
        Some((
            terminal.session_id.clone(),
            terminal.turn_id.clone()?,
            terminal.event_id.clone(),
            terminal.event_type.clone(),
            terminal.event_version.clone(),
            terminal.sequence,
            terminal.emitted_at_ms,
        ))
    }
}

impl<'a, S: TurnEventSink> TurnEventSink for RecordingTurnEventSink<'a, S> {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.inner.emit(name, payload.clone());
        if matches!(
            name,
            "turn:completed" | "turn:failed" | "turn:cancelled" | "turn:suspended"
        ) {
            self.record_terminal_payload(&payload);
        }
    }
}

pub struct HostControlPlane {
    runtime: RwLock<AgentRuntime>,
    sessions_rwlock: Arc<RwLock<SessionStore>>,
    execution_control: ExecutionControlRegistry,
    graph_runs: Arc<Mutex<GraphRunStore>>,
    graph_runner: GraphRunner,
    graph_planner: Box<dyn GraphPlanner>,
    capability_registry: RwLock<CapabilityRegistry>,
    frontend_diagnostics: FrontendDiagnosticsStore,
    /// Governed dispatcher whose pending control-request store backs the Ask (`Interaction`)
    /// control surface (design.md Decision 5). Shared as `Arc` so the runtime executor and the
    /// host adapter can observe the same store.
    pub ask_dispatcher: Arc<GovernedDispatcher>,
    /// Session-owned revisioned plan store backing the Plan control surface (Decision 6).
    plan_store: PlanStore,
}

pub struct HostControlPlaneBuilder {
    runtime: Option<AgentRuntime>,
    execution_control: Option<ExecutionControlRegistry>,
    graph_runs: Option<GraphRunStore>,
    graph_runner: Option<GraphRunner>,
    graph_planner: Option<Box<dyn GraphPlanner>>,
    ask_dispatcher: Option<Arc<GovernedDispatcher>>,
    plan_store: Option<PlanStore>,
}

impl HostControlPlaneBuilder {
    pub fn new() -> Self {
        Self {
            runtime: None,
            execution_control: None,
            graph_runs: None,
            graph_runner: None,
            graph_planner: None,
            ask_dispatcher: None,
            plan_store: None,
        }
    }

    pub fn desktop() -> Self {
        Self::new().graph_run_store(default_graph_run_store())
    }

    pub fn runtime(mut self, runtime: AgentRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn runtime_builder(mut self, runtime_builder: AgentRuntimeBuilder) -> Self {
        self.runtime = Some(runtime_builder.build());
        self
    }

    pub fn execution_control(mut self, execution_control: ExecutionControlRegistry) -> Self {
        self.execution_control = Some(execution_control);
        self
    }

    pub fn graph_run_store(mut self, graph_runs: GraphRunStore) -> Self {
        self.graph_runs = Some(graph_runs);
        self
    }

    pub fn graph_runner(mut self, graph_runner: GraphRunner) -> Self {
        self.graph_runner = Some(graph_runner);
        self
    }

    pub fn graph_planner(mut self, graph_planner: Box<dyn GraphPlanner>) -> Self {
        self.graph_planner = Some(graph_planner);
        self
    }

    pub fn ask_dispatcher(mut self, ask_dispatcher: Arc<GovernedDispatcher>) -> Self {
        self.ask_dispatcher = Some(ask_dispatcher);
        self
    }

    pub fn plan_store(mut self, plan_store: PlanStore) -> Self {
        self.plan_store = Some(plan_store);
        self
    }

    pub fn build(self) -> HostControlPlane {
        let mut runtime = self.runtime.unwrap_or_else(AgentRuntime::new);
        let capability_registry = runtime.capability_registry_snapshot();
        let sessions_rwlock = runtime.sessions_handle();
        // PA-091：注册事件持久化通道（缓冲 + turn 终态 flush）。
        // 闭包按 turn_id 累积事件，终态（completed/failed/cancelled）时经
        // SessionStore.persist_events 与快照同事务落盘；失败只记日志（contained）。
        {
            let sessions = Arc::clone(&sessions_rwlock);
            let buffer: Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<crate::agent::turn_event::TurnEvent>>>> =
                Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            let persist = move |session_id: &str, turn_id: &str, event: crate::agent::turn_event::TurnEvent, is_terminal: bool| {
                let mut buf = buffer.lock().unwrap_or_else(|e| {
                    eprintln!("[pony-agent][runtime] event buffer lock poisoned: {e}, recovering");
                    e.into_inner()
                });
                // 写时聚合：同一 turn 的连续 AssistantChunk 合并为一条（
                // append-only 不可变 + seq 连续与聚合的一致性由"emit 时合并"保证）。
                if let crate::agent::turn_event::TurnEvent::AssistantChunk { text, .. } = &event {
                    if let Some(last) = buf.get_mut(turn_id).and_then(|v| v.last_mut()) {
                        if let crate::agent::turn_event::TurnEvent::AssistantChunk {
                            text: last_text,
                            ..
                        } = last
                        {
                            last_text.push_str(text);
                            if is_terminal {
                                let events = buf.remove(turn_id).unwrap_or_default();
                                flush_events(&sessions, session_id, turn_id, events);
                            }
                            return;
                        }
                    }
                }
                buf.entry(turn_id.to_string()).or_default().push(event);
                if is_terminal {
                    let events = buf.remove(turn_id).unwrap_or_default();
                    flush_events(&sessions, session_id, turn_id, events);
                }
            };
            crate::agent::turn_flow::register_event_persist(Arc::new(persist));
        }
        // The graph run store and the governed ask dispatcher are shared with the runtime so the
        // Ask control surface (`ask_answer` / `graph_resume_ask`) hits the exact pending-request
        // store and run bindings the runtime's turn loop persists (design.md Decision 5, P1-1).
        let graph_runs = Arc::new(Mutex::new(self.graph_runs.unwrap_or_else(GraphRunStore::new)));
        runtime.set_graph_run_store(Arc::clone(&graph_runs));
        let ask_dispatcher = self
            .ask_dispatcher
            .or_else(|| runtime.governed_dispatcher())
            .unwrap_or_else(default_ask_dispatcher);
        HostControlPlane {
            runtime: RwLock::new(runtime),
            sessions_rwlock,
            execution_control: self
                .execution_control
                .unwrap_or_else(ExecutionControlRegistry::new),
            graph_runs,
            graph_runner: self.graph_runner.unwrap_or_else(GraphRunner::new),
            graph_planner: self
                .graph_planner
                .unwrap_or_else(|| Box::new(DefaultGraphPlanner)),
            capability_registry: RwLock::new(capability_registry),
            frontend_diagnostics: FrontendDiagnosticsStore::new(
                default_frontend_diagnostics_sqlite_path(),
            ),
            ask_dispatcher,
            plan_store: self.plan_store.unwrap_or_else(PlanStore::new),
        }
    }
}

impl Default for HostControlPlaneBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// PA-091：事件缓冲 flush（与快照同事务由 SessionStore.persist_events 保证）。
/// 失败只记日志（contained，不阻断流）；session_id 缺失时丢弃并告警。
/// PA-093：事件携带当前 active 分支（branch_id 落列）；flush 成功后同步
/// 会话水位并升级 branch head 节点为引用化（finalize_event_watermark）。
fn flush_events(
    sessions: &Arc<RwLock<crate::agent::session::SessionStore>>,
    session_id: &str,
    turn_id: &str,
    events: Vec<crate::agent::turn_event::TurnEvent>,
) {
    if events.is_empty() {
        return;
    }
    if session_id.is_empty() {
        eprintln!("[pony-agent][runtime] event flush skipped: missing session_id turn={turn_id}");
        return;
    }
    let mut sessions = sessions.write().unwrap_or_else(|e| {
        eprintln!("[pony-agent][runtime] sessions lock poisoned: {e}, recovering");
        e.into_inner()
    });
    let branch_id = sessions
        .session_active_branch_id(session_id)
        .unwrap_or_else(|| "main".to_string());
    if !sessions.persist_events(session_id, turn_id, &branch_id, events.clone()) {
        eprintln!(
            "[pony-agent][runtime] event flush failed: session={session_id} turn={turn_id} events={} first_type={}",
            events.len(),
            events
                .first()
                .map(|e| e.type_name())
                .unwrap_or("none")
        );
        return;
    }
    // PA-093：水位同步 + 节点引用化升级（幂等；失败只记日志）。
    sessions.finalize_event_watermark(session_id, turn_id);
}

pub struct DesktopHostPreset;

impl DesktopHostPreset {
    pub fn builder() -> HostControlPlaneBuilder {
        HostControlPlaneBuilder::desktop()
    }

    pub fn build() -> HostControlPlane {
        Self::builder().build()
    }
}

impl HostControlPlane {
    pub fn new() -> Self {
        DesktopHostPreset::build()
    }

    pub fn with_runtime(runtime: AgentRuntime) -> Self {
        HostControlPlaneBuilder::new().runtime(runtime).build()
    }

    /// 当前 workspace root（PA-078 附件导入/前端获取根路径；PA-079 注册表落地后改由
    /// 会话 `workspace_id` 解析，此处保留为默认回退）。
    pub fn get_workspace_root(&self) -> String {
        let runtime = self.runtime.read().expect("runtime lock poisoned");
        runtime
            .workspace_root()
            .map(|value| value.to_string())
            .unwrap_or_else(default_workspace_root)
    }

    /// 附件导入（PA-078/PA-079）：base64 解码后由 `attachment_import` 写入受控导入目录。
    /// 目标 root 由 `workspace_id` 经注册表解析（缺省 → 默认 workspace root）；未注册 id → 错误。
    pub fn import_attachment(
        &self,
        name: &str,
        bytes_b64: &str,
        _mime_type: &str,
        workspace_id: Option<&str>,
        root_override: Option<&str>,
        _import_dir_override: Option<&str>,
    ) -> Result<crate::agent::attachment_import::ImportAttachmentResult, String> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(bytes_b64.trim())
            .map_err(|error| format!("[import_attachment_failed] base64 解码失败：{error}"))?;

        let workspace_root = match root_override {
            Some(value) if !value.trim().is_empty() => std::path::PathBuf::from(value),
            _ => {
                let sessions = self.sessions_rwlock.read().unwrap_or_else(|e| {
                    eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                    e.into_inner()
                });
                std::path::PathBuf::from(sessions.resolve_workspace_root(workspace_id)?)
            }
        };
        crate::agent::attachment_import::import_attachment_bytes(name, &bytes, &workspace_root)
    }

    pub fn health_snapshot(&self) -> HostHealthSnapshot {
        let runtime = self.runtime.read().expect("runtime lock poisoned");

        HostHealthSnapshot {
            app_name: "Pony Agent".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            runtime: runtime.name().to_string(),
            graph_engine: runtime.graph_engine().to_string(),
            graph_contract_version: runtime.graph_contract_version().to_string(),
        }
    }

    pub fn run_turn(&self, command: RunTurnCommand) -> TurnResult {
        let runtime = self.runtime.read().expect("runtime lock poisoned");
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
            let runtime = self.runtime.read().expect("runtime lock poisoned");
            runtime.start_graph_run(
                run_id.clone(),
                goal.to_string(),
                command.input.session_id.as_deref(),
            )
        };

        {
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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
            let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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
            if let Some(evidence) = Self::build_graph_run_command_boundary_evidence(
                "resume_graph_run_stream",
                "run_resume",
                CanonicalGraphRunEventType::RunUpdated.as_str(),
                "control_plane.resume_graph_run",
                &lifecycle.run,
                "Graph run resume requested and ready to re-enter turn execution.",
            ) {
                let _ = self.graph_runner.record_control_boundary_evidence(
                    &mut graph_runs,
                    &command.run_id,
                    evidence,
                );
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
            let runtime = self.runtime.read().expect("runtime lock poisoned");
            runtime.start_graph_run(
                run_id.clone(),
                goal.to_string(),
                command.input.session_id.as_deref(),
            )
        };

        {
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
            if graph_runs.load_run(&run_id).is_some() {
                return Err(format!("Graph run `{run_id}` already exists."));
            }
            self.graph_runner.start_run(&mut graph_runs, run);
        }

        self.begin_graph_run_stream(run_id, command.turn_id, command.input, None)
    }

    pub fn prepare_continue_graph_run_stream(
        &self,
        command: ContinueGraphRunStreamCommand,
    ) -> Result<(GraphRunStreamStartResponse, PreparedGraphRunStream), String> {
        let input = {
            let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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

        self.begin_graph_run_stream(command.run_id, command.turn_id, input, None)
    }

    pub fn prepare_resume_graph_run_stream(
        &self,
        command: ResumeGraphRunStreamCommand,
    ) -> Result<(GraphRunStreamStartResponse, PreparedGraphRunStream), String> {
        let (input, control_boundary_evidence) = {
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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
            let evidence = Self::build_graph_run_command_boundary_evidence(
                "resume_graph_run_stream",
                "run_resume",
                CanonicalGraphRunEventType::RunUpdated.as_str(),
                "control_plane.prepare_resume_graph_run_stream",
                &lifecycle.run,
                "Graph run resume requested and ready to re-enter streamed turn execution.",
            );
            if let Some(item) = evidence.as_ref() {
                let _ = self.graph_runner.record_control_boundary_evidence(
                    &mut graph_runs,
                    &command.run_id,
                    item.clone(),
                );
            }
            (input, evidence)
        };

        self.begin_graph_run_stream(
            command.run_id,
            command.turn_id,
            input,
            control_boundary_evidence,
        )
    }

    pub fn execute_graph_run_stream<S: TurnEventSink>(
        &self,
        sink: &S,
        prepared: PreparedGraphRunStream,
    ) -> Result<GraphRunTurnResponse, String> {
        let (turn_result, handoff, decision) = {
            let runtime = self.runtime.read().expect("runtime lock poisoned");
            let recording_sink = RecordingTurnEventSink::new(sink);
            runtime.start_turn_stream_with_control_and_facts(
                &recording_sink,
                &self.execution_control,
                prepared.turn_id.clone(),
                prepared.input.clone(),
                RunTurnFacts {
                    run_id: Some(prepared.run_id.clone()),
                    turn_id: Some(prepared.turn_id.clone()),
                    workspace_root: None,
                },
            );
            if let Some((
                session_id,
                turn_id,
                event_id,
                event_type,
                event_version,
                sequence,
                emitted_at_ms,
            )) = recording_sink.terminal_trace_annotation()
            {
                let _ = runtime.annotate_turn_trace_terminal_event(
                    session_id.as_deref(),
                    &turn_id,
                    event_id,
                    event_type,
                    event_version,
                    sequence,
                    emitted_at_ms,
                );
            }
            let checkpoint = self
                .execution_control
                .load_checkpoint(Some(prepared.turn_id.as_str()), None);
            let run = {
                let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                    eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                    e.into_inner()
                });
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
                    prepared.input.workspace_mode.as_deref(),
                )
                .session_context
                .summary;
            let mut turn_result = recording_sink
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
                prepared.input.workspace_mode.as_deref(),
            );
            if turn_result.phase == SUSPENDED_TURN_PHASE {
                // The runtime already bound the Ask wait (run -> `WaitingUser`) and set the
                // WaitUser decision; no planner decision may run while an Ask wait is unresolved
                // (design.md Decision 5). The graph's `has_unresolved_ask_waits` guard keeps the
                // run in `WaitingUser` until the host answers and `resume_ask_wait` injects the
                // unique terminal result.
                let decision = run.last_decision.clone().unwrap_or_else(|| GraphDecision {
                    kind: GraphDecisionKind::WaitUser,
                    reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                    summary: format!(
                        "Ask awaits a host answer before run `{}` resumes.",
                        prepared.run_id
                    ),
                    target_phase: GraphRunPhase::WaitingUser,
                });
                (turn_result, handoff, decision)
            } else {
                let decision_outcome = runtime.decide_graph_after_turn_with_planner(
                    &run,
                    Some(&prepared.turn_id),
                    prepared.input.session_id.as_deref(),
                    &turn_result,
                    checkpoint.as_ref(),
                    prepared.input.workspace_mode.as_deref(),
                    self.graph_planner.as_ref(),
                )?;
                let decision = decision_outcome.decision;
                if let Some(trace_turn_id) = runtime
                    .load_session_snapshot(run.session_id.as_deref())
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone())
                {
                    if !decision_outcome.trace_records.is_empty() {
                        turn_result
                            .hook_trace_records
                            .extend(decision_outcome.trace_records.clone());
                        let _ = runtime.append_turn_trace_hook_records(
                            run.session_id.as_deref(),
                            &trace_turn_id,
                            decision_outcome.trace_records,
                        );
                    }
                    if let Some(record) = runtime.record_planner_graph_decision_trace(
                        run.session_id.as_deref(),
                        &trace_turn_id,
                        &run,
                        &decision,
                    ) {
                        turn_result.hook_trace_records.push(record);
                    }
                }
                (turn_result, handoff, decision)
            }
        };

        if turn_result.phase == SUSPENDED_TURN_PHASE {
            // The run is already `WaitingUser` with a bound Ask wait (design.md Decision 5): the
            // runtime set it via `bind_ask_wait` before the turn returned. No turn-result
            // application may run while the wait is unresolved (`apply_turn_result` rejects it
            // via `has_unresolved_ask_waits`), so return the suspension snapshot directly.
            let run = {
                let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                    eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                    e.into_inner()
                });
                graph_runs.load_run(&prepared.run_id).ok_or_else(|| {
                    format!(
                        "Graph run `{}` failed to load after Ask suspension.",
                        prepared.run_id
                    )
                })?
            };
            let event = GraphRunEvent {
                run_id: run.id.clone(),
                kind: GraphRunEventKind::Updated,
                phase: GraphRunPhase::WaitingUser,
                summary: format!(
                    "Ask awaits a host answer before run `{}` resumes.",
                    prepared.run_id
                ),
                step_count: run.steps.len(),
                updated_at_ms: run.updated_at_ms,
                hook_point: None,
                canonical_event_type: None,
                canonical_phase: None,
            };
            return Ok(GraphRunTurnResponse {
                run,
                handoff,
                decision,
                event,
                turn_result,
            });
        }

        let advance = {
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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
        self.execution_control.register_turn(
            &command.turn_id,
            command.input.session_id.as_deref(),
            None,
        );

        let runtime = self.runtime.read().expect("runtime lock poisoned");
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
            let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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

        let (lifecycle, control_boundary_evidence) = {
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
            let lifecycle = self
                .graph_runner
                .request_stop(
                    &mut graph_runs,
                    &command.run_id,
                    GraphRunStopReason::UserStop,
                    "Graph run stopped and waiting to resume.",
                )
                .ok_or_else(|| format!("Graph run `{}` cannot be stopped.", command.run_id))?;
            let evidence = Self::build_graph_run_command_boundary_evidence(
                "stop_graph_run",
                "stop_requested",
                CanonicalGraphRunEventType::StopRequested.as_str(),
                "control_plane.stop_graph_run",
                &lifecycle.run,
                "Graph run stop requested and awaiting resume handling.",
            );
            let persisted_evidence = evidence.as_ref().and_then(|item| {
                self.graph_runner.record_control_boundary_evidence(
                    &mut graph_runs,
                    &command.run_id,
                    item.clone(),
                )
            });
            let lifecycle = if let Some(run) = persisted_evidence {
                crate::agent::graph::GraphRunLifecycle {
                    run,
                    event: lifecycle.event,
                }
            } else {
                lifecycle
            };
            (lifecycle, evidence)
        };

        let checkpoint = lifecycle.run.active_turn_id.as_ref().and_then(|turn_id| {
            self.load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: Some(turn_id.clone()),
                session_id: lifecycle.run.session_id.clone(),
            })
        });
        Ok(GraphRunControlResponse {
            run_control_audit_summary: Self::project_run_control_audit_summary(
                Some(&lifecycle.run),
                checkpoint.as_ref(),
                None,
                control_boundary_evidence.as_ref(),
            ),
            run: lifecycle.run,
            event: lifecycle.event,
            control_boundary_evidence,
            turn_stop,
        })
    }

    fn begin_graph_run_stream(
        &self,
        run_id: String,
        turn_id: String,
        input: TurnInput,
        control_boundary_evidence: Option<GraphRunControlBoundaryEvidence>,
    ) -> Result<(GraphRunStreamStartResponse, PreparedGraphRunStream), String> {
        self.execution_control
            .register_turn(&turn_id, input.session_id.as_deref(), Some(&run_id));

        let lifecycle = {
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
            self.graph_runner
                .begin_turn(
                    &mut graph_runs,
                    &run_id,
                    &turn_id,
                    input.session_id.as_deref(),
                )
                .ok_or_else(|| format!("Graph run `{run_id}` cannot accept a new turn."))?
        };

        let checkpoint = self.load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: Some(turn_id.clone()),
            session_id: input.session_id.clone(),
        });
        let submission_plan = Some(Self::build_graph_run_submission_plan(
            control_boundary_evidence
                .as_ref()
                .map(|item| {
                    item.hook_envelope
                        .command
                        .as_submission_command()
                        .to_string()
                })
                .unwrap_or_else(|| "start_graph_run_stream".to_string()),
            Some(lifecycle.run.id.clone()),
            "response",
            checkpoint.as_ref(),
        ));
        let response = GraphRunStreamStartResponse {
            control_boundary_evidence: control_boundary_evidence.clone(),
            run_control_audit_summary: Self::project_run_control_audit_summary(
                Some(&lifecycle.run),
                checkpoint.as_ref(),
                submission_plan.as_ref(),
                control_boundary_evidence.as_ref(),
            ),
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
                let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                    eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                    e.into_inner()
                });
                let run = graph_runs
                    .load_run(&run_id)
                    .ok_or_else(|| format!("Graph run `{run_id}` cannot accept a new turn."))?;
                format!("{}-turn-{}", run.id, run.steps.len() + 1)
            };
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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
            let runtime = self.runtime.read().expect("runtime lock poisoned");
            let mut turn_result = runtime.run_turn_with_facts(
                input.clone(),
                RunTurnFacts {
                    run_id: Some(run_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    workspace_root: None,
                },
            );
            let run = {
                let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                    eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                    e.into_inner()
                });
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
                input.workspace_mode.as_deref(),
            );
            if turn_result.phase == SUSPENDED_TURN_PHASE {
                // The runtime already bound the Ask wait (run -> `WaitingUser`) and set the
                // WaitUser decision; no planner decision or turn-result application may run while
                // an Ask wait is unresolved (design.md Decision 5).
                let decision = run.last_decision.clone().unwrap_or_else(|| GraphDecision {
                    kind: GraphDecisionKind::WaitUser,
                    reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                    summary: format!("Ask awaits a host answer before run `{run_id}` resumes."),
                    target_phase: GraphRunPhase::WaitingUser,
                });
                (turn_result, handoff, decision)
            } else {
                let decision_outcome = runtime.decide_graph_after_turn_with_planner(
                    &run,
                    Some(&turn_id),
                    input.session_id.as_deref(),
                    &turn_result,
                    None,
                    input.workspace_mode.as_deref(),
                    self.graph_planner.as_ref(),
                )?;
                let decision = decision_outcome.decision;
                if let Some(trace_turn_id) = runtime
                    .load_session_snapshot(run.session_id.as_deref())
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone())
                {
                    if !decision_outcome.trace_records.is_empty() {
                        turn_result
                            .hook_trace_records
                            .extend(decision_outcome.trace_records.clone());
                        let _ = runtime.append_turn_trace_hook_records(
                            run.session_id.as_deref(),
                            &trace_turn_id,
                            decision_outcome.trace_records,
                        );
                    }
                    if let Some(record) = runtime.record_planner_graph_decision_trace(
                        run.session_id.as_deref(),
                        &trace_turn_id,
                        &run,
                        &decision,
                    ) {
                        turn_result.hook_trace_records.push(record);
                    }
                }
                (turn_result, handoff, decision)
            }
        };

        if turn_result.phase == SUSPENDED_TURN_PHASE {
            let run = {
                let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                    eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                    e.into_inner()
                });
                graph_runs.load_run(&run_id).ok_or_else(|| {
                    format!("Graph run `{run_id}` failed to load after Ask suspension.")
                })?
            };
            let event = GraphRunEvent {
                run_id: run.id.clone(),
                kind: GraphRunEventKind::Updated,
                phase: GraphRunPhase::WaitingUser,
                summary: format!("Ask awaits a host answer before run `{run_id}` resumes."),
                step_count: run.steps.len(),
                updated_at_ms: run.updated_at_ms,
                hook_point: None,
                canonical_event_type: None,
                canonical_phase: None,
            };
            return Ok(GraphRunTurnResponse {
                run,
                handoff,
                decision,
                event,
                turn_result,
            });
        }

        let advance = {
            let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
                eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
                e.into_inner()
            });
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

/// Default governed dispatcher backing the Ask control surface. Built from the builtin registry
/// so descriptor/snapshot/digest facts are real; the runtime may swap in the shared dispatcher it
/// executes through via [`HostControlPlaneBuilder::ask_dispatcher`] so both observe the same
/// pending-request store.
fn default_ask_dispatcher() -> Arc<GovernedDispatcher> {
    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin registry must validate"),
    );
    let clock: Arc<dyn RuntimeClock> = Arc::new(SystemClock);
    Arc::new(GovernedDispatcher::new(registry, clock))
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

fn validate_skill_source_view(source: &SkillSourceView) -> Result<(), String> {
    if source.source_id.trim().is_empty() {
        return Err("Skill source id is empty.".to_string());
    }
    if source.display_name.trim().is_empty() {
        return Err(format!(
            "Skill source `{}` is missing display_name.",
            source.source_id
        ));
    }
    if source.transport_kind.trim().is_empty() {
        return Err(format!(
            "Skill source `{}` is missing transport_kind.",
            source.source_id
        ));
    }
    if source.server_identity.trim().is_empty() {
        return Err(format!(
            "Skill source `{}` is missing server_identity.",
            source.source_id
        ));
    }
    Ok(())
}

fn validate_skill_descriptor(skill: &SkillDescriptor) -> Result<(), String> {
    if skill.skill_id.trim().is_empty() {
        return Err("Skill id is empty.".to_string());
    }
    if skill.source_id.trim().is_empty() {
        return Err(format!("Skill `{}` is missing source_id.", skill.skill_id));
    }
    if skill.label.trim().is_empty() {
        return Err(format!("Skill `{}` is missing label.", skill.skill_id));
    }
    if skill.composed_capability_refs.is_empty() {
        return Err(format!(
            "Skill `{}` must reference at least one capability.",
            skill.skill_id
        ));
    }
    Ok(())
}

fn validate_skill_source_snapshot(snapshot: &SkillSourceSnapshot) -> Result<(), String> {
    validate_skill_source_view(&snapshot.source)?;
    for skill in &snapshot.skills {
        validate_skill_descriptor(skill)?;
        if skill.source_id != snapshot.source.source_id {
            return Err(format!(
                "Skill `{}` does not belong to source `{}`.",
                skill.skill_id, snapshot.source.source_id
            ));
        }
    }
    Ok(())
}

fn normalize_skill_source_snapshot_against_capabilities(
    registry: &CapabilityRegistry,
    snapshot: SkillSourceSnapshot,
) -> Result<SkillSourceSnapshot, String> {
    registry.normalize_skill_source_snapshot(snapshot)
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
    hook_call_count: u64,
    blocked_hook_count: u64,
    failed_request_count: u64,
    retrieval_participation_count: u64,
    input_tokens: u64,
    cache_hit_input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    total_hook_duration_ms: u64,
    first_token_latency: AverageAccumulator,
    turn_duration: AverageAccumulator,
    hook_duration: AverageAccumulator,
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
        self.cache_hit_input_tokens += provider_returned_cache_hit_input_tokens(trace);
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
        self.failed_call_count += u64::from(activity.status == "error");
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
        self.failed_call_count += u64::from(activity.status == "error");
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

#[derive(Clone, Debug, Default)]
struct HookAggregate {
    key: String,
    label: String,
    call_count: u64,
    blocked_call_count: u64,
    total_duration_ms: u64,
    duration: AverageAccumulator,
}

impl HookAggregate {
    fn new(key: String, label: String) -> Self {
        Self {
            key,
            label,
            ..Self::default()
        }
    }

    fn add_trace(&mut self, trace: &crate::agent::hooks::HookTraceRecord) {
        self.call_count += 1;
        self.blocked_call_count += u64::from(trace.blocked);
        self.duration.push(Some(trace.elapsed_ms));
        self.total_duration_ms += trace.elapsed_ms;
    }

    fn into_row(self) -> ModelMonitorHookRow {
        ModelMonitorHookRow {
            key: self.key,
            label: self.label,
            call_count: self.call_count,
            blocked_call_count: self.blocked_call_count,
            avg_duration_ms: self.duration.average(),
            total_duration_ms: self.total_duration_ms,
        }
    }
}

fn aggregate_session_metrics(session: &SessionSnapshot) -> SessionMetricsAggregate {
    let mut aggregate = SessionMetricsAggregate::default();
    for trace in &session.turn_trace_history {
        if !trace_has_canonical_terminal_envelope(trace) {
            continue;
        }
        aggregate.request_count += 1;
        aggregate.model_call_count += count_model_calls(trace);
        aggregate.tool_call_count += count_tool_calls(trace);
        aggregate.hook_call_count += trace.hook_trace_records.len() as u64;
        aggregate.blocked_hook_count += trace
            .hook_trace_records
            .iter()
            .filter(|record| record.blocked)
            .count() as u64;
        aggregate.failed_request_count += u64::from(trace.error.is_some());
        aggregate.retrieval_participation_count += u64::from(trace_uses_retrieval(trace));
        aggregate.input_tokens += trace.input_tokens.unwrap_or(0);
        aggregate.cache_hit_input_tokens += provider_returned_cache_hit_input_tokens(trace);
        aggregate.output_tokens += trace.output_tokens.unwrap_or(0);
        aggregate.total_tokens += trace.total_tokens.unwrap_or(0);
        for hook_trace in &trace.hook_trace_records {
            aggregate.hook_duration.push(Some(hook_trace.elapsed_ms));
            aggregate.total_hook_duration_ms += hook_trace.elapsed_ms;
        }
        aggregate
            .first_token_latency
            .push(trace.first_token_latency_ms);
        aggregate.turn_duration.push(trace.turn_duration_ms);
    }
    aggregate
}

fn trace_has_canonical_terminal_envelope(trace: &TurnTraceRecord) -> bool {
    let has_event_id = trace
        .event_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    let has_event_version = trace
        .event_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    let is_terminal_event_type = matches!(
        trace.event_type.as_deref().map(str::trim),
        Some("turn.completed" | "turn.failed" | "turn.cancelled")
    );

    has_event_id
        && has_event_version
        && is_terminal_event_type
        && trace.sequence.is_some()
        && trace.emitted_at_ms.is_some()
}

fn merge_monitor_overview(
    overview: &mut ModelMonitorOverview,
    metrics: &SessionMetricsAggregate,
    first_token_latency: &mut AverageAccumulator,
    turn_duration: &mut AverageAccumulator,
    hook_duration: &mut AverageAccumulator,
) {
    overview.request_count += metrics.request_count;
    overview.model_call_count += metrics.model_call_count;
    overview.tool_call_count += metrics.tool_call_count;
    overview.hook_call_count += metrics.hook_call_count;
    overview.blocked_hook_count += metrics.blocked_hook_count;
    overview.failed_request_count += metrics.failed_request_count;
    overview.retrieval_participation_count += metrics.retrieval_participation_count;
    overview.input_tokens += metrics.input_tokens;
    overview.cache_hit_input_tokens += metrics.cache_hit_input_tokens;
    overview.output_tokens += metrics.output_tokens;
    overview.total_tokens += metrics.total_tokens;
    overview.total_hook_duration_ms += metrics.total_hook_duration_ms;
    first_token_latency.sum += metrics.first_token_latency.sum;
    first_token_latency.count += metrics.first_token_latency.count;
    turn_duration.sum += metrics.turn_duration.sum;
    turn_duration.count += metrics.turn_duration.count;
    hook_duration.sum += metrics.hook_duration.sum;
    hook_duration.count += metrics.hook_duration.count;
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

fn provider_returned_cache_hit_input_tokens(trace: &crate::agent::session::TurnTraceRecord) -> u64 {
    trace
        .provider_call_records
        .iter()
        .filter_map(|record| record.cache_hit_input_tokens)
        .sum()
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

fn append_hook_class_aggregates(
    trace: &crate::agent::session::TurnTraceRecord,
    hook_classes: &mut std::collections::BTreeMap<String, HookAggregate>,
) {
    for hook_trace in &trace.hook_trace_records {
        let key = hook_class_key(&hook_trace.hook_class).to_string();
        hook_classes
            .entry(key.clone())
            .or_insert_with(|| HookAggregate::new(key.clone(), key.clone()))
            .add_trace(hook_trace);
    }
}

fn append_hook_aggregates(
    trace: &crate::agent::session::TurnTraceRecord,
    hooks: &mut std::collections::BTreeMap<String, HookAggregate>,
) {
    for hook_trace in &trace.hook_trace_records {
        let key = hook_trace.hook_name.clone();
        hooks
            .entry(key.clone())
            .or_insert_with(|| HookAggregate::new(key.clone(), hook_trace.hook_name.clone()))
            .add_trace(hook_trace);
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

fn append_skill_aggregates(
    trace: &crate::agent::session::TurnTraceRecord,
    skill_selections: &mut std::collections::BTreeMap<String, ActivityAggregate>,
    skill_sources: &mut std::collections::BTreeMap<String, ActivityAggregate>,
    skill_failure_layers: &mut std::collections::BTreeMap<String, ActivityAggregate>,
) {
    for activity in &trace.tool_activities {
        let Some(invocation) = activity.capability_invocation.as_ref() else {
            continue;
        };

        let Some(skill_id) = invocation
            .skill_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let skill_key = skill_id.to_string();
        skill_selections
            .entry(skill_key.clone())
            .or_insert_with(|| ActivityAggregate::new(skill_key.clone(), skill_key.clone()))
            .add_activity(activity);

        let source_key = invocation
            .skill_source_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown-skill-source")
            .to_string();
        skill_sources
            .entry(source_key.clone())
            .or_insert_with(|| ActivityAggregate::new(source_key.clone(), source_key.clone()))
            .add_activity(activity);

        let failure_key = invocation
            .failure_layer
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("ok")
            .to_string();
        skill_failure_layers
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

fn hook_row_sort(left: &ModelMonitorHookRow, right: &ModelMonitorHookRow) -> Ordering {
    right
        .call_count
        .cmp(&left.call_count)
        .then_with(|| left.label.cmp(&right.label))
}

fn hook_class_key(hook_class: &crate::agent::hooks::HookClass) -> &'static str {
    match hook_class {
        crate::agent::hooks::HookClass::Observe => "observe",
        crate::agent::hooks::HookClass::Guard => "guard",
        crate::agent::hooks::HookClass::Transform => "transform",
        crate::agent::hooks::HookClass::SideEffect => "side_effect",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{
        ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
        ThinkingParamPattern,
    };
    use crate::agent::context::DefaultTurnContextBuilder;
    use crate::agent::dispatcher::DispatchContext;
    use crate::agent::graph::{
        GraphAskWaitBinding, GraphDecisionKind, GraphDecisionReason, GraphEngine, GraphRunEventKind,
        GraphRunPhase, GraphRunStore, GraphRunner,
    };
    use crate::agent::hooks::{
        turn_hook_point_for_planner_hook_point, AgentHookDescriptor, AgentHookExecutor,
        HistoryStateHookEnvelope, HistoryStateHookExecutor, HistoryStateHookPoint, HookClass,
        HookDenyDecision, HookFailurePolicy, HookPatchOperation, HookPatchOperationKind,
        HookPatchTarget, HookRecoveryMode, HookReplayRequirements, HookResultKind,
        HookSideEffectPersistenceRequirements, HookStructuredResult, HookTraceRequirements,
        NoopHookExecutor, PlannerFactsEnvelope, PlannerHookPoint, TurnHookPoint,
    };
    use crate::agent::planner::{LocalTurnPlanner, TurnPlanner};
    use crate::agent::provider::ProviderAuthType;
    use crate::agent::provider::ProviderProtocol;
    use crate::agent::runtime::TurnStreamEvent;
    use crate::agent::session::SessionStore;
    use crate::agent::telemetry::DefaultTurnTelemetryBuilder;
    use crate::agent::tool_runtime::{
        FakeClock, InvocationOrigin, PendingControlRequestKind, PendingControlRequestState,
        ToolDispatchRequest,
    };
    use crate::agent::tools::ToolRouter;
    use crate::agent::tools::{
        ToolCall, ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy,
        ToolExposure, ToolHandlerProvenance, ToolIdentity, ToolKind, ToolPermissionDeclaration,
        ToolPlan, ToolRegistrySnapshot,
    };
    use crate::agent::turn_flow::TurnEventSink;
    use serde_json::{json, Value};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct NoopSink;

    struct StaticHistoryStateHookExecutor {
        start_results: Vec<crate::agent::hooks::HookExecutionResult>,
        resolved_results: Vec<crate::agent::hooks::HookExecutionResult>,
    }

    impl HistoryStateHookExecutor for StaticHistoryStateHookExecutor {
        fn execute(
            &self,
            envelope: &HistoryStateHookEnvelope,
        ) -> Result<Vec<crate::agent::hooks::HookExecutionResult>, String> {
            Ok(match envelope.hook_point {
                HistoryStateHookPoint::HistoryCheckoutStart
                | HistoryStateHookPoint::BranchRestoreStart
                | HistoryStateHookPoint::BranchForkStart
                | HistoryStateHookPoint::BranchSwitchStart => self.start_results.clone(),
                HistoryStateHookPoint::HistoryCheckoutResolved
                | HistoryStateHookPoint::BranchRestoreResolved
                | HistoryStateHookPoint::BranchForkResolved
                | HistoryStateHookPoint::BranchSwitchResolved => self.resolved_results.clone(),
            })
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

    struct MockHttpResponse {
        content_type: &'static str,
        body: String,
    }

    struct MockHttpServer {
        pending: Arc<Mutex<Vec<MockHttpResponse>>>,
        base_url: String,
        address: String,
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
                pending,
                base_url: format!("http://{}/v1", address),
                address: address.to_string(),
                handle: Some(handle),
            }
        }

        fn finish(mut self) {
            let remaining = self.pending.lock().expect("mock response lock").len();
            for _ in 0..remaining {
                if let Ok(mut stream) = TcpStream::connect(&self.address) {
                    let _ = stream.write_all(
                        b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}",
                    );
                }
            }
            if let Some(handle) = self.handle.take() {
                handle.join().expect("join mock server");
            }
            assert_eq!(
                remaining, 0,
                "mock server finished with {remaining} unconsumed responses"
            );
        }
    }

    struct TransformingPlannerHookExecutor;
    struct BlockingSkillSourceIngressHookExecutor;

    impl AgentHookExecutor for TransformingPlannerHookExecutor {
        fn execute(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: TurnHookPoint,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            NoopHookExecutor.execute(descriptor, hook_point)
        }

        fn execute_planner(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: PlannerHookPoint,
            envelope: &PlannerFactsEnvelope,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            let operations = match hook_point {
                PlannerHookPoint::GraphDecision => vec![HookPatchOperation {
                    target: HookPatchTarget::PlannerFacts,
                    path: "decision_summary".to_string(),
                    operation: HookPatchOperationKind::Set,
                    value_summary: Some("rewrite graph decision summary".to_string()),
                    value_text: Some("\"planner summary patched by hook\"".to_string()),
                }],
                _ => Vec::new(),
            };
            Ok(crate::agent::hooks::HookExecutionResult {
                hook_name: descriptor.name.clone(),
                hook_class: HookClass::Transform,
                hook_point: turn_hook_point_for_planner_hook_point(&hook_point),
                hook_order: 0,
                result_kind: HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch { operations },
                blocked: false,
                elapsed_ms: 0,
                input_summary: envelope.current_decision_summary.clone(),
                persistence_evidence_ref: None,
                trace_summary: format!("hook rewrote planner payload at {:?}", hook_point),
            })
        }
    }

    impl AgentHookExecutor for BlockingSkillSourceIngressHookExecutor {
        fn execute(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: TurnHookPoint,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            NoopHookExecutor.execute(descriptor, hook_point)
        }

        fn execute_capability_mediation(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: crate::agent::hooks::CapabilityMediationHookPoint,
            envelope: &crate::agent::hooks::CapabilityMediationEnvelope,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            match hook_point {
                crate::agent::hooks::CapabilityMediationHookPoint::SkillSourceIngress => {
                    crate::agent::hooks::HookExecutionResult::guard_decision(
                        descriptor,
                        crate::agent::hooks::TurnHookPoint::SkillSourceIngress,
                        HookStructuredResult::Deny(HookDenyDecision {
                            reason_code: "skill_source_blocked".to_string(),
                            message: "skill source ingress blocked by hook".to_string(),
                        }),
                        Some(envelope.source_id.clone().unwrap_or_default()),
                    )
                }
                _ => NoopHookExecutor.execute(
                    descriptor,
                    crate::agent::hooks::turn_hook_point_for_capability_mediation_hook_point(
                        &hook_point,
                    ),
                ),
            }
        }
    }

    fn transform_hook_descriptor(
        name: &str,
        priority: i32,
        hook_point: TurnHookPoint,
    ) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: "agent-hooks-v1".to_string(),
            name: name.to_string(),
            class: HookClass::Transform,
            priority,
            timeout_ms: 1_000,
            allowed_hook_points: vec![hook_point],
            allowed_result_kinds: vec![HookResultKind::Patch],
            can_block: false,
            default_failure_policy: HookFailurePolicy::Ignore,
            allowed_failure_policies: vec![HookFailurePolicy::Ignore, HookFailurePolicy::FailTurn],
            default_recovery_mode: HookRecoveryMode::ReplayRequired,
            trace_requirements: HookTraceRequirements {
                include_name: true,
                include_hook_point: true,
                include_elapsed_ms: true,
                include_result_summary: true,
            },
            replay_requirements: HookReplayRequirements {
                include_hook_order: true,
                include_input_summary: true,
            },
            side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
                require_persistence_evidence: false,
                require_effect_summary: false,
            },
        }
    }

    fn guard_hook_descriptor(
        name: &str,
        priority: i32,
        hook_point: TurnHookPoint,
    ) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: "agent-hooks-v1".to_string(),
            name: name.to_string(),
            class: HookClass::Guard,
            priority,
            timeout_ms: 1_000,
            allowed_hook_points: vec![hook_point],
            allowed_result_kinds: vec![HookResultKind::Allow, HookResultKind::Deny],
            can_block: true,
            default_failure_policy: HookFailurePolicy::FailTurn,
            allowed_failure_policies: vec![HookFailurePolicy::Ignore, HookFailurePolicy::FailTurn],
            default_recovery_mode: HookRecoveryMode::ReplayRequired,
            trace_requirements: HookTraceRequirements {
                include_name: true,
                include_hook_point: true,
                include_elapsed_ms: true,
                include_result_summary: true,
            },
            replay_requirements: HookReplayRequirements {
                include_hook_order: true,
                include_input_summary: true,
            },
            side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
                require_persistence_evidence: false,
                require_effect_summary: false,
            },
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
            auth_type: ProviderAuthType::Auto,
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
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        }
    }

    /// 非 reasoning 模型配置：走 planner 预检决策 + tool-selection 修补流程（Ask stream 测试用，
    /// 与 runtime 测试的 `test_chat_provider_selection` 一致）。
    fn test_chat_provider_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "test-openai-chat".to_string(),
            provider_name: "test-openai-chat".to_string(),
            protocol: ProviderProtocol::OpenAi,
            base_url,
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "TEST_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "gpt-4.1-mini".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: false,
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::EffortStandard,
        }
    }

    fn build_test_control_plane(
        responses: Vec<MockHttpResponse>,
    ) -> (
        HostControlPlane,
        MockHttpServer,
        crate::agent::runtime_helper::TestRuntimeGuard,
    ) {
        let rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
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
        (HostControlPlane::with_runtime(runtime), server, rt_guard)
    }

    fn temp_sessions_path() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("pony-agent-control-plane-test-{stamp}"))
            .join("sessions.json")
    }

    impl TurnEventSink for NoopSink {
        fn emit(&self, _name: &str, _payload: TurnStreamEvent) {}
    }

    #[test]
    fn import_attachment_resolves_workspace_via_registry() {
        // PA-078×PA-079：workspace_id 经注册表解析目标 root；未注册 id → 明确错误。
        let control_plane = HostControlPlane::with_runtime(AgentRuntime::new());

        // 未注册 workspace → 明确错误
        let err = control_plane
            .import_attachment("a.txt", "aGVsbG8=", "text/plain", Some("ws-nope"), None, None)
            .expect_err("未注册 workspace 应被拒绝");
        assert!(err.contains("workspace 不存在"), "err={err}");

        // root_override 注入（测试接缝）仍生效
        let root = std::env::temp_dir().join(format!("pa079-cp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ok = control_plane.import_attachment(
            "a.txt",
            "aGVsbG8=",
            "text/plain",
            Some("default"),
            Some(root.to_str().unwrap()),
            None,
        );
        assert!(ok.is_ok(), "默认 workspace + root_override 应被允许: {ok:?}");
        let _ = std::fs::remove_dir_all(&root);

        // 注册一个 workspace 后，import 落其 root
        let ws_root = std::env::temp_dir().join(format!("pa079-ws-import-{}", std::process::id()));
        std::fs::create_dir_all(&ws_root).unwrap();
        let created = control_plane.create_workspace("Import Target", ws_root.to_str().unwrap()).unwrap();
        let imported = control_plane
            .import_attachment("doc.pdf", "aGVsbG8=", "application/pdf", Some(&created.id), None, None)
            .unwrap();
        assert!(imported.path.starts_with(ws_root.join(".tmp/imports")));
        assert_eq!(
            imported.relative_path.as_deref(),
            Some(".tmp/imports/doc.pdf")
        );
        let _ = std::fs::remove_dir_all(&ws_root);
    }

    #[test]
    fn recording_turn_event_sink_uses_fallback_summary_when_terminal_event_has_none() {
        let sink = NoopSink;
        let recording_sink = RecordingTurnEventSink::new(&sink);

        recording_sink.emit(
            "turn:completed",
            TurnStreamEvent {
                event_id: Some("turn-fallback:1".to_string()),
                session_id: Some("session-fallback".to_string()),
                turn_id: "turn-fallback".to_string(),
                kind: "completed".to_string(),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(1),
                emitted_at_ms: Some(1),
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
                hook_trace_records: None,
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
                    workspace_mode: None,
                    session_id: Some("session-fallback".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
                "retrieval summary fallback".to_string(),
            )
            .expect("turn result should be reconstructed");

        assert_eq!(result.event_id.as_deref(), Some("turn-fallback:1"));
        assert_eq!(result.event_type.as_deref(), Some("turn.completed"));
        assert_eq!(result.sequence, Some(1));
        assert_eq!(result.emitted_at_ms, Some(1));
        assert_eq!(result.session_summary, "retrieval summary fallback");
        assert_eq!(result.assistant_message, "streamed reply");
        assert_eq!(result.phase, "ready");
    }

    #[test]
    fn inspection_can_join_turn_and_session_views() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("inspected")]);
        control_plane.execution_control.register_turn(
            "turn-inspect",
            Some("session-inspect"),
            None,
        );

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "check Cargo.toml".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("session-inspect".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("runtime-view")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请继续推进 PA-018。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("session-runtime-view".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
        let checkpoint = view
            .checkpoint
            .expect("completed session should expose lifecycle boundary checkpoint");
        assert_eq!(checkpoint.checkpoint_kind, "lifecycle_boundary");
        assert_eq!(checkpoint.recovery_mode, "replay_required");
        assert!(!checkpoint.resumable);
        assert!(!checkpoint.replayable);
        assert_eq!(checkpoint.phase, "checkpointing");
        assert_eq!(checkpoint.status, "completed");
        assert_eq!(checkpoint.projected_runtime_phase, "connecting");
        server.finish();
    }

    #[test]
    fn session_runtime_view_reads_runtime_generated_hook_traces_and_metrics() {
        let control_plane = HostControlPlane::new();
        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.record_turn_trace_for_test(
                Some("session-hook-view"),
                crate::agent::session::TurnTraceRecord {
                    turn_id: "turn-hook-view".to_string(),
                    session_id: Some("session-hook-view".to_string()),
                    event_id: Some("turn-hook-view:1".to_string()),
                    event_type: Some("turn.completed".to_string()),
                    event_version: Some("turn-event-v1".to_string()),
                    sequence: Some(1),
                    emitted_at_ms: Some(1000),
                    title: "runtime hook trace view".to_string(),
                    phase: "completed".to_string(),
                    trace_steps: vec![crate::agent::telemetry::TurnTraceStep {
                        id: "step-return".to_string(),
                        label: "Return result".to_string(),
                        state: "completed".to_string(),
                    }],
                    trace_timeline: Vec::new(),
                    tool_activities: Vec::new(),
                    provider_call_records: Vec::new(),
                    hook_trace_records: vec![crate::agent::hooks::HookTraceRecord {
                        hook_name: "audit.observe".to_string(),
                        hook_class: crate::agent::hooks::HookClass::Observe,
                        hook_point: crate::agent::hooks::TurnHookPoint::ModelCallStart,
                        hook_order: 1,
                        result_kind: crate::agent::hooks::HookResultKind::Observe,
                        structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                            summary: "runtime observed model boundary".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 2,
                        input_summary: Some("runtime".to_string()),
                        persistence_evidence_ref: None,
                        summary: "runtime hook summary".to_string(),
                    }],
                    provider_requested_name: Some("test-openai".to_string()),
                    provider_name: Some("test-openai".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5.4".to_string()),
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    session_summary: Some("runtime hook trace".to_string()),
                    fallback_reason: None,
                    error: None,
                    input_tokens: Some(10),
                    cache_hit_input_tokens: Some(4),
                    reasoning_tokens: Some(2),
                    output_tokens: Some(12),
                    total_tokens: Some(22),
                    first_token_latency_ms: Some(100),
                    turn_duration_ms: Some(500),
                    updated_at: 0,
                },
            );
        }

        let view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("session-hook-view".to_string()),
            ..SessionRuntimeViewQuery::default()
        });

        assert_eq!(view.session.conversation_id, "session-hook-view");
        assert_eq!(view.session.turn_trace_history.len(), 1);
        assert_eq!(
            view.session.turn_trace_history[0].hook_trace_records.len(),
            1
        );
        assert_eq!(
            view.session.turn_trace_history[0].hook_trace_records[0].hook_name,
            "audit.observe"
        );

        let drilldown =
            control_plane.load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery {
                session_id: "session-hook-view".to_string(),
            });
        assert_eq!(drilldown.metrics.hook_call_count, 1);
    }

    #[test]
    fn model_monitor_session_drilldown_preserves_failed_and_cancelled_terminal_evidence() {
        let control_plane = HostControlPlane::new();
        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.record_turn_trace_for_test(
                Some("session-terminal-evidence"),
                crate::agent::session::TurnTraceRecord {
                    turn_id: "turn-failed-evidence".to_string(),
                    session_id: Some("session-terminal-evidence".to_string()),
                    event_id: Some("turn-failed-evidence:4".to_string()),
                    event_type: Some("turn.failed".to_string()),
                    event_version: Some("turn-event-v1".to_string()),
                    sequence: Some(4),
                    emitted_at_ms: Some(4004),
                    title: "failed evidence".to_string(),
                    phase: "failed".to_string(),
                    trace_steps: vec![crate::agent::telemetry::TurnTraceStep {
                        id: "step-return".to_string(),
                        label: "Return result".to_string(),
                        state: "error".to_string(),
                    }],
                    trace_timeline: Vec::new(),
                    tool_activities: vec![crate::agent::telemetry::TurnToolActivity {
                        id: "tool-failed-1".to_string(),
                        name: "workspace_list_files".to_string(),
                        canonical_tool_name: Some("List".to_string()),
                        display_name_zh: Some("列表".to_string()),
                        status: "completed".to_string(),
                        description: "tool completed before failure".to_string(),
                        arguments_text: Some("{\"path\":\".\"}".to_string()),
                        result_text: Some("ok".to_string()),
                        duration_seconds: Some(0.2),
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
                        capability_invocation: None,
                    }],
                    provider_call_records: vec![crate::agent::telemetry::ProviderCallCacheRecord {
                        request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                        provider_source: Some("provider_decision".to_string()),
                        provider_mode: Some("live".to_string()),
                        input_tokens: Some(12),
                        cache_hit_input_tokens: Some(4),
                        cache_hit_source: None,
                        cache_miss_input_tokens: Some(8),
                        reasoning_tokens: Some(2),
                        output_tokens: Some(0),
                        total_tokens: Some(14),
                        first_token_latency_ms: Some(140),
                        turn_duration_ms: Some(880),
                        latency_kind:
                            crate::agent::telemetry::ProviderLatencyKind::BufferedResponse,
                        prefix_mutation_reasons: vec![
                            crate::agent::provider::PrefixMutationReason::SessionSummaryChanged,
                        ],
                    }],
                    hook_trace_records: vec![crate::agent::hooks::HookTraceRecord {
                        hook_name: "observe.failed-finalize".to_string(),
                        hook_class: crate::agent::hooks::HookClass::Observe,
                        hook_point: crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
                        hook_order: 1,
                        result_kind: crate::agent::hooks::HookResultKind::Deny,
                        structured_result: crate::agent::hooks::HookStructuredResult::Deny(
                            crate::agent::hooks::HookDenyDecision {
                                reason_code: "hook_blocked_finalize".to_string(),
                                message: "hook blocked finalize".to_string(),
                            },
                        ),
                        blocked: true,
                        elapsed_ms: 5,
                        input_summary: Some("failed".to_string()),
                        persistence_evidence_ref: Some(
                            "trace://turn-failed-evidence/finalize".to_string(),
                        ),
                        summary: "failed finalize hook".to_string(),
                    }],
                    provider_requested_name: Some("test-openai".to_string()),
                    provider_name: Some("test-openai".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5.4".to_string()),
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    session_summary: Some("failed summary".to_string()),
                    fallback_reason: None,
                    error: Some("hook blocked finalize".to_string()),
                    input_tokens: Some(12),
                    cache_hit_input_tokens: Some(4),
                    reasoning_tokens: Some(2),
                    output_tokens: Some(0),
                    total_tokens: Some(14),
                    first_token_latency_ms: Some(140),
                    turn_duration_ms: Some(880),
                    updated_at: 0,
                },
            );
            runtime.record_turn_trace_for_test(
                Some("session-terminal-evidence"),
                crate::agent::session::TurnTraceRecord {
                    turn_id: "turn-cancelled-evidence".to_string(),
                    session_id: Some("session-terminal-evidence".to_string()),
                    event_id: Some("turn-cancelled-evidence:5".to_string()),
                    event_type: Some("turn.cancelled".to_string()),
                    event_version: Some("turn-event-v1".to_string()),
                    sequence: Some(5),
                    emitted_at_ms: Some(5005),
                    title: "cancelled evidence".to_string(),
                    phase: "cancelled".to_string(),
                    trace_steps: vec![crate::agent::telemetry::TurnTraceStep {
                        id: "step-call-tool".to_string(),
                        label: "Call tool".to_string(),
                        state: "completed".to_string(),
                    }],
                    trace_timeline: Vec::new(),
                    tool_activities: vec![crate::agent::telemetry::TurnToolActivity {
                        id: "tool-cancelled-1".to_string(),
                        name: "workspace_list_files".to_string(),
                        canonical_tool_name: Some("List".to_string()),
                        display_name_zh: Some("列表".to_string()),
                        status: "completed".to_string(),
                        description: "tool completed before cancel".to_string(),
                        arguments_text: Some("{\"path\":\".\"}".to_string()),
                        result_text: Some("ok".to_string()),
                        duration_seconds: Some(0.3),
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
                        capability_invocation: None,
                    }],
                    provider_call_records: vec![crate::agent::telemetry::ProviderCallCacheRecord {
                        request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                        provider_source: Some("provider_stream".to_string()),
                        provider_mode: Some("live".to_string()),
                        input_tokens: Some(9),
                        cache_hit_input_tokens: Some(3),
                        cache_hit_source: None,
                        cache_miss_input_tokens: Some(6),
                        reasoning_tokens: Some(1),
                        output_tokens: Some(0),
                        total_tokens: Some(10),
                        first_token_latency_ms: Some(90),
                        turn_duration_ms: Some(510),
                        latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                        prefix_mutation_reasons: vec![
                            crate::agent::provider::PrefixMutationReason::HistoryBoundaryShifted,
                        ],
                    }],
                    hook_trace_records: vec![crate::agent::hooks::HookTraceRecord {
                        hook_name: "observe.cancelled-finalize".to_string(),
                        hook_class: crate::agent::hooks::HookClass::Observe,
                        hook_point: crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
                        hook_order: 1,
                        result_kind: crate::agent::hooks::HookResultKind::Observe,
                        structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                            summary: "cancelled terminal observed".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 3,
                        input_summary: Some("cancelled".to_string()),
                        persistence_evidence_ref: Some(
                            "trace://turn-cancelled-evidence/finalize".to_string(),
                        ),
                        summary: "cancelled finalize hook".to_string(),
                    }],
                    provider_requested_name: Some("test-openai".to_string()),
                    provider_name: Some("test-openai".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5.4".to_string()),
                    provider_source: Some("provider_stream".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    session_summary: Some("cancelled summary".to_string()),
                    fallback_reason: Some("stopped_by_user".to_string()),
                    error: Some("stopped_by_user".to_string()),
                    input_tokens: Some(9),
                    cache_hit_input_tokens: Some(3),
                    reasoning_tokens: Some(1),
                    output_tokens: Some(0),
                    total_tokens: Some(10),
                    first_token_latency_ms: Some(90),
                    turn_duration_ms: Some(510),
                    updated_at: 0,
                },
            );
        }

        let drilldown =
            control_plane.load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery {
                session_id: "session-terminal-evidence".to_string(),
            });

        assert_eq!(drilldown.metrics.request_count, 2);
        assert_eq!(drilldown.runtime_view.session.turn_trace_history.len(), 2);

        let failed_trace = drilldown
            .runtime_view
            .session
            .turn_trace_history
            .iter()
            .find(|trace| trace.turn_id == "turn-failed-evidence")
            .expect("failed trace should be visible");
        assert_eq!(failed_trace.event_type.as_deref(), Some("turn.failed"));
        assert_eq!(failed_trace.provider_call_records.len(), 1);
        assert_eq!(failed_trace.tool_activities.len(), 1);
        assert_eq!(failed_trace.hook_trace_records.len(), 1);
        assert!(failed_trace.hook_trace_records[0].blocked);
        assert_eq!(
            failed_trace.hook_trace_records[0]
                .persistence_evidence_ref
                .as_deref(),
            Some("trace://turn-failed-evidence/finalize")
        );

        let cancelled_trace = drilldown
            .runtime_view
            .session
            .turn_trace_history
            .iter()
            .find(|trace| trace.turn_id == "turn-cancelled-evidence")
            .expect("cancelled trace should be visible");
        assert_eq!(
            cancelled_trace.event_type.as_deref(),
            Some("turn.cancelled")
        );
        assert_eq!(cancelled_trace.provider_call_records.len(), 1);
        assert_eq!(cancelled_trace.tool_activities.len(), 1);
        assert_eq!(cancelled_trace.hook_trace_records.len(), 1);
        assert!(!cancelled_trace.hook_trace_records[0].blocked);
        assert_eq!(
            cancelled_trace.hook_trace_records[0]
                .persistence_evidence_ref
                .as_deref(),
            Some("trace://turn-cancelled-evidence/finalize")
        );
    }

    #[test]
    fn completed_session_can_project_checkpoint_lifecycle_boundary_without_recovery() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("checkpoint lifecycle ready")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请验证 checkpoint boundary".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("trace-boundary-session".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("trace-boundary-session".to_string()),
            })
            .expect("completed session should expose lifecycle boundary checkpoint");

        assert_eq!(checkpoint.checkpoint_kind, "lifecycle_boundary");
        assert_eq!(checkpoint.recovery_mode, "replay_required");
        assert_eq!(checkpoint.phase, "checkpointing");
        assert_eq!(checkpoint.status, "completed");
        assert_eq!(checkpoint.projected_runtime_phase, "connecting");
        assert!(checkpoint.submission_command.is_none());
        assert!(!checkpoint.resumable);
        assert!(!checkpoint.replayable);

        let view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("trace-boundary-session".to_string()),
            ..SessionRuntimeViewQuery::default()
        });
        assert_eq!(
            view.checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.checkpoint_kind.as_str()),
            Some("lifecycle_boundary")
        );
        assert_eq!(
            view.checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.phase.as_str()),
            Some("checkpointing")
        );

        server.finish();
    }

    #[test]
    fn retrieved_context_queries_flow_through_control_plane() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("retrieved")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请记住这个项目优先推进 PA-018。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("session-retrieved".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
        let (control_plane, server, _rt_guard) =
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
                    workspace_mode: None,
                    session_id: Some("session-aware".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
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
                    workspace_mode: None,
                    session_id: Some("stream-session".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
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
        assert_eq!(checkpoint.checkpoint_kind, "runtime_control");
        assert_eq!(checkpoint.recovery_mode, "replay_required");
        assert!(!checkpoint.resumable);
        assert!(!checkpoint.replayable);
        assert!(stop.accepted);
        assert_eq!(stop.state, "running");
    }

    #[test]
    fn graph_run_can_start_and_wait_for_next_user_turn() {
        let (control_plane, server, _rt_guard) =
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
                    workspace_mode: None,
                    session_id: Some("graph-session".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph run should start");

        assert_eq!(response.run.id, "run-alpha");
        assert_eq!(response.run.steps.len(), 1);
        assert_eq!(response.run.phase, GraphRunPhase::Ready);
        assert_eq!(response.event.kind, GraphRunEventKind::Updated);
        assert_eq!(response.decision.kind, GraphDecisionKind::Continue);
        assert!(response
            .turn_result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_point
                == crate::agent::hooks::TurnHookPoint::PlannerGraphDecision));
        assert_eq!(
            response.handoff.turn_id.as_deref(),
            Some("run-alpha-turn-1")
        );
        let snapshot = control_plane.load_session_snapshot(SessionSnapshotQuery {
            session_id: Some("graph-session".to_string()),
        });
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("graph turn trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.graph_decision.observe"));
        server.finish();
    }

    #[test]
    fn graph_run_planner_graph_decision_hooks_can_rewrite_decision_summary() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("first response")]);
        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.set_hook_executor_for_test(Box::new(TransformingPlannerHookExecutor));
            runtime
                .register_hook_descriptor(transform_hook_descriptor(
                    "planner.graph_decision.rewrite",
                    10,
                    crate::agent::hooks::TurnHookPoint::PlannerGraphDecision,
                ))
                .expect("register graph decision transform hook");
        }

        let response = control_plane
            .start_graph_run(StartGraphRunCommand {
                run_id: Some("run-graph-hook".to_string()),
                goal: "audit provider config and continue".to_string(),
                input: TurnInput {
                    message: "run the first streamed turn".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: Some("graph-decision-hook".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph run should start");

        assert_eq!(response.decision.summary, "planner summary patched by hook");
        assert!(response
            .turn_result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.graph_decision.rewrite"));

        let snapshot = control_plane.load_session_snapshot(SessionSnapshotQuery {
            session_id: Some("graph-decision-hook".to_string()),
        });
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("graph decision trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.graph_decision.rewrite"));
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.graph_decision.observe"));
        server.finish();
    }

    #[test]
    fn graph_run_can_continue_across_multiple_turns() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
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
                    workspace_mode: None,
                    session_id: Some("graph-continue".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
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
                    workspace_mode: None,
                    session_id: None,
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
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
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
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
                    workspace_mode: None,
                    session_id: Some("graph-pause".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
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
        assert_eq!(stopped.event.hook_point.as_deref(), Some("run_paused"));
        assert_eq!(
            stopped.event.canonical_event_type.as_deref(),
            Some("graph_run.paused")
        );
        assert_eq!(stopped.event.canonical_phase.as_deref(), Some("paused"));
        assert_eq!(
            stopped
                .control_boundary_evidence
                .as_ref()
                .map(|item| item.hook_point.as_str()),
            Some("stop_requested")
        );
        assert_eq!(
            stopped
                .control_boundary_evidence
                .as_ref()
                .map(|item| item.canonical_event_type.as_str()),
            Some("graph_run.stop_requested")
        );
        assert_eq!(
            stopped
                .control_boundary_evidence
                .as_ref()
                .map(|item| item.hook_envelope.command.as_submission_command()),
            Some("stop_graph_run")
        );

        let checkpoint = control_plane
            .load_graph_run_checkpoint(GraphRunCheckpointQuery {
                run_id: Some("run-pause".to_string()),
            })
            .expect("checkpoint should exist");
        assert_eq!(checkpoint.phase, GraphRunPhase::Paused);
        assert!(checkpoint.resumable);
        assert_eq!(checkpoint.control_boundary_evidence.len(), 1);
        assert_eq!(
            checkpoint.control_boundary_evidence[0].hook_point,
            "stop_requested"
        );
        let recovery_checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("graph-pause".to_string()),
            })
            .expect("recovery checkpoint should be projected");
        assert_eq!(recovery_checkpoint.checkpoint_kind, "recovery");
        assert_eq!(recovery_checkpoint.recovery_mode, "persisted_effect");
        assert!(recovery_checkpoint.resumable);
        assert!(recovery_checkpoint.replayable);
        assert_eq!(recovery_checkpoint.status, "ready");
        assert_eq!(recovery_checkpoint.phase, "paused");
        assert_eq!(
            recovery_checkpoint.session_id.as_deref(),
            Some("graph-pause")
        );
        let paused_runtime_view =
            control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
                session_id: Some("graph-pause".to_string()),
                run_id: Some("run-pause".to_string()),
                ..SessionRuntimeViewQuery::default()
            });
        assert_eq!(
            paused_runtime_view
                .control_boundary_evidence
                .as_ref()
                .map(|items| items.len()),
            Some(1)
        );
        assert_eq!(
            paused_runtime_view
                .control_boundary_evidence
                .as_ref()
                .and_then(|items| items.first())
                .map(|item| item.hook_point.as_str()),
            Some("stop_requested")
        );

        let resumed = control_plane
            .resume_graph_run(ResumeGraphRunCommand {
                run_id: "run-pause".to_string(),
                input: TurnInput {
                    message: "continue with the second turn".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: None,
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph run should resume");
        assert_eq!(resumed.run.phase, GraphRunPhase::WaitingUser);
        assert_eq!(resumed.run.resume_count, 1);
        assert_eq!(resumed.run.steps.len(), 2);
        assert_eq!(resumed.run.control_boundary_evidence.len(), 2);
        assert_eq!(
            resumed.run.control_boundary_evidence[1].hook_point,
            "run_resume"
        );
        let resumed_runtime_view =
            control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
                session_id: Some("graph-pause".to_string()),
                run_id: Some("run-pause".to_string()),
                ..SessionRuntimeViewQuery::default()
            });
        assert_eq!(
            resumed_runtime_view
                .control_boundary_evidence
                .as_ref()
                .map(|items| items.len()),
            Some(2)
        );
        assert_eq!(
            resumed_runtime_view
                .control_boundary_evidence
                .as_ref()
                .and_then(|items| items.last())
                .map(|item| item.hook_point.as_str()),
            Some("run_resume")
        );
        server.finish();
    }

    #[test]
    fn non_resumable_recovery_checkpoint_still_exposes_replayable_contract() {
        let checkpoint =
            HostControlPlane::execution_checkpoint_from_graph_run_checkpoint(GraphRunCheckpoint {
                contract_version: "graph/v1".to_string(),
                run_id: "run-replay-only".to_string(),
                goal: "replay only".to_string(),
                session_id: Some("replay-only-session".to_string()),
                phase: GraphRunPhase::Running,
                active_turn_id: Some("turn-running".to_string()),
                last_completed_turn_id: Some("turn-prev".to_string()),
                stop_reason: None,
                steps: Vec::new(),
                last_decision: None,
                last_handoff: None,
                resume_count: 0,
                control_boundary_evidence: Vec::new(),
                resumable: false,
                created_at_ms: 1000,
                updated_at_ms: 1200,
            });

        assert_eq!(checkpoint.checkpoint_kind, "recovery");
        assert_eq!(checkpoint.recovery_mode, "replay_required");
        assert!(!checkpoint.resumable);
        assert!(checkpoint.replayable);
    }

    #[test]
    fn graph_run_stream_can_start_continue_and_resume() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
            json_completion("ignored stream response one"),
            json_completion("stream response one"),
            json_completion("ignored stream response two"),
            json_completion("stream response two"),
            json_completion("ignored stream response three"),
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
                    workspace_mode: None,
                    session_id: Some("graph-stream".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph stream run should prepare");
        assert_eq!(started.run.phase, GraphRunPhase::Running);
        assert_eq!(started.turn_id, "run-stream-turn-1");
        assert!(started.control_boundary_evidence.is_none());
        let running_checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("graph-stream".to_string()),
            })
            .expect("running graph stream should expose runtime checkpoint");
        assert_eq!(running_checkpoint.checkpoint_kind, "runtime_control");
        assert_eq!(running_checkpoint.recovery_mode, "replay_required");
        assert!(!running_checkpoint.resumable);
        assert!(!running_checkpoint.replayable);

        let first = control_plane
            .execute_graph_run_stream(&NoopSink, prepared_start)
            .expect("graph stream run should execute");
        assert_eq!(first.run.phase, GraphRunPhase::WaitingUser);
        assert_eq!(first.run.steps.len(), 1);
        assert_eq!(first.handoff.turn_id.as_deref(), Some("run-stream-turn-1"));
        let waiting_checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("graph-stream".to_string()),
            })
            .expect("waiting graph run should expose recovery checkpoint");
        assert_eq!(waiting_checkpoint.checkpoint_kind, "recovery");
        assert_eq!(waiting_checkpoint.recovery_mode, "persisted_effect");
        assert!(waiting_checkpoint.resumable);
        assert!(waiting_checkpoint.replayable);

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
                    workspace_mode: None,
                    session_id: None,
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph stream run should continue");
        assert_eq!(continued.run.phase, GraphRunPhase::Running);
        assert_eq!(continued.turn_id, "run-stream-turn-2");
        assert!(continued.control_boundary_evidence.is_none());

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
                    workspace_mode: None,
                    session_id: None,
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph stream run should resume");
        assert_eq!(resumed.run.phase, GraphRunPhase::Running);
        assert_eq!(resumed.turn_id, "run-stream-turn-3");
        assert_eq!(
            resumed
                .control_boundary_evidence
                .as_ref()
                .map(|item| item.hook_point.as_str()),
            Some("run_resume")
        );
        assert_eq!(
            resumed
                .control_boundary_evidence
                .as_ref()
                .map(|item| item.hook_envelope.command.as_submission_command()),
            Some("resume_graph_run_stream")
        );
        assert_eq!(
            resumed
                .run_control_audit_summary
                .action_evidence_summary
                .command_kind
                .as_deref(),
            Some("resume_graph_run_stream")
        );
        assert_eq!(
            resumed
                .run_control_audit_summary
                .action_evidence_summary
                .start_reason
                .as_deref(),
            None
        );
        assert_eq!(
            resumed
                .run_control_audit_summary
                .current_context_projection
                .submission_plan_command
                .as_deref(),
            Some("resume_graph_run_stream")
        );

        let third = control_plane
            .execute_graph_run_stream(&NoopSink, prepared_resume)
            .expect("graph stream resume should execute");
        assert_eq!(third.run.steps.len(), 3);
        assert_eq!(third.run.resume_count, 1);
        assert_eq!(third.handoff.turn_id.as_deref(), Some("run-stream-turn-3"));
        server.finish();
    }

    #[test]
    fn running_graph_stream_uses_runtime_checkpoint_until_waiting_user_boundary() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
            json_completion("ignored stream response"),
            json_completion("single streamed response"),
        ]);

        let (started, prepared_start) = control_plane
            .prepare_start_graph_run_stream(StartGraphRunStreamCommand {
                turn_id: "run-stream-boundary-turn-1".to_string(),
                run_id: Some("run-stream-boundary".to_string()),
                goal: "stream boundary check".to_string(),
                input: TurnInput {
                    message: "start the streamed boundary run".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: Some("graph-stream-boundary".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph stream boundary run should prepare");
        assert_eq!(started.run.phase, GraphRunPhase::Running);

        let running_checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("graph-stream-boundary".to_string()),
            })
            .expect("running graph stream should expose runtime checkpoint");
        assert_eq!(running_checkpoint.checkpoint_kind, "runtime_control");
        assert_eq!(running_checkpoint.recovery_mode, "replay_required");
        assert!(!running_checkpoint.resumable);
        assert!(!running_checkpoint.replayable);

        let first = control_plane
            .execute_graph_run_stream(&NoopSink, prepared_start)
            .expect("graph stream boundary run should execute");
        assert_eq!(first.run.phase, GraphRunPhase::WaitingUser);

        let waiting_checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("graph-stream-boundary".to_string()),
            })
            .expect("waiting graph stream should expose recovery checkpoint");
        assert_eq!(waiting_checkpoint.checkpoint_kind, "recovery");
        assert_eq!(waiting_checkpoint.recovery_mode, "persisted_effect");
        assert!(waiting_checkpoint.resumable);
        assert!(waiting_checkpoint.replayable);

        server.finish();
    }

    #[test]
    fn ordinary_start_graph_run_stream_does_not_enter_run_control_summary() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![]);

        let (started, _prepared) = control_plane
            .prepare_start_graph_run_stream(StartGraphRunStreamCommand {
                turn_id: "ordinary-start-turn-1".to_string(),
                run_id: Some("ordinary-start-run".to_string()),
                goal: "ordinary start".to_string(),
                input: TurnInput {
                    message: "start a normal turn".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: Some("ordinary-start-session".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("ordinary graph stream should prepare");

        assert_eq!(
            started
                .run_control_audit_summary
                .action_evidence_summary
                .status,
            "missing"
        );
        assert_eq!(
            started
                .run_control_audit_summary
                .action_evidence_summary
                .command_kind,
            None
        );
        assert_eq!(
            started
                .run_control_audit_summary
                .action_evidence_summary
                .start_reason,
            None
        );
        assert_eq!(
            started
                .run_control_audit_summary
                .current_context_projection
                .submission_plan_command
                .as_deref(),
            Some("start_graph_run_stream")
        );

        server.finish();
    }

    #[test]
    fn session_checkpoint_query_switches_from_runtime_control_to_recovery_after_turn_boundary() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![]);

        let run = {
            let runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.start_graph_run(
                "run-boundary-internal",
                "boundary contract",
                Some("graph-boundary-internal"),
            )
        };

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane.graph_runner.start_run(&mut graph_runs, run);
            control_plane
                .graph_runner
                .begin_turn(
                    &mut graph_runs,
                    "run-boundary-internal",
                    "turn-boundary-internal-1",
                    Some("graph-boundary-internal"),
                )
                .expect("graph run should enter running");
        }

        control_plane.execution_control.register_turn(
            "turn-boundary-internal-1",
            Some("graph-boundary-internal"),
            Some("run-boundary-internal"),
        );

        let running_checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("graph-boundary-internal".to_string()),
            })
            .expect("running session should expose runtime checkpoint");
        assert_eq!(running_checkpoint.checkpoint_kind, "runtime_control");
        assert_eq!(running_checkpoint.recovery_mode, "replay_required");
        assert!(!running_checkpoint.resumable);
        assert!(!running_checkpoint.replayable);

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane
                .graph_runner
                .apply_turn_result(
                    &mut graph_runs,
                    "run-boundary-internal",
                    GraphTurnHandoff {
                        contract_version: "graph/v1".to_string(),
                        turn_id: Some("turn-boundary-internal-1".to_string()),
                        session_id: Some("graph-boundary-internal".to_string()),
                        turn_phase: "ready".to_string(),
                        checkpoint_status: Some("completed".to_string()),
                        checkpoint_phase: Some("ready".to_string()),
                        user_message: "boundary question".to_string(),
                        assistant_message: "boundary answer".to_string(),
                        session_summary: "boundary summary".to_string(),
                        conversation_id: "graph-boundary-internal".to_string(),
                        session_turn_count: 1,
                        run_id: Some("run-boundary-internal".to_string()),
                        run_phase: Some("waiting_user".to_string()),
                        active_task_focus: None,
                        acceptance_focus: None,
                        closeout_focus: None,
                        last_referenced_file: None,
                        recent_attachment_asset_count: 0,
                        long_term_memory_status: "empty".to_string(),
                        long_term_memory_entry_count: 0,
                        trace_step_count: 0,
                        tool_activity_count: 0,
                        provider_name: "OpenAI".to_string(),
                        provider_model: "gpt-5".to_string(),
                    },
                    GraphDecision {
                        kind: GraphDecisionKind::WaitUser,
                        reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                        summary: "wait for next turn".to_string(),
                        target_phase: GraphRunPhase::WaitingUser,
                    },
                )
                .expect("graph run should reach waiting-user boundary");
        }

        control_plane
            .execution_control
            .update("turn-boundary-internal-1", |checkpoint| {
                checkpoint.status = "completed".to_string();
                checkpoint.phase = "ready".to_string();
            });

        let waiting_checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("graph-boundary-internal".to_string()),
            })
            .expect("waiting session should expose recovery checkpoint");
        assert_eq!(waiting_checkpoint.checkpoint_kind, "recovery");
        assert_eq!(waiting_checkpoint.recovery_mode, "persisted_effect");
        assert!(waiting_checkpoint.resumable);
        assert!(waiting_checkpoint.replayable);

        server.finish();
    }

    #[test]
    fn submission_plan_falls_back_to_graph_run_when_checkpoint_is_absent() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![]);

        let run = {
            let runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.start_graph_run("run-plan-ready", "plan fallback", Some("plan-session"))
        };

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane.graph_runner.start_run(&mut graph_runs, run);
        }

        let plan = control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
            session_id: None,
            node_id: None,
            run_id: Some("run-plan-ready".to_string()),
        });

        assert_eq!(plan.command, "continue_graph_run_stream");
        assert_eq!(plan.run_id.as_deref(), Some("run-plan-ready"));
        assert_eq!(plan.source, "graph_run");
        assert_eq!(plan.hook_point, "submission_plan_resolved");
        assert_eq!(
            plan.canonical_event_type,
            "graph_run.submission_plan_resolved"
        );
        assert_eq!(plan.canonical_phase, "ready");
        assert_eq!(
            plan.hook_envelope
                .as_ref()
                .map(|envelope| envelope.phase.as_str()),
            Some("ready")
        );

        server.finish();
    }

    #[test]
    fn submission_plan_prefers_checkpoint_projection_when_available() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![]);

        let run = {
            let runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.start_graph_run("run-plan-recovery", "plan recovery", Some("plan-recovery"))
        };

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane.graph_runner.start_run(&mut graph_runs, run);
            control_plane
                .graph_runner
                .begin_turn(
                    &mut graph_runs,
                    "run-plan-recovery",
                    "turn-plan-recovery-1",
                    Some("plan-recovery"),
                )
                .expect("graph run should enter running");
        }

        control_plane.execution_control.register_turn(
            "turn-plan-recovery-1",
            Some("plan-recovery"),
            Some("run-plan-recovery"),
        );

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane
                .graph_runner
                .apply_turn_result(
                    &mut graph_runs,
                    "run-plan-recovery",
                    GraphTurnHandoff {
                        contract_version: "graph/v1".to_string(),
                        turn_id: Some("turn-plan-recovery-1".to_string()),
                        session_id: Some("plan-recovery".to_string()),
                        turn_phase: "ready".to_string(),
                        checkpoint_status: Some("completed".to_string()),
                        checkpoint_phase: Some("ready".to_string()),
                        user_message: "resume me".to_string(),
                        assistant_message: "checkpoint ready".to_string(),
                        session_summary: "plan recovery summary".to_string(),
                        conversation_id: "plan-recovery".to_string(),
                        session_turn_count: 1,
                        run_id: Some("run-plan-recovery".to_string()),
                        run_phase: Some("waiting_user".to_string()),
                        active_task_focus: None,
                        acceptance_focus: None,
                        closeout_focus: None,
                        last_referenced_file: None,
                        recent_attachment_asset_count: 0,
                        long_term_memory_status: "empty".to_string(),
                        long_term_memory_entry_count: 0,
                        trace_step_count: 0,
                        tool_activity_count: 0,
                        provider_name: "OpenAI".to_string(),
                        provider_model: "gpt-5".to_string(),
                    },
                    GraphDecision {
                        kind: GraphDecisionKind::WaitUser,
                        reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                        summary: "wait for next turn".to_string(),
                        target_phase: GraphRunPhase::WaitingUser,
                    },
                )
                .expect("graph run should reach waiting-user boundary");
        }

        control_plane
            .execution_control
            .update("turn-plan-recovery-1", |checkpoint| {
                checkpoint.status = "completed".to_string();
                checkpoint.phase = "ready".to_string();
            });

        let plan = control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
            session_id: Some("plan-recovery".to_string()),
            node_id: None,
            run_id: None,
        });

        assert_eq!(plan.command, "continue_graph_run_stream");
        assert_eq!(plan.run_id.as_deref(), Some("run-plan-recovery"));
        assert_eq!(plan.source, "graph_run_reconciled");
        assert_eq!(plan.canonical_phase, "ready");
        assert_eq!(
            plan.hook_envelope
                .as_ref()
                .and_then(|envelope| envelope.checkpoint_kind.as_deref()),
            Some("recovery")
        );
        assert_eq!(
            plan.hook_envelope
                .as_ref()
                .and_then(|envelope| envelope.recovery_mode.as_deref()),
            Some("persisted_effect")
        );

        server.finish();
    }

    #[test]
    fn submission_plan_starts_fresh_run_when_recovery_contract_requires_replay() {
        let control_plane = HostControlPlane::new();
        control_plane.execution_control.register_turn(
            "turn-replay-plan",
            Some("replay-plan-session"),
            Some("run-stale"),
        );
        control_plane
            .execution_control
            .update("turn-replay-plan", |checkpoint| {
                checkpoint.checkpoint_kind = "recovery".to_string();
                checkpoint.recovery_mode = "replay_required".to_string();
                checkpoint.status = "ready".to_string();
                checkpoint.phase = "waiting_user".to_string();
                checkpoint.run_id = Some("run-stale".to_string());
                checkpoint.resumable = false;
                checkpoint.replayable = true;
                refresh_execution_checkpoint_projection(checkpoint);
            });

        let plan = control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
            session_id: Some("replay-plan-session".to_string()),
            node_id: None,
            run_id: Some("run-stale".to_string()),
        });

        assert_eq!(plan.command, "start_graph_run_stream");
        assert_eq!(plan.run_id, None);
        assert_eq!(plan.source, "checkpoint");
        assert_eq!(plan.canonical_phase, "ready");
        assert_eq!(
            plan.hook_envelope
                .as_ref()
                .map(|envelope| envelope.phase.as_str()),
            Some("ready")
        );
    }

    #[test]
    fn submission_plan_does_not_resume_when_run_is_not_resumable() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![]);

        let run = {
            let runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.start_graph_run(
                "run-not-resumable",
                "plan reconcile",
                Some("plan-reconcile"),
            )
        };

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane.graph_runner.start_run(&mut graph_runs, run);
            control_plane
                .graph_runner
                .begin_turn(
                    &mut graph_runs,
                    "run-not-resumable",
                    "turn-not-resumable-1",
                    Some("plan-reconcile"),
                )
                .expect("graph run should enter running");
        }

        control_plane.execution_control.register_turn(
            "turn-not-resumable-1",
            Some("plan-reconcile"),
            Some("run-not-resumable"),
        );
        control_plane
            .execution_control
            .update("turn-not-resumable-1", |checkpoint| {
                checkpoint.checkpoint_kind = "recovery".to_string();
                checkpoint.recovery_mode = "persisted_effect".to_string();
                checkpoint.status = "ready".to_string();
                checkpoint.phase = "paused".to_string();
                checkpoint.resumable = true;
                checkpoint.replayable = true;
                refresh_execution_checkpoint_projection(checkpoint);
            });

        let plan = control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
            session_id: Some("plan-reconcile".to_string()),
            node_id: None,
            run_id: Some("run-not-resumable".to_string()),
        });

        assert_eq!(plan.command, "start_graph_run_stream");
        assert_eq!(plan.run_id, None);
        assert_eq!(plan.source, "graph_run_reconciled");

        server.finish();
    }

    #[test]
    fn lifecycle_boundary_checkpoint_does_not_override_default_submission_plan() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("lifecycle boundary completed")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "完成一个普通 turn".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("plan-lifecycle-boundary".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("plan-lifecycle-boundary".to_string()),
            })
            .expect("completed session should expose lifecycle boundary checkpoint");
        assert_eq!(checkpoint.checkpoint_kind, "lifecycle_boundary");
        assert!(checkpoint.submission_command.is_none());

        let plan = control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
            session_id: Some("plan-lifecycle-boundary".to_string()),
            node_id: None,
            run_id: None,
        });

        assert_eq!(plan.command, "start_graph_run_stream");
        assert_eq!(plan.run_id, None);
        assert_eq!(plan.source, "default");
        assert_eq!(plan.canonical_phase, "ready");
        assert_eq!(
            plan.hook_envelope
                .as_ref()
                .map(|envelope| envelope.phase.as_str()),
            Some("ready")
        );

        server.finish();
    }

    #[test]
    fn lifecycle_boundary_checkpoint_projects_memory_write_evidence_from_session_snapshot() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("我会记住这条信息。")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请记住这个项目当前优先推进 PA-039。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("checkpoint-memory-evidence".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("checkpoint-memory-evidence".to_string()),
            })
            .expect("completed session should expose lifecycle boundary checkpoint");
        assert_eq!(checkpoint.checkpoint_kind, "lifecycle_boundary");
        assert_eq!(checkpoint.recovery_mode, "persisted_effect");
        assert!(checkpoint.replayable);
        assert!(!checkpoint.persisted_effect_evidence.is_empty());
        assert!(checkpoint.persisted_effect_evidence.iter().all(|evidence| {
            evidence.effect_kind == "memory_write.long_term_memory"
                && evidence.replay_required_if_missing
                && evidence.source_history_node_id.is_some()
        }));

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("checkpoint-memory-evidence".to_string()),
            ..SessionRuntimeViewQuery::default()
        });
        let projected = runtime_view
            .checkpoint
            .expect("runtime view should expose lifecycle boundary checkpoint");
        assert_eq!(projected.recovery_mode, "persisted_effect");
        assert!(projected.replayable);
        assert_eq!(
            projected.persisted_effect_evidence,
            runtime_view
                .session
                .memory_write_evidence
                .iter()
                .filter(|evidence| {
                    runtime_view
                        .session
                        .history_cursor
                        .visible_node_id
                        .as_deref()
                        .is_some_and(|history_node_id| {
                            evidence.source_history_node_id.as_deref() == Some(history_node_id)
                        })
                })
                .cloned()
                .collect::<Vec<_>>()
        );

        server.finish();
    }

    #[test]
    fn lifecycle_boundary_checkpoint_keeps_replay_required_when_latest_node_has_no_memory_write_evidence(
    ) {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
            json_completion("我会记住这条信息。"),
            json_completion("好的，我继续推进。"),
        ]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请记住这个项目当前优先推进 PA-039。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("checkpoint-memory-evidence-stale".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
            let _ = runtime.run_turn(TurnInput {
                message: "继续检查 control_plane 的恢复判定。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("checkpoint-memory-evidence-stale".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("checkpoint-memory-evidence-stale".to_string()),
            })
            .expect("completed session should expose lifecycle boundary checkpoint");
        assert_eq!(checkpoint.checkpoint_kind, "lifecycle_boundary");
        assert_eq!(checkpoint.recovery_mode, "replay_required");
        assert!(!checkpoint.replayable);
        assert!(checkpoint.persisted_effect_evidence.is_empty());

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("checkpoint-memory-evidence-stale".to_string()),
            ..SessionRuntimeViewQuery::default()
        });
        assert!(!runtime_view.session.memory_write_evidence.is_empty());
        let projected = runtime_view
            .checkpoint
            .expect("runtime view should expose lifecycle boundary checkpoint");
        assert_eq!(projected.recovery_mode, "replay_required");
        assert!(projected.persisted_effect_evidence.is_empty());

        server.finish();
    }

    #[test]
    fn file_backed_reload_restores_lifecycle_boundary_projection() {
        let path = temp_sessions_path();
        let backend = Box::new(crate::agent::session::FileSessionBackend::new(path.clone()));
        let mut sessions = SessionStore::with_backend(backend);

        sessions.record_turn_trace(
            Some("reload-lifecycle-boundary"),
            TurnTraceRecord {
                turn_id: "turn-reload-lifecycle-boundary".to_string(),
                session_id: Some("reload-lifecycle-boundary".to_string()),
                event_id: Some("turn-reload-lifecycle-boundary:9".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(9),
                emitted_at_ms: Some(9009),
                title: "reload lifecycle boundary".to_string(),
                phase: "completed".to_string(),
                trace_steps: vec![crate::agent::telemetry::TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "completed".to_string(),
                }],
                trace_timeline: vec![
                    crate::agent::session::TraceTimelineEntry {
                        id: "return-1".to_string(),
                        kind: "return".to_string(),
                        label: "RETURN RESULT".to_string(),
                        state: "completed".to_string(),
                        sequence: 1,
                        text: Some("assistant result synthesized".to_string()),
                        output_tokens: Some(13),
                        total_tokens: Some(21),
                        turn_duration_ms: Some(400),
                        ..crate::agent::session::TraceTimelineEntry::default()
                    },
                    crate::agent::session::TraceTimelineEntry {
                        id: "checkpoint-2".to_string(),
                        kind: "checkpoint_persist".to_string(),
                        label: "PERSIST CHECKPOINT".to_string(),
                        state: "completed".to_string(),
                        sequence: 2,
                        output_tokens: Some(13),
                        total_tokens: Some(21),
                        turn_duration_ms: Some(401),
                        ..crate::agent::session::TraceTimelineEntry::default()
                    },
                ],
                session_summary: Some("checkpoint boundary available after reload".to_string()),
                ..TurnTraceRecord::default()
            },
        );

        let reloaded_sessions = SessionStore::with_backend(Box::new(
            crate::agent::session::FileSessionBackend::new(path.clone()),
        ));
        let runtime = AgentRuntime::with_dependencies(
            reloaded_sessions,
            Box::new(StaticResolver {
                selection: test_provider_selection("http://localhost".to_string()),
            }),
            Box::new(ToolRouter::new()),
            Box::new(LocalTurnPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        let control_plane = HostControlPlane::with_runtime(runtime);

        let checkpoint = control_plane
            .load_execution_checkpoint(ExecutionCheckpointQuery {
                turn_id: None,
                session_id: Some("reload-lifecycle-boundary".to_string()),
            })
            .expect("reloaded trace should project lifecycle boundary checkpoint");
        assert_eq!(checkpoint.checkpoint_kind, "lifecycle_boundary");
        assert_eq!(checkpoint.recovery_mode, "replay_required");
        assert_eq!(checkpoint.phase, "checkpointing");
        assert_eq!(checkpoint.projected_runtime_phase, "connecting");
        assert!(!checkpoint.resumable);
        assert!(!checkpoint.replayable);
        assert!(checkpoint.submission_command.is_none());

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("reload-lifecycle-boundary".to_string()),
            ..SessionRuntimeViewQuery::default()
        });
        let projected = runtime_view
            .checkpoint
            .expect("runtime view should expose lifecycle boundary after reload");
        assert_eq!(projected.checkpoint_kind, "lifecycle_boundary");
        assert_eq!(projected.phase, "checkpointing");
        assert_eq!(projected.projected_runtime_phase, "connecting");
        let submission_plan = runtime_view
            .submission_plan
            .expect("runtime view should expose submission plan after reload");
        assert_eq!(submission_plan.command, "start_graph_run_stream");
        assert_eq!(submission_plan.run_id, None);
        assert_eq!(submission_plan.source, "default");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backed_reload_restores_persisted_capability_and_skill_source_ingress() {
        let path = temp_sessions_path();
        let sessions = SessionStore::with_backend(Box::new(
            crate::agent::session::FileSessionBackend::new(path.clone()),
        ));
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

        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::McpSourceSnapshot {
                    source: crate::agent::capability_bridge::CapabilitySourceView {
                        source_id: "mcp-reload".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        display_name: "Reload MCP".to_string(),
                        transport_kind: "stdio".to_string(),
                        server_identity: "mcp://reload".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        declared_capabilities: vec![
                            crate::agent::capability_bridge::CapabilityKind::Tool,
                        ],
                        permission_profile: "host-mediated".to_string(),
                        updated_at_ms: 1,
                        last_ingress_observation: None,
                    },
                    capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                        capability_id: "mcp:tool:reload-search".to_string(),
                        source_id: "mcp-reload".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                        label: "reload.search".to_string(),
                        description: "Reload search tool".to_string(),
                        invocation_mode:
                            crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                        input_schema_summary: "{}".to_string(),
                        safety_class: "host_tool".to_string(),
                        visibility: "default".to_string(),
                        observability_tags: vec!["reload".to_string()],
                        requires_approval: false,
                        host_mediated: true,
                        permission_scope: "workspace.read".to_string(),
                    }],
                },
            })
            .expect("mcp snapshot should persist");

        control_plane
            .apply_skill_source_snapshot(ApplySkillSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::SkillSourceSnapshot {
                    source: crate::agent::capability_bridge::SkillSourceView {
                        source_id: "host-reload-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        display_name: "Reload Skills".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        transport_kind: "host".to_string(),
                        server_identity: "skills://reload".to_string(),
                        updated_at_ms: 2,
                        last_ingress_observation: None,
                    },
                    skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                        skill_id: "skill:reload-search".to_string(),
                        source_id: "host-reload-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        label: "reload-search".to_string(),
                        description: "Reload search skill".to_string(),
                        input_schema_summary: "{}".to_string(),
                        safety_class: "".to_string(),
                        visibility: "default".to_string(),
                        observability_tags: vec!["reload".to_string()],
                        requires_approval: false,
                        host_mediated: false,
                        permission_scope: "".to_string(),
                        composed_capability_refs: vec!["mcp:tool:reload-search".to_string()],
                        composed_capability_kinds: vec![],
                        executable_in_v1: false,
                    }],
                },
            })
            .expect("skill snapshot should persist");

        let reloaded_sessions = SessionStore::with_backend(Box::new(
            crate::agent::session::FileSessionBackend::new(path.clone()),
        ));
        let reloaded_runtime = AgentRuntime::with_dependencies(
            reloaded_sessions,
            Box::new(StaticResolver {
                selection: test_provider_selection("http://localhost".to_string()),
            }),
            Box::new(ToolRouter::new()),
            Box::new(LocalTurnPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        let reloaded_control_plane = HostControlPlane::with_runtime(reloaded_runtime);

        let reloaded_source = reloaded_control_plane
            .inspect_capability_source(CapabilitySourceInspectionQuery {
                source_id: "mcp-reload".to_string(),
            })
            .expect("reloaded mcp source should be visible");
        let ingress = reloaded_source
            .last_ingress_observation
            .expect("mcp ingress should survive reload");
        assert_eq!(ingress.boundary, "control_plane.apply_mcp_source_snapshot");
        assert_eq!(
            ingress.candidate_ids,
            vec!["mcp:tool:reload-search".to_string()]
        );

        let reloaded_skill_source = reloaded_control_plane
            .inspect_skill_source(SkillSourceInspectionQuery {
                source_id: "host-reload-skills".to_string(),
            })
            .expect("reloaded skill source should be visible");
        let skill_ingress = reloaded_skill_source
            .last_ingress_observation
            .expect("skill ingress should survive reload");
        assert_eq!(
            skill_ingress.boundary,
            "control_plane.apply_skill_source_snapshot"
        );
        assert_eq!(
            skill_ingress.candidate_ids,
            vec!["skill:reload-search".to_string()]
        );

        let reloaded_capabilities = reloaded_control_plane.list_capabilities(CapabilityListQuery {
            source_id: Some("mcp-reload".to_string()),
            kind: None,
        });
        assert_eq!(reloaded_capabilities.len(), 1);
        assert_eq!(
            reloaded_capabilities[0].capability_id,
            "mcp:tool:reload-search"
        );

        let reloaded_skills = reloaded_control_plane.list_skills(SkillListQuery {
            source_id: Some("host-reload-skills".to_string()),
        });
        assert_eq!(reloaded_skills.len(), 1);
        assert_eq!(reloaded_skills[0].skill_id, "skill:reload-search");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn submission_plan_switches_with_session_checkpoint_boundary() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![]);

        let run = {
            let runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.start_graph_run(
                "run-plan-boundary",
                "boundary plan check",
                Some("graph-plan-boundary"),
            )
        };

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane.graph_runner.start_run(&mut graph_runs, run);
            control_plane
                .graph_runner
                .begin_turn(
                    &mut graph_runs,
                    "run-plan-boundary",
                    "turn-plan-boundary-1",
                    Some("graph-plan-boundary"),
                )
                .expect("graph run should enter running");
        }

        control_plane.execution_control.register_turn(
            "turn-plan-boundary-1",
            Some("graph-plan-boundary"),
            Some("run-plan-boundary"),
        );

        let running_plan =
            control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
                session_id: Some("graph-plan-boundary".to_string()),
                node_id: None,
                run_id: Some("run-plan-boundary".to_string()),
            });
        assert_eq!(running_plan.command, "continue_graph_run_stream");
        assert_eq!(running_plan.run_id.as_deref(), Some("run-plan-boundary"));
        assert_eq!(running_plan.source, "graph_run");

        {
            let mut graph_runs = control_plane
                .graph_runs
                .lock()
                .expect("graph run lock poisoned");
            control_plane
                .graph_runner
                .apply_turn_result(
                    &mut graph_runs,
                    "run-plan-boundary",
                    GraphTurnHandoff {
                        contract_version: "graph/v1".to_string(),
                        turn_id: Some("turn-plan-boundary-1".to_string()),
                        session_id: Some("graph-plan-boundary".to_string()),
                        turn_phase: "ready".to_string(),
                        checkpoint_status: Some("completed".to_string()),
                        checkpoint_phase: Some("ready".to_string()),
                        user_message: "boundary plan question".to_string(),
                        assistant_message: "boundary plan answer".to_string(),
                        session_summary: "boundary plan summary".to_string(),
                        conversation_id: "graph-plan-boundary".to_string(),
                        session_turn_count: 1,
                        run_id: Some("run-plan-boundary".to_string()),
                        run_phase: Some("waiting_user".to_string()),
                        active_task_focus: None,
                        acceptance_focus: None,
                        closeout_focus: None,
                        last_referenced_file: None,
                        recent_attachment_asset_count: 0,
                        long_term_memory_status: "empty".to_string(),
                        long_term_memory_entry_count: 0,
                        trace_step_count: 0,
                        tool_activity_count: 0,
                        provider_name: "OpenAI".to_string(),
                        provider_model: "gpt-5".to_string(),
                    },
                    GraphDecision {
                        kind: GraphDecisionKind::WaitUser,
                        reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                        summary: "wait for boundary resume".to_string(),
                        target_phase: GraphRunPhase::WaitingUser,
                    },
                )
                .expect("graph run should reach waiting-user boundary");
        }

        control_plane
            .execution_control
            .update("turn-plan-boundary-1", |checkpoint| {
                checkpoint.status = "completed".to_string();
                checkpoint.phase = "ready".to_string();
            });

        let waiting_plan =
            control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
                session_id: Some("graph-plan-boundary".to_string()),
                node_id: None,
                run_id: Some("run-plan-boundary".to_string()),
            });
        assert_eq!(waiting_plan.command, "continue_graph_run_stream");
        assert_eq!(waiting_plan.run_id.as_deref(), Some("run-plan-boundary"));
        assert_eq!(waiting_plan.source, "graph_run_reconciled");

        server.finish();
    }

    #[test]
    fn inspection_can_include_graph_run_views() {
        let (control_plane, server, _rt_guard) =
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
                workspace_mode: None,
                session_id: Some("graph-inspect".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
        let (control_plane, server, _rt_guard) =
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
                workspace_mode: None,
                session_id: Some("graph-infer".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
            json_completion("第一答"),
            json_completion("第二答"),
            json_completion("分叉回答"),
        ]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "第一问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
            let _ = runtime.run_turn(TurnInput {
                message: "第二问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let graph = control_plane.load_history_graph(HistoryGraphQuery {
            session_id: Some("history-control".to_string()),
        });
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.branches.len(), 1);

        // The graph includes a checkpoint root node before the two committed
        // turns, so the first turn is graph.nodes[1] and the main branch head
        // (the last turn) is graph.nodes[2].
        let first_node_id = graph.nodes[1].node_id.clone();
        let latest_node_id = graph.nodes[2].node_id.clone();
        let checkout = control_plane
            .checkout_history_node(CheckoutHistoryNodeCommand {
                session_id: Some("history-control".to_string()),
                node_id: first_node_id.clone(),
                mode: HistoryCheckoutMode::TranscriptAndWorkspace,
                expected_cursor_version: None,
            })
            .expect("checkout should succeed");
        assert!(checkout.transcript_restore_applied);
        assert!(!checkout.workspace_rollback_capable);
        assert!(!checkout.workspace_rollback_applied);
        assert!(checkout.degraded);
        assert_eq!(
            checkout.degradation_reason.as_deref(),
            Some("workspace_rollback_unsupported")
        );
        assert_eq!(checkout.cursor.mode, HistoryCursorMode::Live);

        let historical_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("history-control".to_string()),
            node_id: Some(first_node_id.clone()),
            ..SessionRuntimeViewQuery::default()
        });
        assert_eq!(historical_view.authority_mode, "host_authoritative");
        assert_eq!(
            historical_view.resolved_visible_node_id.as_deref(),
            Some(first_node_id.as_str())
        );
        // checkout_history_node truncates the later node and makes the
        // checked-out node the branch head, so the runtime view at
        // first_node_id is at the (new) branch head.
        assert_eq!(
            historical_view.active_branch_head_node_id.as_deref(),
            Some(first_node_id.as_str())
        );
        assert!(historical_view.is_at_branch_head);
        assert_eq!(historical_view.cursor_version, Some(0));
        assert_eq!(
            historical_view
                .history_cursor
                .as_ref()
                .map(|cursor| cursor.authority_mode.as_str()),
            Some("host_authoritative")
        );
        assert_eq!(
            historical_view
                .history_cursor
                .as_ref()
                .and_then(|cursor| cursor.cursor_version),
            checkout.cursor.cursor_version
        );
        assert_eq!(
            historical_view.session.resolved_node_id.as_deref(),
            Some(first_node_id.as_str())
        );
        assert_eq!(historical_view.session.history.len(), 2);
        assert_eq!(historical_view.session.history[0].content, "第一问");
        let historical_nodes = historical_view
            .history_nodes
            .clone()
            .expect("history nodes should be present in runtime view");
        // Node index 0 is the checkpoint root; the first turn is index 1.
        // Turn ids are generated by the runtime as "sync:{session}:{elapsed}".
        assert!(historical_nodes[1]
            .turn_id
            .as_deref()
            .is_some_and(|value| value.starts_with("sync:history-control:")));
        assert_eq!(historical_nodes[1].workspace_ref.rollback_capable, false);

        let fork = control_plane
            .fork_from_history_node(ForkFromHistoryNodeCommand {
                session_id: Some("history-control".to_string()),
                node_id: first_node_id.clone(),
                expected_cursor_version: None,
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
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "在分叉上继续".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-control".to_string()),
                node_id: fork.cursor.visible_node_id.clone(),
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
        // root + two turns + one fork turn = 4 nodes
        assert_eq!(post_fork_graph.nodes.len(), 4);
        assert_ne!(
            fork_branch.head_node_id.as_deref(),
            Some(latest_node_id.as_str())
        );

        let switched = control_plane
            .switch_history_branch(SwitchHistoryBranchCommand {
                session_id: Some("history-control".to_string()),
                branch_id: "branch-main".to_string(),
                expected_cursor_version: None,
            })
            .expect("switch back to main should succeed");
        // The fork command runs ensure_history_graph, which recomputes the
        // main branch head as the last main-branch node (the latest turn,
        // which was marked cancelled by the earlier checkout).
        assert_eq!(switched.node_id.as_deref(), Some(latest_node_id.as_str()));

        let restored = control_plane
            .restore_branch_head(RestoreBranchHeadCommand {
                session_id: Some("history-control".to_string()),
                branch_id: Some(fork.branch.branch_id.clone()),
                expected_cursor_version: None,
            })
            .expect("restore fork head should succeed");
        assert_eq!(
            restored.branch_id.as_deref(),
            Some(fork.branch.branch_id.as_str())
        );
        assert!(restored.transcript_restore_applied);
        assert!(!restored.workspace_rollback_capable);
        assert!(!restored.workspace_rollback_applied);
        assert!(!restored.degraded);
        assert!(restored.degradation_reason.is_none());
        assert_eq!(restored.cursor.mode, HistoryCursorMode::Live);
        assert_ne!(
            restored.restored_node_id.as_deref(),
            Some(latest_node_id.as_str())
        );

        server.finish();
    }

    #[test]
    fn history_checkout_response_and_runtime_view_share_same_history_state_evidence_projection() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("第一答"), json_completion("第二答")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.set_history_state_hook_executor_for_test(Box::new(
                StaticHistoryStateHookExecutor {
                    start_results: vec![crate::agent::hooks::HookExecutionResult {
                        hook_name: "history.guard.observe".to_string(),
                        hook_class: HookClass::Observe,
                        hook_point: TurnHookPoint::TurnPrepareStart,
                        hook_order: 1,
                        result_kind: HookResultKind::Observe,
                        structured_result: HookStructuredResult::Observe {
                            summary: "history checkout start observed".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 2,
                        input_summary: Some("checkout start".to_string()),
                        persistence_evidence_ref: None,
                        trace_summary: "history checkout start observed".to_string(),
                    }],
                    resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                        hook_name: "history.resolved.observe".to_string(),
                        hook_class: HookClass::Observe,
                        hook_point: TurnHookPoint::TurnPrepareEnd,
                        hook_order: 1,
                        result_kind: HookResultKind::Observe,
                        structured_result: HookStructuredResult::Observe {
                            summary: "history checkout resolved".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 3,
                        input_summary: Some("checkout resolved".to_string()),
                        persistence_evidence_ref: None,
                        trace_summary: "history checkout resolved".to_string(),
                    }],
                },
            ));
            let _ = runtime.run_turn(TurnInput {
                message: "第一问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-evidence-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
            let _ = runtime.run_turn(TurnInput {
                message: "第二问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-evidence-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let graph = control_plane.load_history_graph(HistoryGraphQuery {
            session_id: Some("history-evidence-control".to_string()),
        });
        let first_node_id = graph.nodes[0].node_id.clone();
        let checkout = control_plane
            .checkout_history_node(CheckoutHistoryNodeCommand {
                session_id: Some("history-evidence-control".to_string()),
                node_id: first_node_id.clone(),
                mode: HistoryCheckoutMode::TranscriptOnly,
                expected_cursor_version: None,
            })
            .expect("checkout should succeed");

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("history-evidence-control".to_string()),
            node_id: Some(first_node_id.clone()),
            ..SessionRuntimeViewQuery::default()
        });

        let checkout_evidence = checkout
            .history_state_evidence
            .expect("checkout response should project history state evidence");
        let runtime_evidence = runtime_view
            .history_state_evidence
            .expect("runtime view should project history state evidence");
        assert_eq!(checkout_evidence, runtime_evidence);
        assert_eq!(checkout_evidence.len(), 2);
        assert_eq!(checkout_evidence[0].boundary, "history.checkout.start");
        assert_eq!(checkout_evidence[1].boundary, "history.checkout.resolved");
        assert_eq!(
            checkout_evidence[1].resolved_node_id.as_deref(),
            Some(first_node_id.as_str())
        );
        assert_eq!(
            runtime_view.session.history_state_evidence,
            checkout_evidence
        );
        // Both the checkout response and runtime view should agree on the
        // action-level summary (status, boundary, resolved node).  The
        // current_context.mode may differ between the two code paths because
        // the checkout response derives its cursor from the session's live
        // cursor (which is updated to Live by truncation), while the runtime
        // view snapshot_at reconstructs the cursor from history branch head.
        assert_eq!(
            checkout.history_state_audit_summary.action.status,
            runtime_view.history_state_audit_summary.action.status
        );
        assert_eq!(
            checkout.history_state_audit_summary.action.boundary,
            runtime_view.history_state_audit_summary.action.boundary
        );
        assert_eq!(
            checkout.history_state_audit_summary.action.resolved_node_id,
            runtime_view
                .history_state_audit_summary
                .action
                .resolved_node_id
        );
        // After truncation, the checkout response cursor is always Live
        // because the target node is now the branch head.
        assert_eq!(checkout.cursor.mode, HistoryCursorMode::Live);
        assert_eq!(
            checkout.history_state_audit_summary.action.status,
            "available"
        );
        assert_eq!(
            checkout
                .history_state_audit_summary
                .action
                .boundary
                .as_deref(),
            Some("history.checkout.resolved")
        );
        assert_eq!(
            checkout
                .history_state_audit_summary
                .current_context
                .visible_node_id
                .as_deref(),
            Some(first_node_id.as_str())
        );

        server.finish();
    }

    #[test]
    fn history_restore_fork_switch_responses_project_history_state_evidence() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("第一答"), json_completion("第二答")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.set_history_state_hook_executor_for_test(Box::new(
                StaticHistoryStateHookExecutor {
                    start_results: vec![crate::agent::hooks::HookExecutionResult {
                        hook_name: "history.guard.observe".to_string(),
                        hook_class: HookClass::Observe,
                        hook_point: TurnHookPoint::TurnPrepareStart,
                        hook_order: 1,
                        result_kind: HookResultKind::Observe,
                        structured_result: HookStructuredResult::Observe {
                            summary: "history start observed".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 2,
                        input_summary: Some("history start".to_string()),
                        persistence_evidence_ref: None,
                        trace_summary: "history start observed".to_string(),
                    }],
                    resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                        hook_name: "history.resolved.observe".to_string(),
                        hook_class: HookClass::Observe,
                        hook_point: TurnHookPoint::TurnPrepareEnd,
                        hook_order: 1,
                        result_kind: HookResultKind::Observe,
                        structured_result: HookStructuredResult::Observe {
                            summary: "history resolved observed".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 3,
                        input_summary: Some("history resolved".to_string()),
                        persistence_evidence_ref: None,
                        trace_summary: "history resolved observed".to_string(),
                    }],
                },
            ));
            let _ = runtime.run_turn(TurnInput {
                message: "第一问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-command-evidence".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
            let _ = runtime.run_turn(TurnInput {
                message: "第二问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-command-evidence".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let graph = control_plane.load_history_graph(HistoryGraphQuery {
            session_id: Some("history-command-evidence".to_string()),
        });
        let first_node_id = graph.nodes[0].node_id.clone();
        let fork = control_plane
            .fork_from_history_node(ForkFromHistoryNodeCommand {
                session_id: Some("history-command-evidence".to_string()),
                node_id: first_node_id.clone(),
                expected_cursor_version: None,
            })
            .expect("fork should succeed");
        assert_eq!(
            fork.history_state_evidence
                .as_ref()
                .map(|items| items.len()),
            Some(2)
        );
        assert_eq!(
            fork.history_state_evidence
                .as_ref()
                .and_then(|items| items.last())
                .map(|evidence| evidence.boundary.as_str()),
            Some("history.branch_fork.resolved")
        );

        let switch = control_plane
            .switch_history_branch(SwitchHistoryBranchCommand {
                session_id: Some("history-command-evidence".to_string()),
                branch_id: "branch-main".to_string(),
                expected_cursor_version: None,
            })
            .expect("switch should succeed");
        assert_eq!(
            switch
                .history_state_evidence
                .as_ref()
                .map(|items| items.len()),
            Some(4)
        );
        assert_eq!(
            switch
                .history_state_evidence
                .as_ref()
                .and_then(|items| items.get(2))
                .map(|evidence| evidence.boundary.as_str()),
            Some("history.branch_switch.start")
        );
        assert_eq!(
            switch
                .history_state_evidence
                .as_ref()
                .and_then(|items| items.last())
                .map(|evidence| evidence.boundary.as_str()),
            Some("history.branch_switch.resolved")
        );

        let restore = control_plane
            .restore_branch_head(RestoreBranchHeadCommand {
                session_id: Some("history-command-evidence".to_string()),
                branch_id: Some("branch-main".to_string()),
                expected_cursor_version: None,
            })
            .expect("restore should succeed");
        assert_eq!(
            restore
                .history_state_evidence
                .as_ref()
                .map(|items| items.len()),
            Some(6)
        );
        assert_eq!(
            restore
                .history_state_evidence
                .as_ref()
                .and_then(|items| items.get(4))
                .map(|evidence| evidence.boundary.as_str()),
            Some("history.branch_restore.start")
        );
        assert_eq!(
            restore
                .history_state_evidence
                .as_ref()
                .and_then(|items| items.last())
                .map(|evidence| evidence.boundary.as_str()),
            Some("history.branch_restore.resolved")
        );

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("history-command-evidence".to_string()),
            node_id: Some(first_node_id),
            ..SessionRuntimeViewQuery::default()
        });
        assert_eq!(
            Some(runtime_view.session.history_state_evidence),
            restore.history_state_evidence
        );

        server.finish();
    }

    #[test]
    fn history_checkout_degrade_truth_source_does_not_depend_on_hooks_evidence() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("第一答"), json_completion("第二答")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.set_history_state_hook_executor_for_test(Box::new(
                StaticHistoryStateHookExecutor {
                    start_results: vec![crate::agent::hooks::HookExecutionResult {
                        hook_name: "history.guard.observe".to_string(),
                        hook_class: HookClass::Observe,
                        hook_point: TurnHookPoint::TurnPrepareStart,
                        hook_order: 1,
                        result_kind: HookResultKind::Observe,
                        structured_result: HookStructuredResult::Observe {
                            summary: "history checkout start observed".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 2,
                        input_summary: Some("checkout start".to_string()),
                        persistence_evidence_ref: None,
                        trace_summary: "history checkout start observed".to_string(),
                    }],
                    resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                        hook_name: "history.resolved.observe".to_string(),
                        hook_class: HookClass::Observe,
                        hook_point: TurnHookPoint::TurnPrepareEnd,
                        hook_order: 1,
                        result_kind: HookResultKind::Observe,
                        structured_result: HookStructuredResult::Observe {
                            summary: "history checkout resolved".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 3,
                        input_summary: Some("checkout resolved".to_string()),
                        persistence_evidence_ref: None,
                        trace_summary: "history checkout resolved".to_string(),
                    }],
                },
            ));
            let _ = runtime.run_turn(TurnInput {
                message: "第一问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-degrade-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
            let _ = runtime.run_turn(TurnInput {
                message: "第二问".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("history-degrade-control".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
        }

        let graph = control_plane.load_history_graph(HistoryGraphQuery {
            session_id: Some("history-degrade-control".to_string()),
        });
        let first_node_id = graph.nodes[0].node_id.clone();
        let checkout = control_plane
            .checkout_history_node(CheckoutHistoryNodeCommand {
                session_id: Some("history-degrade-control".to_string()),
                node_id: first_node_id.clone(),
                mode: HistoryCheckoutMode::TranscriptAndWorkspace,
                expected_cursor_version: None,
            })
            .expect("degraded checkout should succeed");

        assert!(checkout.degraded);
        assert!(!checkout.workspace_rollback_capable);
        assert!(!checkout.workspace_rollback_applied);
        assert_eq!(
            checkout.degradation_reason.as_deref(),
            Some("workspace_rollback_unsupported")
        );
        assert_eq!(checkout.cursor.mode, HistoryCursorMode::Live);
        let evidence = checkout
            .history_state_evidence
            .as_ref()
            .expect("degraded checkout should still project evidence");
        assert_eq!(evidence.len(), 2);
        assert!(evidence.last().expect("resolved evidence").degraded);

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("history-degrade-control".to_string()),
            node_id: Some(first_node_id),
            ..SessionRuntimeViewQuery::default()
        });
        assert_eq!(
            runtime_view.session.history_cursor.checkout_status,
            crate::agent::session::HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
        assert_eq!(runtime_view.session.history_state_evidence.len(), 2);
        assert!(runtime_view
            .history_state_evidence
            .as_ref()
            .and_then(|items| items.last())
            .is_some_and(|evidence| evidence.degraded));

        server.finish();
    }

    #[test]
    fn load_model_monitor_summary_aggregates_existing_session_traces() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
            json_completion("第一轮总结"),
            json_completion("第二轮总结"),
        ]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "先看第一轮".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("monitor-summary".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            });
            let _ = runtime.run_turn(TurnInput {
                message: "再继续第二轮".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("monitor-summary".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
                event_id: Some("turn-1:4".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(4),
                emitted_at_ms: Some(4004),
                title: "Capability summary".to_string(),
                phase: "completed".to_string(),
                tool_activities: vec![
                    crate::agent::telemetry::TurnToolActivity {
                        id: "activity-1".to_string(),
                        name: "workspace_search".to_string(),
                        canonical_tool_name: Some("Search".to_string()),
                        display_name_zh: Some("搜索".to_string()),
                        status: "completed".to_string(),
                        description: "tool ok".to_string(),
                        arguments_text: None,
                        result_text: None,
                        duration_seconds: Some(0.2),
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
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
                                permission_facts: Some(crate::agent::tools::ToolPermissionFacts {
                                    requires_approval: Some(false),
                                    permission_scope: Some("workspace.read".to_string()),
                                    host_mediated: Some(true),
                                    permission_profile: Some("capability_registry".to_string()),
                                    approval_mode: Some("none".to_string()),
                                    decision_source: Some("capability_registry".to_string()),
                                }),
                                skill_id: Some("skill:search".to_string()),
                                skill_source_id: Some("host-skills".to_string()),
                                composed_capability_refs: Some(vec![
                                    "mcp:tool:workspace_search".to_string()
                                ]),
                                composed_capability_kinds: Some(vec!["tool".to_string()]),
                                failure_layer: Some("ok".to_string()),
                            },
                        ),
                    },
                    crate::agent::telemetry::TurnToolActivity {
                        id: "activity-2".to_string(),
                        name: "workspace_read".to_string(),
                        canonical_tool_name: Some("Read".to_string()),
                        display_name_zh: Some("读取".to_string()),
                        status: "error".to_string(),
                        description: "resource denied".to_string(),
                        arguments_text: None,
                        result_text: None,
                        duration_seconds: Some(0.4),
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
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
                                permission_facts: Some(crate::agent::tools::ToolPermissionFacts {
                                    requires_approval: Some(true),
                                    permission_scope: Some("workspace.read".to_string()),
                                    host_mediated: Some(false),
                                    permission_profile: Some("capability_registry".to_string()),
                                    approval_mode: Some("approval_required".to_string()),
                                    decision_source: Some("capability_registry".to_string()),
                                }),
                                skill_id: None,
                                skill_source_id: None,
                                composed_capability_refs: None,
                                composed_capability_kinds: None,
                                failure_layer: None,
                            },
                        ),
                    },
                    crate::agent::telemetry::TurnToolActivity {
                        id: "activity-3".to_string(),
                        name: "local_tool".to_string(),
                        canonical_tool_name: Some("Run".to_string()),
                        display_name_zh: Some("运行".to_string()),
                        status: "completed".to_string(),
                        description: "local only".to_string(),
                        arguments_text: None,
                        result_text: None,
                        duration_seconds: Some(0.1),
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
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
                hook_trace_records: vec![
                    crate::agent::hooks::HookTraceRecord {
                        hook_name: "audit.observe".to_string(),
                        hook_class: crate::agent::hooks::HookClass::Observe,
                        hook_point: crate::agent::hooks::TurnHookPoint::ModelCallStart,
                        hook_order: 1,
                        result_kind: crate::agent::hooks::HookResultKind::Observe,
                        structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                            summary: "hook observed lifecycle boundary without mutation"
                                .to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 1,
                        input_summary: Some("monitor".to_string()),
                        persistence_evidence_ref: None,
                        summary: "observe hook summary".to_string(),
                    },
                    crate::agent::hooks::HookTraceRecord {
                        hook_name: "guard.input".to_string(),
                        hook_class: crate::agent::hooks::HookClass::Guard,
                        hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildStart,
                        hook_order: 2,
                        result_kind: crate::agent::hooks::HookResultKind::Deny,
                        structured_result: crate::agent::hooks::HookStructuredResult::Deny(
                            crate::agent::hooks::HookDenyDecision {
                                reason_code: "unsafe_input".to_string(),
                                message: "guard denied the input".to_string(),
                            },
                        ),
                        blocked: true,
                        elapsed_ms: 3,
                        input_summary: Some("monitor-guard".to_string()),
                        persistence_evidence_ref: None,
                        summary: "guard hook blocked the turn".to_string(),
                    },
                ],
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

        assert_eq!(summary.overview.hook_call_count, 2);
        assert_eq!(summary.overview.blocked_hook_count, 1);
        assert_eq!(summary.overview.avg_hook_duration_ms, Some(2));
        assert_eq!(summary.overview.total_hook_duration_ms, 4);
        assert_eq!(summary.sessions.len(), 1);
        assert_eq!(summary.sessions[0].hook_call_count, 2);
        assert_eq!(summary.sessions[0].blocked_hook_count, 1);
        assert_eq!(summary.sessions[0].avg_hook_duration_ms, Some(2));
        assert_eq!(summary.sessions[0].total_hook_duration_ms, 4);
        assert_eq!(summary.hook_classes.len(), 2);
        assert!(summary
            .hook_classes
            .iter()
            .any(|row| row.key == "observe" && row.call_count == 1 && row.blocked_call_count == 0));
        assert!(summary
            .hook_classes
            .iter()
            .any(|row| row.key == "guard" && row.call_count == 1 && row.blocked_call_count == 1));
        assert_eq!(summary.hooks.len(), 2);
        assert!(summary
            .hooks
            .iter()
            .any(|row| row.key == "audit.observe" && row.total_duration_ms == 1));
        assert!(summary
            .hooks
            .iter()
            .any(|row| row.key == "guard.input" && row.blocked_call_count == 1));

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
        assert_eq!(summary.skill_selections.len(), 1);
        assert!(summary
            .skill_selections
            .iter()
            .any(|row| row.key == "skill:search" && row.call_count == 1));
        assert_eq!(summary.skill_sources.len(), 1);
        assert!(summary
            .skill_sources
            .iter()
            .any(|row| row.key == "host-skills" && row.call_count == 1));
        assert_eq!(summary.skill_failure_layers.len(), 1);
        assert!(summary
            .skill_failure_layers
            .iter()
            .any(|row| row.key == "ok" && row.call_count == 1));
    }

    #[test]
    fn monitor_summary_excludes_raw_traces_without_terminal_envelope_from_canonical_metrics() {
        let mut sessions = SessionStore::memory_only();
        sessions.record_turn_trace(
            Some("monitor-raw-terminal"),
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-canonical".to_string(),
                session_id: Some("monitor-raw-terminal".to_string()),
                event_id: Some("turn-canonical:5".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(5),
                emitted_at_ms: Some(5005),
                title: "canonical terminal".to_string(),
                phase: "completed".to_string(),
                provider_name: Some("test-openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_source: Some("test".to_string()),
                provider_mode: Some("chat".to_string()),
                session_summary: Some("canonical".to_string()),
                input_tokens: Some(10),
                output_tokens: Some(7),
                total_tokens: Some(17),
                first_token_latency_ms: Some(120),
                turn_duration_ms: Some(800),
                ..crate::agent::session::TurnTraceRecord::default()
            },
        );
        sessions.record_turn_trace(
            Some("monitor-raw-terminal"),
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-raw".to_string(),
                session_id: Some("monitor-raw-terminal".to_string()),
                title: "raw terminal".to_string(),
                phase: "completed".to_string(),
                provider_name: Some("test-openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_source: Some("test".to_string()),
                provider_mode: Some("chat".to_string()),
                session_summary: Some("raw".to_string()),
                input_tokens: Some(99),
                output_tokens: Some(88),
                total_tokens: Some(187),
                first_token_latency_ms: Some(999),
                turn_duration_ms: Some(1999),
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
            session_id: Some("monitor-raw-terminal".to_string()),
        });
        assert_eq!(summary.overview.request_count, 1);
        assert_eq!(summary.overview.input_tokens, 10);
        assert_eq!(summary.overview.output_tokens, 7);
        assert_eq!(summary.overview.total_tokens, 17);
        assert_eq!(summary.sessions.len(), 1);
        assert_eq!(summary.sessions[0].request_count, 1);
        assert_eq!(summary.sessions[0].total_tokens, 17);

        let drilldown =
            control_plane.load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery {
                session_id: "monitor-raw-terminal".to_string(),
            });
        assert_eq!(drilldown.metrics.request_count, 1);
        assert_eq!(drilldown.runtime_view.session.turn_trace_history.len(), 2);
        let raw_trace = drilldown
            .runtime_view
            .session
            .turn_trace_history
            .iter()
            .find(|trace| trace.turn_id == "turn-raw")
            .expect("raw trace should remain visible in drilldown");
        assert!(raw_trace.event_type.is_none());
    }

    #[test]
    fn monitor_summary_aggregates_mixed_sync_and_stream_terminal_truth_from_persisted_traces() {
        let mut sessions = SessionStore::memory_only();
        sessions.record_turn_trace(
            Some("monitor-sync-session"),
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-sync".to_string(),
                session_id: Some("monitor-sync-session".to_string()),
                event_id: Some("sync:monitor-sync-session:1".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(1),
                emitted_at_ms: Some(1001),
                title: "sync terminal".to_string(),
                phase: "completed".to_string(),
                provider_name: Some("test-openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                session_summary: Some("sync summary".to_string()),
                input_tokens: Some(10),
                output_tokens: Some(7),
                total_tokens: Some(17),
                first_token_latency_ms: Some(120),
                turn_duration_ms: Some(800),
                provider_call_records: vec![crate::agent::telemetry::ProviderCallCacheRecord {
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    input_tokens: Some(10),
                    cache_hit_input_tokens: Some(3),
                    cache_hit_source: None,
                    cache_miss_input_tokens: Some(7),
                    reasoning_tokens: Some(1),
                    output_tokens: Some(7),
                    total_tokens: Some(17),
                    first_token_latency_ms: Some(120),
                    turn_duration_ms: Some(800),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::BufferedResponse,
                    prefix_mutation_reasons: Vec::new(),
                }],
                ..crate::agent::session::TurnTraceRecord::default()
            },
        );
        sessions.record_turn_trace(
            Some("monitor-stream-session"),
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-stream".to_string(),
                session_id: Some("monitor-stream-session".to_string()),
                event_id: Some("turn-stream:7".to_string()),
                event_type: Some("turn.failed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(7),
                emitted_at_ms: Some(7007),
                title: "stream terminal".to_string(),
                phase: "failed".to_string(),
                provider_name: Some("test-openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_source: Some("provider_stream".to_string()),
                provider_mode: Some("live".to_string()),
                session_summary: Some("stream summary".to_string()),
                error: Some("provider timeout".to_string()),
                input_tokens: Some(20),
                output_tokens: Some(9),
                total_tokens: Some(29),
                first_token_latency_ms: Some(220),
                turn_duration_ms: Some(1500),
                provider_call_records: vec![crate::agent::telemetry::ProviderCallCacheRecord {
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    provider_source: Some("provider_stream".to_string()),
                    provider_mode: Some("live".to_string()),
                    input_tokens: Some(20),
                    cache_hit_input_tokens: Some(5),
                    cache_hit_source: None,
                    cache_miss_input_tokens: Some(15),
                    reasoning_tokens: Some(2),
                    output_tokens: Some(9),
                    total_tokens: Some(29),
                    first_token_latency_ms: Some(220),
                    turn_duration_ms: Some(1500),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    prefix_mutation_reasons: Vec::new(),
                }],
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

        let summary = control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery::default());

        assert_eq!(summary.overview.session_count, 2);
        assert_eq!(summary.overview.request_count, 2);
        assert_eq!(summary.overview.model_call_count, 2);
        assert_eq!(summary.overview.failed_request_count, 1);
        assert_eq!(summary.overview.input_tokens, 30);
        assert_eq!(summary.overview.output_tokens, 16);
        assert_eq!(summary.overview.total_tokens, 46);
        assert_eq!(summary.overview.avg_first_token_latency_ms, Some(170));
        assert_eq!(summary.overview.avg_turn_duration_ms, Some(1150));
        assert_eq!(summary.sessions.len(), 2);
        assert!(summary
            .sessions
            .iter()
            .any(|row| row.session_id == "monitor-sync-session" && row.total_tokens == 17));
        assert!(summary
            .sessions
            .iter()
            .any(|row| row.session_id == "monitor-stream-session"
                && row.total_tokens == 29
                && row.failed_request_count == 1));
        assert_eq!(summary.providers.len(), 1);
        assert_eq!(summary.providers[0].request_count, 2);
        assert_eq!(summary.models.len(), 1);
        assert_eq!(summary.models[0].request_count, 2);
    }

    #[test]
    fn load_model_monitor_summary_reads_capability_activity_from_runtime_generated_trace() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
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
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let result = runtime.run_turn(TurnInput {
                message: "列出当前目录文件".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("monitor-runtime-capability".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
    fn monitor_summary_keeps_source_ingress_out_of_canonical_trace_aggregates() {
        let mut sessions = SessionStore::memory_only();
        sessions.record_turn_trace(
            Some("monitor-source-ingress"),
            crate::agent::session::TurnTraceRecord {
                turn_id: "turn-monitor-source-ingress".to_string(),
                session_id: Some("monitor-source-ingress".to_string()),
                event_id: Some("turn-monitor-source-ingress:3".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(3),
                emitted_at_ms: Some(3003),
                title: "plain terminal summary".to_string(),
                phase: "completed".to_string(),
                provider_name: Some("test-openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_source: Some("test".to_string()),
                provider_mode: Some("chat".to_string()),
                session_summary: Some("plain summary".to_string()),
                input_tokens: Some(12),
                output_tokens: Some(8),
                total_tokens: Some(20),
                first_token_latency_ms: Some(150),
                turn_duration_ms: Some(900),
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

        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::McpSourceSnapshot {
                    source: crate::agent::capability_bridge::CapabilitySourceView {
                        source_id: "mcp-monitor-source".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        display_name: "Monitor Source MCP".to_string(),
                        transport_kind: "stdio".to_string(),
                        server_identity: "mcp://monitor-source".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        declared_capabilities: vec![
                            crate::agent::capability_bridge::CapabilityKind::Tool,
                        ],
                        permission_profile: "host-mediated".to_string(),
                        updated_at_ms: 1,
                        last_ingress_observation: None,
                    },
                    capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                        capability_id: "mcp:tool:monitor-source-search".to_string(),
                        source_id: "mcp-monitor-source".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                        label: "monitor.source.search".to_string(),
                        description: "Search for monitor source test".to_string(),
                        invocation_mode:
                            crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                        input_schema_summary: "{}".to_string(),
                        safety_class: "host_tool".to_string(),
                        visibility: "default".to_string(),
                        observability_tags: vec!["monitor".to_string()],
                        requires_approval: false,
                        host_mediated: true,
                        permission_scope: "workspace.read".to_string(),
                    }],
                },
            })
            .expect("mcp source snapshot should apply");

        control_plane
            .apply_skill_source_snapshot(ApplySkillSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::SkillSourceSnapshot {
                    source: crate::agent::capability_bridge::SkillSourceView {
                        source_id: "host-monitor-source-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        display_name: "Monitor Source Skills".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        transport_kind: "host".to_string(),
                        server_identity: "skills://monitor-source".to_string(),
                        updated_at_ms: 2,
                        last_ingress_observation: None,
                    },
                    skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                        skill_id: "skill:monitor-source-search".to_string(),
                        source_id: "host-monitor-source-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        label: "monitor-source-search".to_string(),
                        description: "Skill for monitor source test".to_string(),
                        input_schema_summary: "{}".to_string(),
                        safety_class: "".to_string(),
                        visibility: "default".to_string(),
                        observability_tags: vec!["monitor".to_string()],
                        requires_approval: false,
                        host_mediated: false,
                        permission_scope: "".to_string(),
                        composed_capability_refs: vec![
                            "mcp:tool:monitor-source-search".to_string(),
                        ],
                        composed_capability_kinds: vec![],
                        executable_in_v1: false,
                    }],
                },
            })
            .expect("skill source snapshot should apply");

        let summary = control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery {
            session_id: Some("monitor-source-ingress".to_string()),
        });
        assert_eq!(summary.overview.request_count, 1);
        assert_eq!(summary.overview.total_tokens, 20);
        assert!(summary.capability_sources.is_empty());
        assert!(summary.skill_sources.is_empty());

        let source = control_plane
            .inspect_capability_source(CapabilitySourceInspectionQuery {
                source_id: "mcp-monitor-source".to_string(),
            })
            .expect("source drilldown should still exist");
        assert!(source.last_ingress_observation.is_some());

        let skill_source = control_plane
            .inspect_skill_source(SkillSourceInspectionQuery {
                source_id: "host-monitor-source-skills".to_string(),
            })
            .expect("skill source drilldown should still exist");
        assert!(skill_source.last_ingress_observation.is_some());
    }

    #[test]
    fn monitor_and_drilldown_read_runtime_generated_capability_hook_evidence() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
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

        let result = control_plane.run_turn(RunTurnCommand {
            input: TurnInput {
                message: "列出当前目录文件".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("monitor-capability-hook".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            },
        });
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "capability.resolve.observe"));

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("monitor-capability-hook".to_string()),
            ..SessionRuntimeViewQuery::default()
        });
        let trace = runtime_view
            .session
            .turn_trace_history
            .last()
            .expect("capability hook trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "capability.resolve.observe"));

        let drilldown =
            control_plane.load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery {
                session_id: "monitor-capability-hook".to_string(),
            });
        assert!(drilldown.metrics.hook_call_count >= 1);
        assert!(drilldown
            .runtime_view
            .session
            .turn_trace_history
            .last()
            .expect("drilldown trace should exist")
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "capability.resolve.observe"));

        let summary = control_plane.load_model_monitor_summary(ModelMonitorSummaryQuery {
            session_id: Some("monitor-capability-hook".to_string()),
        });
        assert!(summary
            .hooks
            .iter()
            .any(|row| row.key == "capability.resolve.observe" && row.call_count >= 1));

        server.finish();
    }

    #[test]
    fn monitor_and_drilldown_read_runtime_generated_planner_hook_evidence() {
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("目录已分析。")]);

        let result = control_plane.run_turn(RunTurnCommand {
            input: TurnInput {
                message: "列出当前目录文件".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("monitor-planner-hook".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            },
        });
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.preflight.observe"));
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.tool_selection.observe"));

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("monitor-planner-hook".to_string()),
            ..SessionRuntimeViewQuery::default()
        });
        let trace = runtime_view
            .session
            .turn_trace_history
            .last()
            .expect("planner hook trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.preflight.observe"));
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.tool_selection.observe"));

        let drilldown =
            control_plane.load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery {
                session_id: "monitor-planner-hook".to_string(),
            });
        assert!(drilldown.metrics.hook_call_count >= 2);
        let drilldown_trace = drilldown
            .runtime_view
            .session
            .turn_trace_history
            .last()
            .expect("planner drilldown trace should exist");
        assert!(drilldown_trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.preflight.observe"));
        assert!(drilldown_trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.tool_selection.observe"));

        server.finish();
    }

    #[test]
    fn monitor_and_drilldown_read_runtime_generated_skill_hook_evidence() {
        let (control_plane, server, _rt_guard) = build_test_control_plane(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "先执行 skill。",
                            "tool_calls": [
                                {
                                    "id": "call_skill_echo",
                                    "type": "function",
                                    "function": {
                                        "name": "echo_skill",
                                        "arguments": json!({"text": "original"}).to_string()
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
            json_completion("skill 已执行。"),
        ]);
        control_plane
            .apply_skill_source_snapshot(ApplySkillSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::SkillSourceSnapshot {
                    source: crate::agent::capability_bridge::SkillSourceView {
                        source_id: "builtin-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        display_name: "Builtin Skills".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        transport_kind: "host".to_string(),
                        server_identity: "skills://builtin".to_string(),
                        updated_at_ms: 1,
                        last_ingress_observation: None,
                    },
                    skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                        skill_id: "skill:echo".to_string(),
                        source_id: "builtin-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        label: "echo_skill".to_string(),
                        description: "Echo message".to_string(),
                        input_schema_summary: "{}".to_string(),
                        safety_class: "".to_string(),
                        visibility: "default".to_string(),
                        observability_tags: vec![],
                        requires_approval: false,
                        host_mediated: false,
                        permission_scope: "".to_string(),
                        composed_capability_refs: vec!["builtin:echo_input".to_string()],
                        composed_capability_kinds: vec![
                            crate::agent::capability_bridge::CapabilityKind::Tool,
                        ],
                        executable_in_v1: true,
                    }],
                },
            })
            .expect("skill snapshot should apply");

        let result = control_plane.run_turn(RunTurnCommand {
            input: TurnInput {
                message: "运行 echo skill".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("monitor-skill-hook".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
            },
        });
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "skill.tool_actions.observe"));

        let runtime_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
            session_id: Some("monitor-skill-hook".to_string()),
            ..SessionRuntimeViewQuery::default()
        });
        let trace = runtime_view
            .session
            .turn_trace_history
            .last()
            .expect("skill hook trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "skill.tool_actions.observe"));

        let drilldown =
            control_plane.load_model_monitor_session_drilldown(ModelMonitorSessionDrilldownQuery {
                session_id: "monitor-skill-hook".to_string(),
            });
        assert!(drilldown.metrics.hook_call_count >= 1);
        assert!(drilldown
            .runtime_view
            .session
            .turn_trace_history
            .last()
            .expect("drilldown trace should exist")
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "skill.tool_actions.observe"));

        server.finish();
    }

    #[test]
    fn list_capabilities_exposes_builtin_tools_through_unified_registry() {
        let control_plane = HostControlPlane::new();

        let sources = control_plane.list_capability_sources();
        assert!(sources
            .iter()
            .any(|source| source.source_id == "builtin-tools"));

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
                last_ingress_observation: None,
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
        let ingress = source
            .last_ingress_observation
            .expect("mcp source ingress observation should be recorded");
        assert_eq!(ingress.boundary, "control_plane.apply_mcp_source_snapshot");
        assert_eq!(
            ingress.candidate_ids,
            vec!["mcp:tool:workspace-search".to_string()]
        );
        assert!(ingress.summary.contains("mcp-local"));

        let capabilities = control_plane.list_capabilities(CapabilityListQuery {
            source_id: Some("mcp-local".to_string()),
            kind: None,
        });
        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].capability_id, "mcp:tool:workspace-search");

        let runtime = control_plane
            .runtime
            .write()
            .expect("runtime lock poisoned");
        let runtime_source = runtime
            .inspect_capability_source("mcp-local")
            .expect("runtime source registry should be synchronized");
        let runtime_ingress = runtime_source
            .last_ingress_observation
            .expect("runtime mcp source ingress observation should be synchronized");
        assert_eq!(
            runtime_ingress.boundary,
            "control_plane.apply_mcp_source_snapshot"
        );
        assert_eq!(
            runtime_ingress.candidate_ids,
            vec!["mcp:tool:workspace-search".to_string()]
        );
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
                last_ingress_observation: None,
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
                        last_ingress_observation: None,
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
    fn apply_skill_source_snapshot_updates_read_plane_and_runtime_registry() {
        let control_plane = HostControlPlane::new();

        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::McpSourceSnapshot {
                    source: crate::agent::capability_bridge::CapabilitySourceView {
                        source_id: "mcp-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        display_name: "Skills MCP".to_string(),
                        transport_kind: "stdio".to_string(),
                        server_identity: "mcp://skills".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        declared_capabilities: vec![
                            crate::agent::capability_bridge::CapabilityKind::Tool,
                        ],
                        permission_profile: "host-mediated".to_string(),
                        updated_at_ms: 1,
                        last_ingress_observation: None,
                    },
                    capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                        capability_id: "mcp:tool:workspace-search".to_string(),
                        source_id: "mcp-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                        label: "workspace_search".to_string(),
                        description: "Search workspace files".to_string(),
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
                },
            })
            .expect("capability snapshot should apply");

        control_plane
            .apply_skill_source_snapshot(ApplySkillSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::SkillSourceSnapshot {
                    source: crate::agent::capability_bridge::SkillSourceView {
                        source_id: "host-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        display_name: "Host Skills".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        transport_kind: "host".to_string(),
                        server_identity: "skills://host".to_string(),
                        updated_at_ms: 2,
                        last_ingress_observation: None,
                    },
                    skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                        skill_id: "skill:search".to_string(),
                        source_id: "host-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        label: "search".to_string(),
                        description: "Search workspace".to_string(),
                        input_schema_summary: "{}".to_string(),
                        safety_class: "".to_string(),
                        visibility: "default".to_string(),
                        observability_tags: vec!["host".to_string()],
                        requires_approval: false,
                        host_mediated: false,
                        permission_scope: "".to_string(),
                        composed_capability_refs: vec!["mcp:tool:workspace-search".to_string()],
                        composed_capability_kinds: vec![],
                        executable_in_v1: false,
                    }],
                },
            })
            .expect("skill snapshot should apply");

        let skills = control_plane.list_skills(SkillListQuery {
            source_id: Some("host-skills".to_string()),
        });
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].skill_id, "skill:search");
        assert_eq!(skills[0].composed_capability_kinds.len(), 1);
        assert!(skills[0].executable_in_v1);
        let source = control_plane
            .inspect_skill_source(SkillSourceInspectionQuery {
                source_id: "host-skills".to_string(),
            })
            .expect("skill source should be visible");
        let ingress = source
            .last_ingress_observation
            .expect("skill source ingress observation should be recorded");
        assert_eq!(
            ingress.boundary,
            "control_plane.apply_skill_source_snapshot"
        );
        assert_eq!(ingress.candidate_ids, vec!["skill:search".to_string()]);
        assert!(ingress.summary.contains("host-skills"));

        let runtime = control_plane
            .runtime
            .write()
            .expect("runtime lock poisoned");
        let runtime_source = runtime
            .inspect_skill_source("host-skills")
            .expect("runtime skill source registry should be synchronized");
        let runtime_ingress = runtime_source
            .last_ingress_observation
            .expect("runtime skill source ingress observation should be synchronized");
        assert_eq!(
            runtime_ingress.boundary,
            "control_plane.apply_skill_source_snapshot"
        );
        assert_eq!(
            runtime_ingress.candidate_ids,
            vec!["skill:search".to_string()]
        );
        let runtime_skill = runtime
            .inspect_skill("skill:search")
            .expect("runtime skill registry should be synchronized");
        assert_eq!(runtime_skill.source_id, "host-skills");
    }

    #[test]
    fn skill_source_ingress_hooks_can_block_snapshot_apply_without_persisting_source() {
        let control_plane = HostControlPlane::new();
        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            runtime.set_hook_executor_for_test(Box::new(BlockingSkillSourceIngressHookExecutor));
            runtime
                .register_hook_descriptor(guard_hook_descriptor(
                    "skill.source_ingress.guard",
                    10,
                    TurnHookPoint::SkillSourceIngress,
                ))
                .expect("register skill source ingress guard hook");
        }

        let error = control_plane
            .apply_skill_source_snapshot(ApplySkillSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::SkillSourceSnapshot {
                    source: crate::agent::capability_bridge::SkillSourceView {
                        source_id: "blocked-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                        display_name: "Blocked Skills".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        transport_kind: "host".to_string(),
                        server_identity: "skills://blocked".to_string(),
                        updated_at_ms: 1,
                        last_ingress_observation: None,
                    },
                    skills: vec![],
                },
            })
            .expect_err("skill source ingress hook should block apply");

        assert!(error.contains("skill source ingress blocked by hook"));
        assert!(control_plane
            .inspect_skill_source(SkillSourceInspectionQuery {
                source_id: "blocked-skills".to_string(),
            })
            .is_none());
    }

    #[test]
    fn apply_skill_source_snapshot_replaces_stale_skills_for_same_source() {
        let control_plane = HostControlPlane::new();

        control_plane
            .apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand {
                snapshot: crate::agent::capability_bridge::McpSourceSnapshot {
                    source: crate::agent::capability_bridge::CapabilitySourceView {
                        source_id: "mcp-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        display_name: "Skills MCP".to_string(),
                        transport_kind: "stdio".to_string(),
                        server_identity: "mcp://skills".to_string(),
                        availability:
                            crate::agent::capability_bridge::CapabilityAvailability::Available,
                        declared_capabilities: vec![
                            crate::agent::capability_bridge::CapabilityKind::Tool,
                        ],
                        permission_profile: "host-mediated".to_string(),
                        updated_at_ms: 1,
                        last_ingress_observation: None,
                    },
                    capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                        capability_id: "mcp:tool:workspace-search".to_string(),
                        source_id: "mcp-skills".to_string(),
                        source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                        kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                        label: "workspace_search".to_string(),
                        description: "Search workspace files".to_string(),
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
                },
            })
            .expect("capability snapshot should apply");

        let old_snapshot = crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Host Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://host".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:a".to_string(),
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "a".to_string(),
                description: "A".to_string(),
                input_schema_summary: "{}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec![],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["mcp:tool:workspace-search".to_string()],
                composed_capability_kinds: vec![],
                executable_in_v1: false,
            }],
        };
        let new_snapshot = crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                updated_at_ms: 2,
                ..old_snapshot.source.clone()
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:b".to_string(),
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "b".to_string(),
                description: "B".to_string(),
                input_schema_summary: "{}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec![],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["mcp:tool:workspace-search".to_string()],
                composed_capability_kinds: vec![],
                executable_in_v1: false,
            }],
        };

        control_plane
            .apply_skill_source_snapshot(ApplySkillSourceSnapshotCommand {
                snapshot: old_snapshot,
            })
            .expect("old skill snapshot should apply");
        control_plane
            .apply_skill_source_snapshot(ApplySkillSourceSnapshotCommand {
                snapshot: new_snapshot,
            })
            .expect("new skill snapshot should replace old entries");

        let skills = control_plane.list_skills(SkillListQuery {
            source_id: Some("host-skills".to_string()),
        });
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].skill_id, "skill:b");
        assert!(control_plane
            .inspect_skill(SkillInspectionQuery {
                skill_id: "skill:a".to_string(),
            })
            .is_none());
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
        let (control_plane, server, _rt_guard) =
            build_test_control_plane(vec![json_completion("下钻摘要")]);

        {
            let mut runtime = control_plane
                .runtime
                .write()
                .expect("runtime lock poisoned");
            let _ = runtime.run_turn(TurnInput {
                message: "请准备一个下钻样本".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                workspace_mode: None,
                session_id: Some("monitor-drilldown".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
            hook_trace_records: vec![crate::agent::hooks::HookTraceRecord {
                hook_name: "audit.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::ModelCallStart,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Observe,
                structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                    summary: "hook observed lifecycle boundary without mutation".to_string(),
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("openai".to_string()),
                persistence_evidence_ref: None,
                summary: "observe hook summary".to_string(),
            }],
            ..crate::agent::session::TurnTraceRecord::default()
        };
        let anthropic_trace = crate::agent::session::TurnTraceRecord {
            provider_name: Some("anthropic".to_string()),
            provider_model: Some("gpt-5".to_string()),
            hook_trace_records: vec![crate::agent::hooks::HookTraceRecord {
                hook_name: "guard.input".to_string(),
                hook_class: crate::agent::hooks::HookClass::Guard,
                hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildStart,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Allow,
                structured_result: crate::agent::hooks::HookStructuredResult::Allow {
                    summary: "guard allowed runtime to continue".to_string(),
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("anthropic".to_string()),
                persistence_evidence_ref: None,
                summary: "guard hook summary".to_string(),
            }],
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

    // ── Ask / Plan control-plane surface (PA-076 task 4.4) ─────────────────────────────────

    fn host_mediated_ask_descriptor() -> ToolDescriptor {
        let mut declaration = ToolPermissionDeclaration::default();
        declaration.host_mediated = true;
        ToolDescriptor {
            identity: ToolIdentity {
                descriptor_id: "builtin:ask".to_string(),
                model_name: "Ask".to_string(),
                canonical_name: "ask".to_string(),
                primitive_name: "echo_input".to_string(),
                source: ToolDescriptorSource::Builtin,
            },
            aliases: vec!["ask".to_string(), "builtin:ask".to_string()],
            description: String::new(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false,
            }),
            kind: ToolKind::Interactive,
            exposure: ToolExposure::ModelVisible,
            permission_declaration: declaration,
            execution_policy: ToolExecutionPolicy::default(),
            display_metadata: ToolDisplayMetadata::default(),
            handler_provenance: ToolHandlerProvenance {
                handler_kind: "test".to_string(),
                source_id: "builtin-tools".to_string(),
            },
            source_revision: "test-v1".to_string(),
            composed_descriptor_ids: Vec::new(),
        }
    }

    #[test]
    fn ask_control_plane_surface_lists_answers_cancels_and_expires() {
        let registry = Arc::new(
            ToolRegistrySnapshot::from_descriptors("test-ask-snapshot", vec![host_mediated_ask_descriptor()])
                .expect("ask registry must build"),
        );
        let dispatcher = Arc::new(GovernedDispatcher::new(
            registry,
            Arc::new(FakeClock::new(1_000)),
        ));
        let control_plane = HostControlPlaneBuilder::new()
            .ask_dispatcher(Arc::clone(&dispatcher))
            .build();

        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        };
        let dispatch = |call_id: &str, text: &str| {
            let outcome = dispatcher.dispatch_governed(
                ToolDispatchRequest {
                    origin: InvocationOrigin::Model,
                    descriptor_id: "builtin:ask".to_string(),
                    call_id: call_id.to_string(),
                    arguments: json!({ "text": text }),
                },
                &context,
            );
            outcome.control_outcome.expect("ask control outcome").request_id
        };

        let request_id = dispatch("call-1", "first");

        // list_pending_asks returns the Interaction request, filterable by session.
        assert_eq!(control_plane.list_pending_asks(None).as_array().map(Vec::len), Some(1));
        assert_eq!(control_plane.list_pending_asks(Some("session-1")).as_array().map(Vec::len), Some(1));
        assert_eq!(control_plane.list_pending_asks(Some("session-2")).as_array().map(Vec::len), Some(0));
        let listed = control_plane.list_pending_asks(None);
        assert_eq!(listed[0]["requestId"].as_str(), Some(request_id.as_str()));
        assert_eq!(listed[0]["requestKind"].as_str(), Some("interaction"));
        assert_eq!(listed[0]["version"].as_u64(), Some(1));
        assert_eq!(listed[0]["callId"].as_str(), Some("call-1"));

        // Answer with the current version consumes the request.
        let answered = control_plane
            .answer_ask(&request_id, 1, json!("continue"))
            .expect("answer");
        assert_eq!(answered["request"]["state"].as_str(), Some("consumed"));
        assert_eq!(answered["answer"].as_str(), Some("continue"));

        // A stale version is rejected after consumption.
        let stale = control_plane
            .answer_ask(&request_id, 1, json!("again"))
            .expect_err("stale answer must fail closed");
        assert!(stale.contains("stale"), "{stale}");

        // Cancel path.
        let cancel_id = dispatch("call-2", "second");
        let cancelled = control_plane.cancel_ask(&cancel_id, 1).expect("cancel");
        assert_eq!(cancelled["request"]["state"].as_str(), Some("cancelled"));

        // Expire path (explicit now_ms past the pending request expiry).
        let expire_id = dispatch("call-3", "third");
        let pending = dispatcher.pending_request(&expire_id).expect("pending ask");
        let expired = control_plane.expire_asks(Some(pending.expires_at_ms + 1));
        assert_eq!(expired["expired"].as_u64(), Some(1));
    }

    #[test]
    fn plan_control_plane_surface_creates_reads_mutates_and_lists() {
        let control_plane = HostControlPlaneBuilder::new().build();

        let created = control_plane
            .plan_create(
                "session-1",
                json!({
                    "kind": "implement",
                    "summary": "Implement the feature",
                    "steps": [{ "name": "a", "summary": "sa" }, { "name": "b", "summary": "sb" }],
                }),
            )
            .expect("create");
        assert_eq!(created["planId"].as_str(), Some("plan-1"));
        assert_eq!(created["revision"].as_u64(), Some(1));
        assert_eq!(created["lifecycle"].as_str(), Some("draft"));
        assert_eq!(created["steps"].as_array().map(Vec::len), Some(2));

        let merged = control_plane
            .plan_merge("session-1", "plan-1", 1, json!({ "name": "c", "summary": "sc" }))
            .expect("merge");
        assert_eq!(merged["revision"].as_u64(), Some(2));
        assert_eq!(merged["steps"].as_array().map(Vec::len), Some(3));

        let step_id = merged["steps"][0]["stepId"]
            .as_str()
            .expect("step id")
            .to_string();
        let completed = control_plane
            .plan_complete_step("session-1", "plan-1", 2, &step_id)
            .expect("complete step");
        assert_eq!(completed["steps"][0]["status"].as_str(), Some("completed"));
        assert_eq!(completed["lifecycle"].as_str(), Some("executing"));

        let replaced = control_plane
            .plan_replace(
                "session-1",
                "plan-1",
                3,
                json!({ "kind": "refactor", "summary": "Refactor", "steps": [] }),
            )
            .expect("replace");
        assert_eq!(replaced["planId"].as_str(), Some("plan-1"));
        assert_eq!(replaced["kind"].as_str(), Some("refactor"));
        assert_eq!(replaced["revision"].as_u64(), Some(4));

        assert_eq!(control_plane.plan_list("session-1").as_array().map(Vec::len), Some(1));
        let got = control_plane.plan_get("session-1", "plan-1").expect("get");
        assert_eq!(got["planId"].as_str(), Some("plan-1"));

        // Cross-session and stale mutations fail closed.
        assert!(control_plane.plan_get("session-2", "plan-1").is_err());
        let stale = control_plane
            .plan_merge("session-1", "plan-1", 1, json!({ "name": "x" }))
            .expect_err("stale merge must fail closed");
        assert!(stale.starts_with("stale_revision:"), "{stale}");
    }

    #[test]
    fn graph_ask_wait_control_plane_surface_binds_lists_and_resumes() {
        let registry = Arc::new(
            ToolRegistrySnapshot::from_descriptors("test-ask-snapshot", vec![host_mediated_ask_descriptor()])
                .expect("ask registry must build"),
        );
        let dispatcher = Arc::new(GovernedDispatcher::new(
            registry,
            Arc::new(FakeClock::new(1_000)),
        ));
        let mut graph_store = GraphRunStore::new();
        GraphRunner::new().start_run(
            &mut graph_store,
            GraphEngine::new("state-machine-v1").start_run("run-ask", "ask flow", Some("session-1")),
        );
        let control_plane = HostControlPlaneBuilder::new()
            .ask_dispatcher(Arc::clone(&dispatcher))
            .graph_run_store(graph_store)
            .build();

        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            run_id: Some("run-ask".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        };
        let outcome = dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:ask".to_string(),
                call_id: "call-1".to_string(),
                arguments: json!({ "text": "continue?" }),
            },
            &context,
        );
        let request_id = outcome.control_outcome.expect("ask control outcome").request_id;
        let pending = dispatcher.pending_request(&request_id).expect("pending ask");

        let binding = GraphAskWaitBinding {
            request_id: request_id.clone(),
            expected_version: pending.version,
            run_id: "run-ask".to_string(),
            turn_id: pending.turn_id.clone(),
            session_id: pending.session_id.clone(),
            call_id: pending.call_id.clone(),
            tool_name: "Ask".to_string(),
            assistant_transcript: json!({ "toolCalls": [{ "id": pending.call_id }] }),
            created_at_ms: 1_000,
        };
        let bound = control_plane
            .graph_bind_ask_wait("run-ask", binding)
            .expect("bind ask wait");
        assert_eq!(bound["phase"].as_str(), Some("waiting_user"));

        let waits = control_plane.graph_list_ask_waits("run-ask");
        assert_eq!(waits.as_array().map(Vec::len), Some(1));
        assert_eq!(waits[0]["requestId"].as_str(), Some(request_id.as_str()));
        assert_eq!(waits[0]["expectedVersion"].as_u64(), Some(pending.version));

        // Phase-4 P1-2: graph_resume_ask refuses to resume a request that has not been
        // CAS-consumed through the Ask control surface — it must not bypass the single
        // consumption entry point with an arbitrary answer.
        let unconsumed = control_plane
            .graph_resume_ask("run-ask", &request_id, pending.version, json!("continue"))
            .expect_err("resume of an unconsumed request must fail closed");
        assert!(
            unconsumed.contains("CAS-consumed") || unconsumed.contains("cannot resume"),
            "{unconsumed}"
        );

        // The host answers through the shared dispatcher (CAS consumes the pending request).
        let authorization =
            crate::agent::dispatcher::ControlRequestAuthorization::for_request(
                &pending,
                Some(json!("continue")),
            );
        let consumed = control_plane
            .ask_dispatcher
            .answer_control_request(&request_id, &authorization)
            .expect("answer must succeed on shared dispatcher");
        assert_eq!(
            consumed.request.state,
            PendingControlRequestState::Consumed
        );

        // Stale version rejected by the graph resume (binding version must be presented, not the
        // post-consumption request version).
        let stale = control_plane
            .graph_resume_ask("run-ask", &request_id, pending.version + 1, json!("continue"))
            .expect_err("stale graph resume must fail closed");
        assert!(stale.contains("stale"), "{stale}");

        let resumed = control_plane
            .graph_resume_ask("run-ask", &request_id, pending.version, json!("continue"))
            .expect("resume");
        assert_eq!(resumed["callId"].as_str(), Some("call-1"));
        assert_eq!(resumed["terminalResult"]["toolCallId"].as_str(), Some("call-1"));
        assert_eq!(resumed["terminalResult"]["output"]["answer"].as_str(), Some("continue"));

        assert_eq!(control_plane.graph_list_ask_waits("run-ask").as_array().map(Vec::len), Some(0));
    }

    struct ForcedToolPlanner {
        tool_name: String,
        arguments: serde_json::Value,
    }

    impl TurnPlanner for ForcedToolPlanner {
        fn preflight_decision(
            &self,
            _user_message: &str,
            _history: &[crate::agent::session::TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        ) -> Option<crate::agent::provider::ProviderDecision> {
            Some(crate::agent::provider::ProviderDecision {
                output_text: String::new(),
                tool_call: Some(ToolCall {
                    call_id: Some("forced-stream-tool-call".to_string()),
                    name: self.tool_name.clone(),
                    arguments: self.arguments.clone(),
                    plan: Some(ToolPlan {
                        kind: "forced".to_string(),
                        summary: format!("强制执行工具 `{}`。", self.tool_name),
                        parallel: false,
                        continue_on_error: false,
                        steps: Vec::new(),
                    }),
                }),
                reasoning_content: None,
                reasoning_content_value: None,
                assistant_message: None,
                provider_source: "planner_preflight".to_string(),
                provider_mode: "preflight".to_string(),
                fallback_reason: None,
                token_usage: None,
            })
        }

        fn select_tool_call(
            &self,
            _user_message: &str,
            _history: &[crate::agent::session::TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
            provider_tool_call: Option<ToolCall>,
        ) -> Option<ToolCall> {
            provider_tool_call
        }
    }

    #[test]
    fn graph_run_stream_ask_suspends_returns_waiting_user_and_resumes_unique_terminal() {
        // PA-076 P1-1: the frontend's main path is the graph-run stream. A real streamed turn
        // must persist the Ask pending request against run/turn facts, `execute_graph_run_stream`
        // must return a `WaitingUser` suspension (provider never called again), and the host
        // answering through the shared dispatcher must let `graph_resume_ask` inject exactly one
        // terminal result keyed to the original tool-call id.
        let rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let workspace = std::env::temp_dir().join(format!(
            "pony-ask-cp-stream-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&workspace).expect("create temp workspace for ask cp stream test");
        let server = MockHttpServer::start(Vec::new());
        let runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver {
                selection: test_chat_provider_selection(server.base_url.clone()),
            }),
            Box::new(crate::agent::governed_executor::build_governed_executor(
                Some(workspace),
                None,
            )),
            Box::new(ForcedToolPlanner {
                tool_name: "Ask".to_string(),
                arguments: json!({ "text": "继续吗？", "description": "test" }),
            }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        let control_plane = HostControlPlaneBuilder::new().runtime(runtime).build();

        let (started, prepared) = control_plane
            .prepare_start_graph_run_stream(StartGraphRunStreamCommand {
                turn_id: "run-ask-stream-turn-1".to_string(),
                run_id: Some("run-ask-stream".to_string()),
                goal: "ask stream flow".to_string(),
                input: TurnInput {
                    message: "start the ask stream run".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: Some("session-ask-stream".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("graph stream run should prepare");
        assert_eq!(started.run.phase, GraphRunPhase::Running);

        let suspended = control_plane
            .execute_graph_run_stream(&NoopSink, prepared)
            .expect("graph stream run should execute");
        assert_eq!(suspended.turn_result.phase, SUSPENDED_TURN_PHASE);
        assert!(suspended
            .turn_result
            .fallback_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("control_outcome_pending")));
        assert_eq!(suspended.run.phase, GraphRunPhase::WaitingUser);
        assert_eq!(suspended.decision.kind, GraphDecisionKind::WaitUser);
        assert_eq!(
            suspended.decision.reason,
            GraphDecisionReason::TurnCompletedAwaitingUser
        );
        assert_eq!(suspended.event.kind, GraphRunEventKind::Updated);

        // The Ask request was persisted against the real run/turn, and the run is bound.
        let pending = control_plane.ask_dispatcher.pending_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_kind, PendingControlRequestKind::Interaction);
        assert_eq!(pending[0].session_id.as_deref(), Some("session-ask-stream"));
        assert_eq!(pending[0].run_id.as_deref(), Some("run-ask-stream"));
        assert_eq!(pending[0].turn_id, "run-ask-stream-turn-1");
        assert_eq!(pending[0].prompt.as_deref(), Some("继续吗？"));
        let request_id = pending[0].request_id.clone();
        let expected_version = pending[0].version;
        let original_call_id = pending[0].call_id.clone();
        assert_eq!(
            control_plane
                .graph_list_ask_waits("run-ask-stream")
                .as_array()
                .map(Vec::len),
            Some(1)
        );

        // Host answers through the shared dispatcher (same store the runtime persisted into).
        let authorization =
            crate::agent::dispatcher::ControlRequestAuthorization::for_request(
                &pending[0],
                Some(json!("继续")),
            );
        let consumed = control_plane
            .ask_dispatcher
            .answer_control_request(&request_id, &authorization)
            .expect("answer must succeed on shared dispatcher");
        assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
        assert_eq!(consumed.answer.as_ref().and_then(Value::as_str), Some("继续"));

        // Graph resume injects exactly one terminal result for the original tool-call id.
        let resumed = control_plane
            .graph_resume_ask("run-ask-stream", &request_id, expected_version, json!("继续"))
            .expect("graph resume must succeed");
        assert_eq!(resumed["callId"].as_str(), Some(original_call_id.as_str()));
        assert_eq!(
            resumed["terminalResult"]["toolCallId"].as_str(),
            Some(original_call_id.as_str())
        );
        assert_eq!(
            resumed["terminalResult"]["output"]["answer"].as_str(),
            Some("继续")
        );
        assert_eq!(
            control_plane
                .graph_list_ask_waits("run-ask-stream")
                .as_array()
                .map(Vec::len),
            Some(0)
        );

        server.finish();
        drop(rt_guard);
    }

    #[test]
    fn graph_run_stream_normal_tool_turn_completes() {
        // Regression: the app.s exact entry (prepare_start_graph_run_stream →
        // execute_graph_run_stream) with a real workspace tool + followup must complete, not hang.
        let rt_guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let workspace = std::env::temp_dir().join(format!(
            "pony-repro-glob-cp-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&workspace).expect("create temp workspace for repro");
        std::fs::write(workspace.join("a.txt"), "hello").expect("write file");
        // Streaming (SSE) followup response: the assistant's final answer after the glob tool
        // executes — mirrors the real app's DeepSeek streaming provider.
        let sse_chunks = [
            json!({"choices": [{"delta": {"content": "done listing files"}}]}),
            json!({"choices": [{"delta": {"content": "\n"}}]}),
            json!({"choices": [{"delta": {}}]}),
        ];
        let mut sse_body = String::new();
        for chunk in &sse_chunks {
            sse_body.push_str(&format!("data: {chunk}\n\n"));
        }
        sse_body.push_str("data: [DONE]\n\n");
        let server = MockHttpServer::start(vec![MockHttpResponse {
            content_type: "text/event-stream",
            body: sse_body,
        }]);
        let runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver {
                selection: test_provider_selection(server.base_url.clone()),
            }),
            Box::new(crate::agent::governed_executor::build_governed_executor(None, None)),
            Box::new(ForcedToolPlanner {
                tool_name: "workspace_glob_files".to_string(),
                arguments: json!({ "pattern": "**/*" }),
            }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        let control_plane = HostControlPlaneBuilder::new().runtime(runtime).build();

        let (started, prepared) = control_plane
            .prepare_start_graph_run_stream(StartGraphRunStreamCommand {
                turn_id: "run-repro-glob-turn-1".to_string(),
                run_id: Some("run-repro-glob".to_string()),
                goal: "repro glob flow".to_string(),
                input: TurnInput {
                    message: "list files".to_string(),
                    display_message: None,
                    provider_id: None,
                    model_id: None,
                    reasoning_effort: None,
                    workspace_mode: None,
                    session_id: Some("session-repro-glob".to_string()),
                    node_id: None,
                    history: Vec::new(),
                    images: Vec::new(),
                    workspace_id: None,
                },
            })
            .expect("repro graph stream should prepare");
        assert_eq!(started.run.phase, GraphRunPhase::Running);

        let result = control_plane
            .execute_graph_run_stream(&NoopSink, prepared)
            .expect("repro graph stream should execute and return");
        // A normal tool turn must NOT suspend on an Ask wait.
        assert_ne!(
            result.turn_result.phase, SUSPENDED_TURN_PHASE,
            "normal glob tool turn must not suspend"
        );
        server.finish();
        drop(rt_guard);
    }
}
