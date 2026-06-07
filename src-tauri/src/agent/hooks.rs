use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const AGENT_HOOKS_CONTRACT_VERSION: &str = "agent-hooks-v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TurnHookPoint {
    TurnPrepareStart,
    TurnPrepareEnd,
    ContextBuildStart,
    ContextBuildEnd,
    ModelCallStart,
    ModelResponseEnd,
    ToolCallStart,
    ToolCallEnd,
    CheckpointPersistStart,
    CheckpointPersistEnd,
    TurnFinalizeStart,
    TurnFinalizeEnd,
    PlannerTurnPreflight,
    PlannerToolSelection,
    PlannerGraphDecision,
    CapabilityResolve,
    SkillToolActionsResolve,
    CapabilitySourceIngress,
    SkillSourceIngress,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalTurnPhase {
    Created,
    Preparing,
    BuildingContext,
    CallingModel,
    StreamingResponse,
    ExecutingTool,
    ToolResultIntegrating,
    Checkpointing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalTurnEventType {
    TurnCreated,
    TurnPhaseChanged,
    TurnContextBuilt,
    TurnModelCallStarted,
    TurnFirstToken,
    TurnOutputDelta,
    TurnToolCallStarted,
    TurnToolCallCompleted,
    TurnTraceUpdated,
    TurnCheckpointPersisted,
    TurnCompleted,
    TurnFailed,
    TurnCancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RunHookPoint {
    RunStart,
    SubmissionPlanResolved,
    WaitUser,
    StopRequested,
    RunPaused,
    RunResume,
    RunCompleted,
    RunFailed,
    RunCancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWriteHookPoint {
    LongTermMemoryWrite,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PlannerHookPoint {
    TurnPreflight,
    ToolSelection,
    GraphDecision,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityMediationHookPoint {
    CapabilityResolve,
    SkillToolActionsResolve,
    McpSourceIngress,
    SkillSourceIngress,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStateHookPoint {
    HistoryCheckoutStart,
    HistoryCheckoutResolved,
    BranchRestoreStart,
    BranchRestoreResolved,
    BranchForkStart,
    BranchForkResolved,
    BranchSwitchStart,
    BranchSwitchResolved,
}

pub fn turn_hook_point_for_planner_hook_point(hook_point: &PlannerHookPoint) -> TurnHookPoint {
    match hook_point {
        PlannerHookPoint::TurnPreflight => TurnHookPoint::PlannerTurnPreflight,
        PlannerHookPoint::ToolSelection => TurnHookPoint::PlannerToolSelection,
        PlannerHookPoint::GraphDecision => TurnHookPoint::PlannerGraphDecision,
    }
}

pub fn turn_hook_point_for_capability_mediation_hook_point(
    hook_point: &CapabilityMediationHookPoint,
) -> TurnHookPoint {
    match hook_point {
        CapabilityMediationHookPoint::CapabilityResolve => TurnHookPoint::CapabilityResolve,
        CapabilityMediationHookPoint::SkillToolActionsResolve => {
            TurnHookPoint::SkillToolActionsResolve
        }
        CapabilityMediationHookPoint::McpSourceIngress => TurnHookPoint::CapabilitySourceIngress,
        CapabilityMediationHookPoint::SkillSourceIngress => TurnHookPoint::SkillSourceIngress,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWriteTarget {
    LongTermMemory,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWriteOperation {
    Insert,
    Update,
    Noop,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWriteIntentRecord {
    pub key: String,
    pub kind: String,
    pub content: String,
    pub content_summary: String,
    pub source: String,
    pub operation: MemoryWriteOperation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWriteHookEnvelope {
    pub session_id: Option<String>,
    pub hook_point: MemoryWriteHookPoint,
    pub target: MemoryWriteTarget,
    pub source_boundary: String,
    pub user_message_summary: String,
    pub writes: Vec<MemoryWriteIntentRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlannerFactsEnvelope {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub hook_point: PlannerHookPoint,
    pub source_boundary: String,
    pub planner_source: String,
    pub user_message_summary: Option<String>,
    pub history_turn_count: usize,
    pub available_skill_ids: Vec<String>,
    pub provider_decision_summary: Option<String>,
    pub provider_tool_call_name: Option<String>,
    pub graph_goal_summary: Option<String>,
    pub graph_step_count: Option<usize>,
    pub current_decision_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityMediationEnvelope {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub hook_point: CapabilityMediationHookPoint,
    pub source_boundary: String,
    pub mediation_source: String,
    pub requested_capability_id: Option<String>,
    pub requested_skill_id: Option<String>,
    pub capability_kind: Option<String>,
    pub candidate_ids: Vec<String>,
    pub argument_summary: String,
    pub source_id: Option<String>,
    pub source_kind: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStateCommandKind {
    CheckoutHistoryNode,
    RestoreBranchHead,
    ForkFromHistoryNode,
    SwitchHistoryBranch,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStateCursorSummary {
    pub visible_node_id: Option<String>,
    pub active_branch_id: Option<String>,
    pub branch_head_node_id: Option<String>,
    pub workspace_node_id: Option<String>,
    pub mode: String,
    pub checkout_mode: String,
    pub checkout_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStateHookEnvelope {
    pub session_id: String,
    pub hook_point: HistoryStateHookPoint,
    pub command_kind: HistoryStateCommandKind,
    pub source_boundary: String,
    pub requested_node_id: Option<String>,
    pub requested_branch_id: Option<String>,
    pub requested_checkout_mode: Option<String>,
    pub resolved_node_id: Option<String>,
    pub resolved_branch_id: Option<String>,
    pub transcript_restore_applied: bool,
    pub workspace_rollback_capable: bool,
    pub workspace_rollback_applied: bool,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub cursor_summary: HistoryStateCursorSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStateHookEvidence {
    pub evidence_id: String,
    pub session_id: String,
    pub boundary: String,
    pub command_kind: String,
    pub result_kind: String,
    pub summary: String,
    pub elapsed_ms: u64,
    pub blocked: bool,
    pub degraded: bool,
    pub requested_node_id: Option<String>,
    pub requested_branch_id: Option<String>,
    pub resolved_node_id: Option<String>,
    pub resolved_branch_id: Option<String>,
    pub recorded_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedEffectEvidence {
    pub evidence_id: String,
    pub effect_kind: String,
    pub boundary: String,
    pub target_session_id: Option<String>,
    #[serde(default)]
    pub source_history_node_id: Option<String>,
    pub target_summary: String,
    pub persistence_ref: String,
    pub replay_decision_basis: String,
    pub persisted_at_ms: u64,
    pub replay_required_if_missing: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalGraphRunPhase {
    Ready,
    Running,
    WaitingUser,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalGraphRunEventType {
    RunStarted,
    RunUpdated,
    RunPaused,
    RunCompleted,
    RunFailed,
    RunCancelled,
    SubmissionPlanResolved,
    StopRequested,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunHookLifecycleBinding {
    pub hook_point: RunHookPoint,
    pub canonical_event_types: Vec<CanonicalGraphRunEventType>,
    pub canonical_phases: Vec<CanonicalGraphRunPhase>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionControlCommandKind {
    StartGraphRunStream,
    ContinueGraphRunStream,
    ResumeGraphRunStream,
    StopGraphRun,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunControlHookEnvelope {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub phase: String,
    pub command: ExecutionControlCommandKind,
    pub source: String,
    pub checkpoint_kind: Option<String>,
    pub recovery_mode: Option<String>,
    pub resumable: bool,
    pub replayable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunControlCheckpointContext {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub phase: String,
    pub checkpoint_kind: String,
    pub recovery_mode: String,
    pub resumable: bool,
    pub replayable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookLifecycleBinding {
    pub hook_point: TurnHookPoint,
    pub canonical_event_types: Vec<CanonicalTurnEventType>,
    pub canonical_phases: Vec<CanonicalTurnPhase>,
}

impl CanonicalTurnPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            CanonicalTurnPhase::Created => "created",
            CanonicalTurnPhase::Preparing => "preparing",
            CanonicalTurnPhase::BuildingContext => "building_context",
            CanonicalTurnPhase::CallingModel => "calling_model",
            CanonicalTurnPhase::StreamingResponse => "streaming_response",
            CanonicalTurnPhase::ExecutingTool => "executing_tool",
            CanonicalTurnPhase::ToolResultIntegrating => "tool_result_integrating",
            CanonicalTurnPhase::Checkpointing => "checkpointing",
            CanonicalTurnPhase::Completed => "completed",
            CanonicalTurnPhase::Failed => "failed",
            CanonicalTurnPhase::Cancelled => "cancelled",
        }
    }
}

impl CanonicalTurnEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CanonicalTurnEventType::TurnCreated => "turn.created",
            CanonicalTurnEventType::TurnPhaseChanged => "turn.phase_changed",
            CanonicalTurnEventType::TurnContextBuilt => "turn.context_built",
            CanonicalTurnEventType::TurnModelCallStarted => "turn.model_call_started",
            CanonicalTurnEventType::TurnFirstToken => "turn.first_token",
            CanonicalTurnEventType::TurnOutputDelta => "turn.output_delta",
            CanonicalTurnEventType::TurnToolCallStarted => "turn.tool_call_started",
            CanonicalTurnEventType::TurnToolCallCompleted => "turn.tool_call_completed",
            CanonicalTurnEventType::TurnTraceUpdated => "turn.trace_updated",
            CanonicalTurnEventType::TurnCheckpointPersisted => "turn.checkpoint_persisted",
            CanonicalTurnEventType::TurnCompleted => "turn.completed",
            CanonicalTurnEventType::TurnFailed => "turn.failed",
            CanonicalTurnEventType::TurnCancelled => "turn.cancelled",
        }
    }
}

impl CanonicalGraphRunPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            CanonicalGraphRunPhase::Ready => "ready",
            CanonicalGraphRunPhase::Running => "running",
            CanonicalGraphRunPhase::WaitingUser => "waiting_user",
            CanonicalGraphRunPhase::Paused => "paused",
            CanonicalGraphRunPhase::Completed => "completed",
            CanonicalGraphRunPhase::Failed => "failed",
            CanonicalGraphRunPhase::Cancelled => "cancelled",
        }
    }
}

impl CanonicalGraphRunEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CanonicalGraphRunEventType::RunStarted => "graph_run.started",
            CanonicalGraphRunEventType::RunUpdated => "graph_run.updated",
            CanonicalGraphRunEventType::RunPaused => "graph_run.paused",
            CanonicalGraphRunEventType::RunCompleted => "graph_run.completed",
            CanonicalGraphRunEventType::RunFailed => "graph_run.failed",
            CanonicalGraphRunEventType::RunCancelled => "graph_run.cancelled",
            CanonicalGraphRunEventType::SubmissionPlanResolved => {
                "graph_run.submission_plan_resolved"
            }
            CanonicalGraphRunEventType::StopRequested => "graph_run.stop_requested",
        }
    }
}

impl ExecutionControlCommandKind {
    pub fn as_submission_command(&self) -> &'static str {
        match self {
            ExecutionControlCommandKind::StartGraphRunStream => "start_graph_run_stream",
            ExecutionControlCommandKind::ContinueGraphRunStream => "continue_graph_run_stream",
            ExecutionControlCommandKind::ResumeGraphRunStream => "resume_graph_run_stream",
            ExecutionControlCommandKind::StopGraphRun => "stop_graph_run",
        }
    }
}

pub fn execution_control_command_kind_from_submission_command(
    command: &str,
) -> Option<ExecutionControlCommandKind> {
    match command.trim() {
        "start_graph_run_stream" => Some(ExecutionControlCommandKind::StartGraphRunStream),
        "continue_graph_run_stream" => Some(ExecutionControlCommandKind::ContinueGraphRunStream),
        "resume_graph_run_stream" => Some(ExecutionControlCommandKind::ResumeGraphRunStream),
        "stop_graph_run" => Some(ExecutionControlCommandKind::StopGraphRun),
        _ => None,
    }
}

pub fn canonical_graph_run_phase_for_submission_plan(
    command: &ExecutionControlCommandKind,
    checkpoint: Option<&RunControlCheckpointContext>,
) -> CanonicalGraphRunPhase {
    match command {
        ExecutionControlCommandKind::ResumeGraphRunStream => CanonicalGraphRunPhase::Paused,
        ExecutionControlCommandKind::StartGraphRunStream => CanonicalGraphRunPhase::Ready,
        ExecutionControlCommandKind::ContinueGraphRunStream => {
            let Some(checkpoint) = checkpoint else {
                return CanonicalGraphRunPhase::Ready;
            };
            match normalize_graph_run_phase_token(&checkpoint.phase).as_str() {
                "waiting_user" => CanonicalGraphRunPhase::WaitingUser,
                "paused" => CanonicalGraphRunPhase::Paused,
                _ => CanonicalGraphRunPhase::Ready,
            }
        }
        ExecutionControlCommandKind::StopGraphRun => CanonicalGraphRunPhase::Running,
    }
}

pub fn build_submission_plan_run_control_hook_envelope(
    command: &str,
    run_id: Option<&str>,
    source: &str,
    checkpoint: Option<&RunControlCheckpointContext>,
) -> Option<RunControlHookEnvelope> {
    let command = execution_control_command_kind_from_submission_command(command)?;
    let phase = canonical_graph_run_phase_for_submission_plan(&command, checkpoint)
        .as_str()
        .to_string();
    Some(RunControlHookEnvelope {
        session_id: checkpoint.and_then(|item| item.session_id.clone()),
        run_id: run_id
            .map(str::to_string)
            .or_else(|| checkpoint.and_then(|item| item.run_id.clone())),
        phase,
        command,
        source: source.to_string(),
        checkpoint_kind: checkpoint.map(|item| item.checkpoint_kind.clone()),
        recovery_mode: checkpoint.map(|item| item.recovery_mode.clone()),
        resumable: checkpoint.map(|item| item.resumable).unwrap_or(false),
        replayable: checkpoint.map(|item| item.replayable).unwrap_or(false),
    })
}

pub fn planner_transform_allowed_paths(hook_point: &PlannerHookPoint) -> &'static [&'static str] {
    match hook_point {
        PlannerHookPoint::TurnPreflight => &["provider_decision", "provider_tool_call"],
        PlannerHookPoint::ToolSelection => &[
            "provider_tool_call",
            "selected_tool_call",
            "selected_skill_id",
        ],
        PlannerHookPoint::GraphDecision => &["decision_summary"],
    }
}

pub fn planner_transform_readonly_paths(hook_point: &PlannerHookPoint) -> &'static [&'static str] {
    match hook_point {
        PlannerHookPoint::TurnPreflight => &[
            "planner_source",
            "user_message_summary",
            "history_turn_count",
            "available_skill_ids",
        ],
        PlannerHookPoint::ToolSelection => &[
            "planner_source",
            "user_message_summary",
            "history_turn_count",
            "available_skill_ids",
        ],
        PlannerHookPoint::GraphDecision => &[
            "planner_source",
            "graph_goal_summary",
            "graph_step_count",
            "decision_kind",
            "target_phase",
        ],
    }
}

pub fn planner_transform_operation_allowed(
    hook_point: &PlannerHookPoint,
    operation: &HookPatchOperation,
) -> bool {
    operation.target == HookPatchTarget::PlannerFacts
        && planner_transform_allowed_paths(hook_point)
            .iter()
            .any(|path| *path == operation.path)
}

pub fn capability_mediation_transform_allowed_paths(
    hook_point: &CapabilityMediationHookPoint,
) -> &'static [&'static str] {
    match hook_point {
        CapabilityMediationHookPoint::CapabilityResolve => &["request.arguments"],
        CapabilityMediationHookPoint::SkillToolActionsResolve => &["request.arguments"],
        CapabilityMediationHookPoint::McpSourceIngress => &[],
        CapabilityMediationHookPoint::SkillSourceIngress => &[],
    }
}

pub fn capability_mediation_readonly_paths(
    hook_point: &CapabilityMediationHookPoint,
) -> &'static [&'static str] {
    match hook_point {
        CapabilityMediationHookPoint::CapabilityResolve => &[
            "request.capability_id",
            "capability_kind",
            "candidate_ids",
            "source_id",
            "source_kind",
        ],
        CapabilityMediationHookPoint::SkillToolActionsResolve => &[
            "request.skill_id",
            "capability_kind",
            "candidate_ids",
            "source_id",
            "source_kind",
        ],
        CapabilityMediationHookPoint::McpSourceIngress => {
            &["source_id", "source_kind", "candidate_ids"]
        }
        CapabilityMediationHookPoint::SkillSourceIngress => {
            &["source_id", "source_kind", "candidate_ids"]
        }
    }
}

pub fn capability_mediation_transform_operation_allowed(
    hook_point: &CapabilityMediationHookPoint,
    operation: &HookPatchOperation,
) -> bool {
    operation.target == HookPatchTarget::CapabilityMediation
        && capability_mediation_transform_allowed_paths(hook_point)
            .iter()
            .any(|path| *path == operation.path)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookClass {
    Observe,
    Guard,
    Transform,
    SideEffect,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookFailurePolicy {
    Ignore,
    Degrade,
    FailTurn,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookRecoveryMode {
    ReplayRequired,
    PersistedEffect,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookResultKind {
    Observe,
    Allow,
    Deny,
    Patch,
    SideEffectRequest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HookPatchTarget {
    TurnInput,
    ContextPayload,
    ModelRequest,
    MemoryWriteIntent,
    PlannerFacts,
    CapabilityMediation,
    ToolArguments,
    ToolResult,
    CheckpointMetadata,
    TurnOutput,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookPatchOperationKind {
    Set,
    Merge,
    Remove,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookPatchOperation {
    pub target: HookPatchTarget,
    pub path: String,
    pub operation: HookPatchOperationKind,
    pub value_summary: Option<String>,
    #[serde(default)]
    pub value_text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookDenyDecision {
    pub reason_code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookSideEffectRequest {
    pub request_kind: String,
    pub summary: String,
    pub requires_persistence_evidence: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "resultKind", content = "payload", rename_all = "snake_case")]
pub enum HookStructuredResult {
    Observe { summary: String },
    Allow { summary: String },
    Deny(HookDenyDecision),
    Patch { operations: Vec<HookPatchOperation> },
    SideEffectRequest(HookSideEffectRequest),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookTraceRequirements {
    pub include_name: bool,
    pub include_hook_point: bool,
    pub include_elapsed_ms: bool,
    pub include_result_summary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookReplayRequirements {
    pub include_hook_order: bool,
    pub include_input_summary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookSideEffectPersistenceRequirements {
    pub require_persistence_evidence: bool,
    pub require_effect_summary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentHookDescriptor {
    pub contract_version: String,
    pub name: String,
    pub class: HookClass,
    pub priority: i32,
    pub timeout_ms: u64,
    pub allowed_hook_points: Vec<TurnHookPoint>,
    pub allowed_result_kinds: Vec<HookResultKind>,
    pub can_block: bool,
    pub default_failure_policy: HookFailurePolicy,
    pub allowed_failure_policies: Vec<HookFailurePolicy>,
    pub default_recovery_mode: HookRecoveryMode,
    pub trace_requirements: HookTraceRequirements,
    pub replay_requirements: HookReplayRequirements,
    pub side_effect_persistence_requirements: HookSideEffectPersistenceRequirements,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookPatchConflictPolicy {
    Reject,
    FirstWriteWins,
    LastWriteWins,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookPatchConflict {
    pub target: HookPatchTarget,
    pub path: String,
    pub existing_hook_name: String,
    pub incoming_hook_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookPatchMergeOutcome {
    pub operations: Vec<HookPatchOperationEnvelope>,
    pub conflicts: Vec<HookPatchConflict>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookPatchOperationEnvelope {
    pub hook_name: String,
    pub operation: HookPatchOperation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookExecutionResult {
    pub hook_name: String,
    pub hook_class: HookClass,
    pub hook_point: TurnHookPoint,
    pub hook_order: u32,
    pub result_kind: HookResultKind,
    pub structured_result: HookStructuredResult,
    pub blocked: bool,
    pub elapsed_ms: u64,
    pub input_summary: Option<String>,
    pub persistence_evidence_ref: Option<String>,
    pub trace_summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookTraceRecord {
    pub hook_name: String,
    pub hook_class: HookClass,
    pub hook_point: TurnHookPoint,
    pub hook_order: u32,
    pub result_kind: HookResultKind,
    pub structured_result: HookStructuredResult,
    pub blocked: bool,
    pub elapsed_ms: u64,
    pub input_summary: Option<String>,
    pub persistence_evidence_ref: Option<String>,
    pub summary: String,
}

pub fn build_observe_hook_trace_record(
    hook_name: impl Into<String>,
    hook_point: TurnHookPoint,
    hook_order: u32,
    summary: impl Into<String>,
    input_summary: Option<String>,
) -> HookTraceRecord {
    let summary = summary.into();
    HookTraceRecord {
        hook_name: hook_name.into(),
        hook_class: HookClass::Observe,
        hook_point,
        hook_order,
        result_kind: HookResultKind::Observe,
        structured_result: HookStructuredResult::Observe {
            summary: summary.clone(),
        },
        blocked: false,
        elapsed_ms: 0,
        input_summary,
        persistence_evidence_ref: None,
        summary,
    }
}

impl HookExecutionResult {
    pub fn to_trace_record(&self) -> HookTraceRecord {
        HookTraceRecord {
            hook_name: self.hook_name.clone(),
            hook_class: self.hook_class.clone(),
            hook_point: self.hook_point.clone(),
            hook_order: self.hook_order,
            result_kind: self.result_kind.clone(),
            structured_result: self.structured_result.clone(),
            blocked: self.blocked,
            elapsed_ms: self.elapsed_ms,
            input_summary: self.input_summary.clone(),
            persistence_evidence_ref: self.persistence_evidence_ref.clone(),
            summary: self.trace_summary.clone(),
        }
    }

    pub fn guard_decision(
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
        decision: HookStructuredResult,
        input_summary: Option<String>,
    ) -> Result<Self, String> {
        if descriptor.class != HookClass::Guard {
            return Err(format!(
                "Hook `{}` is not a guard hook and cannot produce guard decisions.",
                descriptor.name
            ));
        }
        if !descriptor.allowed_hook_points.contains(&hook_point) {
            return Err(format!(
                "Hook `{}` cannot run on hook point `{:?}`.",
                descriptor.name, hook_point
            ));
        }

        let (result_kind, blocked, trace_summary) = match &decision {
            HookStructuredResult::Allow { summary } => {
                (HookResultKind::Allow, false, summary.clone())
            }
            HookStructuredResult::Deny(deny) => {
                if !descriptor.can_block {
                    return Err(format!(
                        "Hook `{}` cannot deny execution because can_block is false.",
                        descriptor.name
                    ));
                }
                (
                    HookResultKind::Deny,
                    true,
                    format!("guard denied execution: {}", deny.reason_code),
                )
            }
            other => {
                return Err(format!(
                    "Hook `{}` guard decision must be allow or deny, got `{:?}`.",
                    descriptor.name, other
                ));
            }
        };

        if !descriptor.allowed_result_kinds.contains(&result_kind) {
            return Err(format!(
                "Hook `{}` does not allow guard result `{:?}`.",
                descriptor.name, result_kind
            ));
        }

        Ok(Self {
            hook_name: descriptor.name.clone(),
            hook_class: descriptor.class.clone(),
            hook_point,
            hook_order: 0,
            result_kind,
            structured_result: decision,
            blocked,
            elapsed_ms: 0,
            input_summary,
            persistence_evidence_ref: None,
            trace_summary,
        })
    }
}

pub struct AgentHookRegistry {
    descriptors: Vec<AgentHookDescriptor>,
}

impl AgentHookRegistry {
    pub fn new() -> Self {
        Self {
            descriptors: Vec::new(),
        }
    }

    pub fn register(&mut self, descriptor: AgentHookDescriptor) -> Result<(), String> {
        validate_descriptor(&descriptor)?;
        if self
            .descriptors
            .iter()
            .any(|existing| existing.name == descriptor.name)
        {
            return Err(format!("Hook `{}` is already registered.", descriptor.name));
        }
        self.descriptors.push(descriptor);
        Ok(())
    }

    pub fn list(&self) -> &[AgentHookDescriptor] {
        &self.descriptors
    }

    pub fn list_for_hook_point(&self, hook_point: &TurnHookPoint) -> Vec<&AgentHookDescriptor> {
        let mut descriptors = self
            .descriptors
            .iter()
            .enumerate()
            .filter(|(_, descriptor)| descriptor.allowed_hook_points.contains(hook_point))
            .collect::<Vec<_>>();
        descriptors.sort_by(|(left_index, left), (right_index, right)| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left_index.cmp(right_index))
        });
        descriptors
            .into_iter()
            .map(|(_, descriptor)| descriptor)
            .collect()
    }
}

impl Default for AgentHookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub trait AgentHookExecutor: Send + Sync {
    fn execute(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
    ) -> Result<HookExecutionResult, String>;

    fn execute_planner(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: PlannerHookPoint,
        _envelope: &PlannerFactsEnvelope,
    ) -> Result<HookExecutionResult, String> {
        self.execute(
            descriptor,
            turn_hook_point_for_planner_hook_point(&hook_point),
        )
    }

    fn execute_capability_mediation(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: CapabilityMediationHookPoint,
        _envelope: &CapabilityMediationEnvelope,
    ) -> Result<HookExecutionResult, String> {
        self.execute(
            descriptor,
            turn_hook_point_for_capability_mediation_hook_point(&hook_point),
        )
    }
}

pub struct NoopHookExecutor;

pub trait MemoryWriteHookExecutor: Send + Sync {
    fn execute(
        &self,
        envelope: &MemoryWriteHookEnvelope,
    ) -> Result<Vec<HookExecutionResult>, String>;
}

pub struct NoopMemoryWriteHookExecutor;

pub trait HistoryStateHookExecutor: Send + Sync {
    fn execute(
        &self,
        envelope: &HistoryStateHookEnvelope,
    ) -> Result<Vec<HookExecutionResult>, String>;
}

pub struct NoopHistoryStateHookExecutor;

impl AgentHookExecutor for NoopHookExecutor {
    fn execute(
        &self,
        descriptor: &AgentHookDescriptor,
        hook_point: TurnHookPoint,
    ) -> Result<HookExecutionResult, String> {
        if !descriptor.allowed_hook_points.contains(&hook_point) {
            return Err(format!(
                "Hook `{}` cannot run on hook point `{:?}`.",
                descriptor.name, hook_point
            ));
        }

        let (result_kind, structured_result) = normalized_result_for_class(&descriptor.class);

        if !descriptor.allowed_result_kinds.contains(&result_kind) {
            return Err(format!(
                "Hook `{}` does not allow normalized result `{:?}`.",
                descriptor.name, result_kind
            ));
        }

        Ok(HookExecutionResult {
            hook_name: descriptor.name.clone(),
            hook_class: descriptor.class.clone(),
            hook_point,
            hook_order: 0,
            result_kind,
            structured_result,
            blocked: false,
            elapsed_ms: 0,
            input_summary: Some("noop hook input summary placeholder".to_string()),
            persistence_evidence_ref: None,
            trace_summary: "noop hook executor applied normalized result".to_string(),
        })
    }
}

impl MemoryWriteHookExecutor for NoopMemoryWriteHookExecutor {
    fn execute(
        &self,
        _envelope: &MemoryWriteHookEnvelope,
    ) -> Result<Vec<HookExecutionResult>, String> {
        Ok(Vec::new())
    }
}

impl HistoryStateHookExecutor for NoopHistoryStateHookExecutor {
    fn execute(
        &self,
        _envelope: &HistoryStateHookEnvelope,
    ) -> Result<Vec<HookExecutionResult>, String> {
        Ok(Vec::new())
    }
}

fn validate_descriptor(descriptor: &AgentHookDescriptor) -> Result<(), String> {
    if descriptor.contract_version.trim().is_empty() {
        return Err("Hook contract_version is empty.".to_string());
    }
    if descriptor.name.trim().is_empty() {
        return Err("Hook name is empty.".to_string());
    }
    if descriptor.allowed_hook_points.is_empty() {
        return Err(format!(
            "Hook `{}` must declare at least one hook point.",
            descriptor.name
        ));
    }
    if descriptor.allowed_result_kinds.is_empty() {
        return Err(format!(
            "Hook `{}` must declare at least one result kind.",
            descriptor.name
        ));
    }
    if descriptor.timeout_ms == 0 {
        return Err(format!(
            "Hook `{}` timeout_ms must be greater than 0.",
            descriptor.name
        ));
    }
    if !descriptor
        .allowed_failure_policies
        .contains(&descriptor.default_failure_policy)
    {
        return Err(format!(
            "Hook `{}` default failure policy must be included in allowed_failure_policies.",
            descriptor.name
        ));
    }
    let allowed_result_kinds = allowed_result_kinds_for_class(&descriptor.class);
    if descriptor
        .allowed_result_kinds
        .iter()
        .any(|kind| !allowed_result_kinds.contains(kind))
    {
        return Err(format!(
            "Hook `{}` declares result kinds incompatible with hook class `{:?}`.",
            descriptor.name, descriptor.class
        ));
    }
    if descriptor
        .allowed_result_kinds
        .contains(&HookResultKind::Deny)
        && !descriptor.can_block
    {
        return Err(format!(
            "Hook `{}` deny-capable descriptors must set can_block=true.",
            descriptor.name
        ));
    }
    if descriptor.default_recovery_mode == HookRecoveryMode::PersistedEffect
        && !descriptor
            .side_effect_persistence_requirements
            .require_persistence_evidence
    {
        return Err(format!(
            "Hook `{}` persisted_effect hooks must require persistence evidence.",
            descriptor.name
        ));
    }
    Ok(())
}

fn allowed_result_kinds_for_class(class: &HookClass) -> &'static [HookResultKind] {
    match class {
        HookClass::Observe => &[HookResultKind::Observe],
        HookClass::Guard => &[HookResultKind::Allow, HookResultKind::Deny],
        HookClass::Transform => &[HookResultKind::Patch],
        HookClass::SideEffect => &[HookResultKind::SideEffectRequest],
    }
}

pub(crate) fn normalized_result_for_class(
    class: &HookClass,
) -> (HookResultKind, HookStructuredResult) {
    match class {
        HookClass::Observe => (
            HookResultKind::Observe,
            HookStructuredResult::Observe {
                summary: "hook observed lifecycle boundary without mutation".to_string(),
            },
        ),
        HookClass::Guard => (
            HookResultKind::Allow,
            HookStructuredResult::Allow {
                summary: "hook allowed runtime to continue".to_string(),
            },
        ),
        HookClass::Transform => (
            HookResultKind::Patch,
            HookStructuredResult::Patch {
                operations: vec![HookPatchOperation {
                    target: HookPatchTarget::ContextPayload,
                    path: "noop".to_string(),
                    operation: HookPatchOperationKind::Merge,
                    value_summary: Some("no-op normalized patch placeholder".to_string()),
                    value_text: None,
                }],
            },
        ),
        HookClass::SideEffect => (
            HookResultKind::SideEffectRequest,
            HookStructuredResult::SideEffectRequest(HookSideEffectRequest {
                request_kind: "noop".to_string(),
                summary: "hook requested canonical runtime side-effect handling".to_string(),
                requires_persistence_evidence: true,
            }),
        ),
    }
}

pub fn canonical_lifecycle_binding_for_hook_point(
    hook_point: &TurnHookPoint,
) -> HookLifecycleBinding {
    match hook_point {
        TurnHookPoint::TurnPrepareStart => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![
                CanonicalTurnEventType::TurnCreated,
                CanonicalTurnEventType::TurnPhaseChanged,
            ],
            canonical_phases: vec![CanonicalTurnPhase::Created, CanonicalTurnPhase::Preparing],
        },
        TurnHookPoint::TurnPrepareEnd => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnPhaseChanged],
            canonical_phases: vec![CanonicalTurnPhase::Preparing],
        },
        TurnHookPoint::ContextBuildStart => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnPhaseChanged],
            canonical_phases: vec![CanonicalTurnPhase::BuildingContext],
        },
        TurnHookPoint::ContextBuildEnd => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnContextBuilt],
            canonical_phases: vec![CanonicalTurnPhase::BuildingContext],
        },
        TurnHookPoint::ModelCallStart => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnModelCallStarted],
            canonical_phases: vec![CanonicalTurnPhase::CallingModel],
        },
        TurnHookPoint::ModelResponseEnd => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![
                CanonicalTurnEventType::TurnFirstToken,
                CanonicalTurnEventType::TurnOutputDelta,
            ],
            canonical_phases: vec![CanonicalTurnPhase::StreamingResponse],
        },
        TurnHookPoint::ToolCallStart => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnToolCallStarted],
            canonical_phases: vec![CanonicalTurnPhase::ExecutingTool],
        },
        TurnHookPoint::ToolCallEnd => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnToolCallCompleted],
            canonical_phases: vec![CanonicalTurnPhase::ToolResultIntegrating],
        },
        TurnHookPoint::CheckpointPersistStart => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnPhaseChanged],
            canonical_phases: vec![CanonicalTurnPhase::Checkpointing],
        },
        TurnHookPoint::CheckpointPersistEnd => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnCheckpointPersisted],
            canonical_phases: vec![CanonicalTurnPhase::Checkpointing],
        },
        TurnHookPoint::TurnFinalizeStart => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalTurnEventType::TurnPhaseChanged],
            canonical_phases: vec![CanonicalTurnPhase::Checkpointing],
        },
        TurnHookPoint::TurnFinalizeEnd => HookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![
                CanonicalTurnEventType::TurnCompleted,
                CanonicalTurnEventType::TurnFailed,
                CanonicalTurnEventType::TurnCancelled,
            ],
            canonical_phases: vec![
                CanonicalTurnPhase::Completed,
                CanonicalTurnPhase::Failed,
                CanonicalTurnPhase::Cancelled,
            ],
        },
        TurnHookPoint::PlannerTurnPreflight
        | TurnHookPoint::PlannerToolSelection
        | TurnHookPoint::PlannerGraphDecision
        | TurnHookPoint::CapabilityResolve
        | TurnHookPoint::SkillToolActionsResolve
        | TurnHookPoint::CapabilitySourceIngress
        | TurnHookPoint::SkillSourceIngress => {
            panic!(
                "planner/capability mediation hook points do not map to canonical turn lifecycle bindings"
            )
        }
    }
}

pub fn hook_point_matches_canonical_boundary(
    hook_point: &TurnHookPoint,
    event_type: &str,
    phase: &str,
) -> bool {
    let binding = canonical_lifecycle_binding_for_hook_point(hook_point);
    binding
        .canonical_event_types
        .iter()
        .any(|candidate| candidate.as_str() == event_type)
        && binding
            .canonical_phases
            .iter()
            .any(|candidate| candidate.as_str() == phase)
}

pub fn canonical_graph_run_binding_for_hook_point(
    hook_point: &RunHookPoint,
) -> RunHookLifecycleBinding {
    match hook_point {
        RunHookPoint::RunStart => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::RunStarted],
            canonical_phases: vec![
                CanonicalGraphRunPhase::Ready,
                CanonicalGraphRunPhase::Running,
            ],
        },
        RunHookPoint::SubmissionPlanResolved => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::SubmissionPlanResolved],
            canonical_phases: vec![
                CanonicalGraphRunPhase::Ready,
                CanonicalGraphRunPhase::WaitingUser,
                CanonicalGraphRunPhase::Paused,
            ],
        },
        RunHookPoint::WaitUser => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::RunUpdated],
            canonical_phases: vec![CanonicalGraphRunPhase::WaitingUser],
        },
        RunHookPoint::StopRequested => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::StopRequested],
            canonical_phases: vec![
                CanonicalGraphRunPhase::Running,
                CanonicalGraphRunPhase::WaitingUser,
            ],
        },
        RunHookPoint::RunPaused => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::RunPaused],
            canonical_phases: vec![CanonicalGraphRunPhase::Paused],
        },
        RunHookPoint::RunResume => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::RunUpdated],
            canonical_phases: vec![
                CanonicalGraphRunPhase::Ready,
                CanonicalGraphRunPhase::Running,
            ],
        },
        RunHookPoint::RunCompleted => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::RunCompleted],
            canonical_phases: vec![CanonicalGraphRunPhase::Completed],
        },
        RunHookPoint::RunFailed => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::RunFailed],
            canonical_phases: vec![CanonicalGraphRunPhase::Failed],
        },
        RunHookPoint::RunCancelled => RunHookLifecycleBinding {
            hook_point: hook_point.clone(),
            canonical_event_types: vec![CanonicalGraphRunEventType::RunCancelled],
            canonical_phases: vec![CanonicalGraphRunPhase::Cancelled],
        },
    }
}

pub fn run_hook_point_matches_canonical_boundary(
    hook_point: &RunHookPoint,
    event_type: &str,
    phase: &str,
) -> bool {
    let binding = canonical_graph_run_binding_for_hook_point(hook_point);
    binding
        .canonical_event_types
        .iter()
        .any(|candidate| candidate.as_str() == event_type)
        && binding
            .canonical_phases
            .iter()
            .any(|candidate| candidate.as_str() == phase)
}

fn normalize_graph_run_phase_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

pub fn merge_patch_results(
    results: &[HookExecutionResult],
    conflict_policy: HookPatchConflictPolicy,
) -> Result<HookPatchMergeOutcome, String> {
    let mut operations_by_key: BTreeMap<(HookPatchTarget, String), HookPatchOperationEnvelope> =
        BTreeMap::new();
    let mut conflicts = Vec::new();

    for result in results {
        let HookStructuredResult::Patch { operations } = &result.structured_result else {
            continue;
        };

        for operation in operations {
            let key = (operation.target.clone(), operation.path.clone());
            let incoming = HookPatchOperationEnvelope {
                hook_name: result.hook_name.clone(),
                operation: operation.clone(),
            };

            if let Some(existing) = operations_by_key.get(&key) {
                conflicts.push(HookPatchConflict {
                    target: operation.target.clone(),
                    path: operation.path.clone(),
                    existing_hook_name: existing.hook_name.clone(),
                    incoming_hook_name: result.hook_name.clone(),
                });
                match conflict_policy {
                    HookPatchConflictPolicy::Reject => {
                        return Err(format!(
                            "Patch conflict on `{:?}:{}` between hooks `{}` and `{}`.",
                            operation.target, operation.path, existing.hook_name, result.hook_name
                        ));
                    }
                    HookPatchConflictPolicy::FirstWriteWins => {}
                    HookPatchConflictPolicy::LastWriteWins => {
                        operations_by_key.insert(key, incoming);
                    }
                }
            } else {
                operations_by_key.insert(key, incoming);
            }
        }
    }

    Ok(HookPatchMergeOutcome {
        operations: operations_by_key.into_values().collect(),
        conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observe_descriptor(name: &str) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: AGENT_HOOKS_CONTRACT_VERSION.to_string(),
            name: name.to_string(),
            class: HookClass::Observe,
            priority: 100,
            timeout_ms: 1_000,
            allowed_hook_points: vec![TurnHookPoint::ModelCallStart],
            allowed_result_kinds: vec![HookResultKind::Observe],
            can_block: false,
            default_failure_policy: HookFailurePolicy::Ignore,
            allowed_failure_policies: vec![HookFailurePolicy::Ignore],
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

    fn transform_descriptor(name: &str) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: AGENT_HOOKS_CONTRACT_VERSION.to_string(),
            name: name.to_string(),
            class: HookClass::Transform,
            priority: 200,
            timeout_ms: 2_000,
            allowed_hook_points: vec![TurnHookPoint::ContextBuildEnd],
            allowed_result_kinds: vec![HookResultKind::Patch],
            can_block: false,
            default_failure_policy: HookFailurePolicy::Degrade,
            allowed_failure_policies: vec![HookFailurePolicy::Ignore, HookFailurePolicy::Degrade],
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

    fn side_effect_descriptor(name: &str) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: AGENT_HOOKS_CONTRACT_VERSION.to_string(),
            name: name.to_string(),
            class: HookClass::SideEffect,
            priority: 300,
            timeout_ms: 3_000,
            allowed_hook_points: vec![TurnHookPoint::CheckpointPersistEnd],
            allowed_result_kinds: vec![HookResultKind::SideEffectRequest],
            can_block: false,
            default_failure_policy: HookFailurePolicy::Degrade,
            allowed_failure_policies: vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn],
            default_recovery_mode: HookRecoveryMode::PersistedEffect,
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
                require_persistence_evidence: true,
                require_effect_summary: true,
            },
        }
    }

    fn guard_descriptor(name: &str) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: AGENT_HOOKS_CONTRACT_VERSION.to_string(),
            name: name.to_string(),
            class: HookClass::Guard,
            priority: 150,
            timeout_ms: 1_500,
            allowed_hook_points: vec![TurnHookPoint::ContextBuildStart],
            allowed_result_kinds: vec![HookResultKind::Allow, HookResultKind::Deny],
            can_block: true,
            default_failure_policy: HookFailurePolicy::FailTurn,
            allowed_failure_policies: vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn],
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

    #[test]
    fn registry_rejects_duplicate_hook_names() {
        let mut registry = AgentHookRegistry::new();
        registry
            .register(observe_descriptor("audit.observe"))
            .expect("first register should succeed");

        let duplicate = registry.register(observe_descriptor("audit.observe"));
        assert!(duplicate.is_err());
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn registry_lists_hook_point_in_stable_priority_order() {
        let mut registry = AgentHookRegistry::new();
        let mut second = observe_descriptor("observe.second");
        second.priority = 200;
        let mut first = observe_descriptor("observe.first");
        first.priority = 100;
        let mut tied = observe_descriptor("observe.tied");
        tied.priority = 100;

        registry.register(second).expect("register should succeed");
        registry.register(first).expect("register should succeed");
        registry.register(tied).expect("register should succeed");

        let ordered = registry.list_for_hook_point(&TurnHookPoint::ModelCallStart);
        let names = ordered
            .iter()
            .map(|descriptor| descriptor.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["observe.first", "observe.tied", "observe.second"]
        );
    }

    #[test]
    fn registry_rejects_descriptor_without_allowed_failure_policy_match() {
        let mut descriptor = observe_descriptor("audit.observe");
        descriptor.default_failure_policy = HookFailurePolicy::Degrade;

        let error = AgentHookRegistry::new()
            .register(descriptor)
            .expect_err("descriptor should be rejected");
        assert!(error.contains("default failure policy"));
    }

    #[test]
    fn registry_rejects_zero_timeout_descriptor() {
        let mut descriptor = observe_descriptor("audit.observe");
        descriptor.timeout_ms = 0;

        let error = AgentHookRegistry::new()
            .register(descriptor)
            .expect_err("descriptor should be rejected");
        assert!(error.contains("timeout_ms"));
    }

    #[test]
    fn registry_rejects_result_kind_incompatible_with_hook_class() {
        let mut descriptor = observe_descriptor("audit.observe");
        descriptor.allowed_result_kinds = vec![HookResultKind::Patch];

        let error = AgentHookRegistry::new()
            .register(descriptor)
            .expect_err("descriptor should be rejected");
        assert!(error.contains("incompatible with hook class"));
    }

    #[test]
    fn registry_rejects_deny_capable_guard_without_block_permission() {
        let mut descriptor = guard_descriptor("guard.input");
        descriptor.can_block = false;

        let error = AgentHookRegistry::new()
            .register(descriptor)
            .expect_err("descriptor should be rejected");
        assert!(error.contains("can_block=true"));
    }

    #[test]
    fn noop_executor_normalizes_observe_hook_result() {
        let descriptor = observe_descriptor("audit.observe");
        let executor = NoopHookExecutor;

        let result = executor
            .execute(&descriptor, TurnHookPoint::ModelCallStart)
            .expect("noop executor should succeed");

        assert_eq!(result.result_kind, HookResultKind::Observe);
        assert!(!result.blocked);
        assert_eq!(result.hook_name, "audit.observe");
        assert_eq!(result.hook_order, 0);
        assert_eq!(
            result.input_summary.as_deref(),
            Some("noop hook input summary placeholder")
        );
        assert_eq!(
            result.structured_result,
            HookStructuredResult::Observe {
                summary: "hook observed lifecycle boundary without mutation".to_string()
            }
        );
    }

    #[test]
    fn execution_result_can_be_projected_to_trace_record() {
        let descriptor = observe_descriptor("audit.observe");
        let executor = NoopHookExecutor;

        let result = executor
            .execute(&descriptor, TurnHookPoint::ModelCallStart)
            .expect("noop executor should succeed");
        let trace = result.to_trace_record();

        assert_eq!(trace.hook_name, "audit.observe");
        assert_eq!(trace.hook_class, HookClass::Observe);
        assert_eq!(trace.hook_point, TurnHookPoint::ModelCallStart);
        assert_eq!(trace.hook_order, 0);
        assert_eq!(trace.result_kind, HookResultKind::Observe);
        assert_eq!(trace.elapsed_ms, 0);
        assert_eq!(
            trace.input_summary.as_deref(),
            Some("noop hook input summary placeholder")
        );
    }

    #[test]
    fn guard_decision_can_project_deny_reason_to_trace() {
        let descriptor = guard_descriptor("guard.input");
        let result = HookExecutionResult::guard_decision(
            &descriptor,
            TurnHookPoint::ContextBuildStart,
            HookStructuredResult::Deny(HookDenyDecision {
                reason_code: "unsafe_context".to_string(),
                message: "context contains blocked segment".to_string(),
            }),
            Some("context-window".to_string()),
        )
        .expect("guard deny should succeed");
        let trace = result.to_trace_record();

        assert!(result.blocked);
        assert_eq!(result.result_kind, HookResultKind::Deny);
        assert_eq!(trace.result_kind, HookResultKind::Deny);
        match trace.structured_result {
            HookStructuredResult::Deny(deny) => {
                assert_eq!(deny.reason_code, "unsafe_context");
                assert_eq!(deny.message, "context contains blocked segment");
            }
            other => panic!("expected deny result, got {:?}", other),
        }
    }

    #[test]
    fn guard_decision_rejects_deny_when_descriptor_cannot_block() {
        let mut descriptor = guard_descriptor("guard.input");
        descriptor.can_block = false;

        let error = HookExecutionResult::guard_decision(
            &descriptor,
            TurnHookPoint::ContextBuildStart,
            HookStructuredResult::Deny(HookDenyDecision {
                reason_code: "unsafe_context".to_string(),
                message: "context contains blocked segment".to_string(),
            }),
            None,
        )
        .expect_err("deny should be rejected");

        assert!(error.contains("can_block is false"));
    }

    #[test]
    fn noop_executor_normalizes_transform_hook_to_structured_patch() {
        let descriptor = transform_descriptor("context.transform");
        let executor = NoopHookExecutor;

        let result = executor
            .execute(&descriptor, TurnHookPoint::ContextBuildEnd)
            .expect("noop executor should succeed");

        assert_eq!(result.result_kind, HookResultKind::Patch);
        match result.structured_result {
            HookStructuredResult::Patch { operations } => {
                assert_eq!(operations.len(), 1);
                assert_eq!(operations[0].target, HookPatchTarget::ContextPayload);
                assert_eq!(operations[0].operation, HookPatchOperationKind::Merge);
            }
            other => panic!("expected patch result, got {:?}", other),
        }
    }

    #[test]
    fn noop_executor_normalizes_side_effect_hook_to_persisted_request() {
        let descriptor = side_effect_descriptor("audit.side_effect");
        let executor = NoopHookExecutor;

        let result = executor
            .execute(&descriptor, TurnHookPoint::CheckpointPersistEnd)
            .expect("noop executor should succeed");
        let trace = result.to_trace_record();

        assert_eq!(result.result_kind, HookResultKind::SideEffectRequest);
        match trace.structured_result {
            HookStructuredResult::SideEffectRequest(request) => {
                assert_eq!(request.request_kind, "noop");
                assert!(request.requires_persistence_evidence);
            }
            other => panic!("expected side effect request, got {:?}", other),
        }
    }

    #[test]
    fn noop_history_state_executor_returns_empty_results() {
        let executor = NoopHistoryStateHookExecutor;

        let results = executor
            .execute(&HistoryStateHookEnvelope {
                session_id: "session-history".to_string(),
                hook_point: HistoryStateHookPoint::HistoryCheckoutStart,
                command_kind: HistoryStateCommandKind::CheckoutHistoryNode,
                source_boundary: "history.checkout.start".to_string(),
                requested_node_id: Some("node-1".to_string()),
                requested_branch_id: Some("branch-main".to_string()),
                requested_checkout_mode: Some("transcript_and_workspace".to_string()),
                resolved_node_id: None,
                resolved_branch_id: None,
                transcript_restore_applied: false,
                workspace_rollback_capable: false,
                workspace_rollback_applied: false,
                degraded: false,
                degradation_reason: None,
                cursor_summary: HistoryStateCursorSummary {
                    visible_node_id: Some("node-2".to_string()),
                    active_branch_id: Some("branch-main".to_string()),
                    branch_head_node_id: Some("node-2".to_string()),
                    workspace_node_id: Some("node-2".to_string()),
                    mode: "live".to_string(),
                    checkout_mode: "transcript_only".to_string(),
                    checkout_status: "not_requested".to_string(),
                },
            })
            .expect("noop history-state executor should succeed");

        assert!(results.is_empty());
    }

    #[test]
    fn merge_patch_results_rejects_conflicts_by_default_policy() {
        let results = vec![
            HookExecutionResult {
                hook_name: "transform.first".to_string(),
                hook_class: HookClass::Transform,
                hook_point: TurnHookPoint::ContextBuildEnd,
                result_kind: HookResultKind::Patch,
                hook_order: 1,
                structured_result: HookStructuredResult::Patch {
                    operations: vec![HookPatchOperation {
                        target: HookPatchTarget::ToolArguments,
                        path: "arguments.query".to_string(),
                        operation: HookPatchOperationKind::Set,
                        value_summary: Some("first".to_string()),
                        value_text: None,
                    }],
                },
                blocked: false,
                elapsed_ms: 0,
                input_summary: Some("query=first".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "first patch".to_string(),
            },
            HookExecutionResult {
                hook_name: "transform.second".to_string(),
                hook_class: HookClass::Transform,
                hook_point: TurnHookPoint::ContextBuildEnd,
                hook_order: 2,
                result_kind: HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch {
                    operations: vec![HookPatchOperation {
                        target: HookPatchTarget::ToolArguments,
                        path: "arguments.query".to_string(),
                        operation: HookPatchOperationKind::Set,
                        value_summary: Some("second".to_string()),
                        value_text: None,
                    }],
                },
                blocked: false,
                elapsed_ms: 0,
                input_summary: Some("query=second".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "second patch".to_string(),
            },
        ];

        let error = merge_patch_results(&results, HookPatchConflictPolicy::Reject)
            .expect_err("conflicting patch should be rejected");
        assert!(error.contains("Patch conflict"));
    }

    #[test]
    fn merge_patch_results_can_keep_last_writer_with_conflict_trace() {
        let results = vec![
            HookExecutionResult {
                hook_name: "transform.first".to_string(),
                hook_class: HookClass::Transform,
                hook_point: TurnHookPoint::ContextBuildEnd,
                result_kind: HookResultKind::Patch,
                hook_order: 1,
                structured_result: HookStructuredResult::Patch {
                    operations: vec![HookPatchOperation {
                        target: HookPatchTarget::ToolArguments,
                        path: "arguments.query".to_string(),
                        operation: HookPatchOperationKind::Set,
                        value_summary: Some("first".to_string()),
                        value_text: None,
                    }],
                },
                blocked: false,
                elapsed_ms: 0,
                input_summary: Some("query=first".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "first patch".to_string(),
            },
            HookExecutionResult {
                hook_name: "transform.second".to_string(),
                hook_class: HookClass::Transform,
                hook_point: TurnHookPoint::ContextBuildEnd,
                hook_order: 2,
                result_kind: HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch {
                    operations: vec![HookPatchOperation {
                        target: HookPatchTarget::ToolArguments,
                        path: "arguments.query".to_string(),
                        operation: HookPatchOperationKind::Set,
                        value_summary: Some("second".to_string()),
                        value_text: None,
                    }],
                },
                blocked: false,
                elapsed_ms: 0,
                input_summary: Some("query=second".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "second patch".to_string(),
            },
        ];

        let outcome = merge_patch_results(&results, HookPatchConflictPolicy::LastWriteWins)
            .expect("last writer wins should succeed");

        assert_eq!(outcome.operations.len(), 1);
        assert_eq!(outcome.operations[0].hook_name, "transform.second");
        assert_eq!(outcome.conflicts.len(), 1);
        assert_eq!(outcome.conflicts[0].existing_hook_name, "transform.first");
        assert_eq!(outcome.conflicts[0].incoming_hook_name, "transform.second");
    }

    #[test]
    fn registry_rejects_persisted_effect_without_persistence_requirement() {
        let mut descriptor = side_effect_descriptor("audit.side_effect");
        descriptor
            .side_effect_persistence_requirements
            .require_persistence_evidence = false;

        let error = AgentHookRegistry::new()
            .register(descriptor)
            .expect_err("descriptor should be rejected");
        assert!(error.contains("persisted_effect"));
    }

    #[test]
    fn hook_points_map_only_to_canonical_lifecycle_vocabulary() {
        let hook_points = [
            TurnHookPoint::TurnPrepareStart,
            TurnHookPoint::TurnPrepareEnd,
            TurnHookPoint::ContextBuildStart,
            TurnHookPoint::ContextBuildEnd,
            TurnHookPoint::ModelCallStart,
            TurnHookPoint::ModelResponseEnd,
            TurnHookPoint::ToolCallStart,
            TurnHookPoint::ToolCallEnd,
            TurnHookPoint::CheckpointPersistStart,
            TurnHookPoint::CheckpointPersistEnd,
            TurnHookPoint::TurnFinalizeStart,
            TurnHookPoint::TurnFinalizeEnd,
        ];

        for hook_point in hook_points {
            let binding = canonical_lifecycle_binding_for_hook_point(&hook_point);
            assert!(!binding.canonical_event_types.is_empty());
            assert!(!binding.canonical_phases.is_empty());
            for event_type in binding.canonical_event_types {
                assert!(event_type.as_str().starts_with("turn."));
            }
            for phase in binding.canonical_phases {
                assert!(!phase.as_str().trim().is_empty());
            }
        }
    }

    #[test]
    fn run_hook_points_map_only_to_canonical_graph_vocabulary() {
        let hook_points = [
            RunHookPoint::RunStart,
            RunHookPoint::SubmissionPlanResolved,
            RunHookPoint::WaitUser,
            RunHookPoint::StopRequested,
            RunHookPoint::RunPaused,
            RunHookPoint::RunResume,
            RunHookPoint::RunCompleted,
            RunHookPoint::RunFailed,
            RunHookPoint::RunCancelled,
        ];

        for hook_point in hook_points {
            let binding = canonical_graph_run_binding_for_hook_point(&hook_point);
            assert!(!binding.canonical_event_types.is_empty());
            assert!(!binding.canonical_phases.is_empty());
            for event_type in binding.canonical_event_types {
                assert!(event_type.as_str().starts_with("graph_run."));
            }
            for phase in binding.canonical_phases {
                assert!(!phase.as_str().trim().is_empty());
            }
        }
    }

    #[test]
    fn run_hook_point_binding_matches_wait_user_boundary() {
        let binding = canonical_graph_run_binding_for_hook_point(&RunHookPoint::WaitUser);

        assert_eq!(
            binding.canonical_event_types,
            vec![CanonicalGraphRunEventType::RunUpdated]
        );
        assert_eq!(
            binding.canonical_phases,
            vec![CanonicalGraphRunPhase::WaitingUser]
        );
        assert!(run_hook_point_matches_canonical_boundary(
            &RunHookPoint::WaitUser,
            "graph_run.updated",
            "waiting_user"
        ));
        assert!(!run_hook_point_matches_canonical_boundary(
            &RunHookPoint::WaitUser,
            "graph_run.paused",
            "paused"
        ));
    }

    #[test]
    fn submission_plan_hook_binding_matches_resume_boundary_without_new_command_source() {
        let binding =
            canonical_graph_run_binding_for_hook_point(&RunHookPoint::SubmissionPlanResolved);

        assert_eq!(
            binding.canonical_event_types,
            vec![CanonicalGraphRunEventType::SubmissionPlanResolved]
        );
        assert!(binding
            .canonical_phases
            .contains(&CanonicalGraphRunPhase::Paused));
        assert!(run_hook_point_matches_canonical_boundary(
            &RunHookPoint::SubmissionPlanResolved,
            "graph_run.submission_plan_resolved",
            "paused"
        ));
        assert!(!run_hook_point_matches_canonical_boundary(
            &RunHookPoint::SubmissionPlanResolved,
            "graph_run.updated",
            "paused"
        ));
    }

    #[test]
    fn submission_plan_envelope_normalizes_graph_run_continue_to_ready_boundary() {
        let envelope = build_submission_plan_run_control_hook_envelope(
            "continue_graph_run_stream",
            Some("run-continue"),
            "graph_run",
            None,
        )
        .expect("continue command should be supported");

        assert_eq!(envelope.phase, "ready");
        assert_eq!(
            envelope.command,
            ExecutionControlCommandKind::ContinueGraphRunStream
        );
        assert_eq!(envelope.source, "graph_run");
        assert!(!envelope.resumable);
        assert!(!envelope.replayable);
        assert!(run_hook_point_matches_canonical_boundary(
            &RunHookPoint::SubmissionPlanResolved,
            CanonicalGraphRunEventType::SubmissionPlanResolved.as_str(),
            &envelope.phase
        ));
    }

    #[test]
    fn submission_plan_envelope_preserves_checkpoint_waiting_user_boundary() {
        let checkpoint = RunControlCheckpointContext {
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            phase: "waiting_user".to_string(),
            checkpoint_kind: "recovery".to_string(),
            recovery_mode: "persisted_effect".to_string(),
            resumable: true,
            replayable: true,
        };

        let envelope = build_submission_plan_run_control_hook_envelope(
            "continue_graph_run_stream",
            Some("run-1"),
            "checkpoint",
            Some(&checkpoint),
        )
        .expect("continue command should be supported");

        assert_eq!(envelope.phase, "waiting_user");
        assert_eq!(envelope.checkpoint_kind.as_deref(), Some("recovery"));
        assert_eq!(envelope.recovery_mode.as_deref(), Some("persisted_effect"));
        assert!(envelope.resumable);
        assert!(envelope.replayable);
        assert!(run_hook_point_matches_canonical_boundary(
            &RunHookPoint::SubmissionPlanResolved,
            CanonicalGraphRunEventType::SubmissionPlanResolved.as_str(),
            &envelope.phase
        ));
    }

    #[test]
    fn model_response_end_maps_to_streaming_response_boundary() {
        let binding = canonical_lifecycle_binding_for_hook_point(&TurnHookPoint::ModelResponseEnd);

        assert_eq!(
            binding.canonical_phases,
            vec![CanonicalTurnPhase::StreamingResponse]
        );
        assert!(binding
            .canonical_event_types
            .contains(&CanonicalTurnEventType::TurnFirstToken));
        assert!(binding
            .canonical_event_types
            .contains(&CanonicalTurnEventType::TurnOutputDelta));
        assert!(hook_point_matches_canonical_boundary(
            &TurnHookPoint::ModelResponseEnd,
            "turn.output_delta",
            "streaming_response"
        ));
    }

    #[test]
    fn tool_call_end_maps_to_tool_result_integrating_phase() {
        let binding = canonical_lifecycle_binding_for_hook_point(&TurnHookPoint::ToolCallEnd);

        assert_eq!(
            binding.canonical_event_types,
            vec![CanonicalTurnEventType::TurnToolCallCompleted]
        );
        assert_eq!(
            binding.canonical_phases,
            vec![CanonicalTurnPhase::ToolResultIntegrating]
        );
        assert!(hook_point_matches_canonical_boundary(
            &TurnHookPoint::ToolCallEnd,
            "turn.tool_call_completed",
            "tool_result_integrating"
        ));
        assert!(!hook_point_matches_canonical_boundary(
            &TurnHookPoint::ToolCallEnd,
            "turn.tool_call_completed",
            "executing_tool"
        ));
    }

    #[test]
    fn turn_finalize_end_maps_only_to_terminal_events_and_phases() {
        let binding = canonical_lifecycle_binding_for_hook_point(&TurnHookPoint::TurnFinalizeEnd);

        assert_eq!(
            binding.canonical_event_types,
            vec![
                CanonicalTurnEventType::TurnCompleted,
                CanonicalTurnEventType::TurnFailed,
                CanonicalTurnEventType::TurnCancelled,
            ]
        );
        assert_eq!(
            binding.canonical_phases,
            vec![
                CanonicalTurnPhase::Completed,
                CanonicalTurnPhase::Failed,
                CanonicalTurnPhase::Cancelled,
            ]
        );
        assert!(hook_point_matches_canonical_boundary(
            &TurnHookPoint::TurnFinalizeEnd,
            "turn.completed",
            "completed"
        ));
        assert!(hook_point_matches_canonical_boundary(
            &TurnHookPoint::TurnFinalizeEnd,
            "turn.failed",
            "failed"
        ));
        assert!(hook_point_matches_canonical_boundary(
            &TurnHookPoint::TurnFinalizeEnd,
            "turn.cancelled",
            "cancelled"
        ));
    }

    #[test]
    fn planner_transform_whitelist_keeps_scheduler_fields_readonly() {
        assert_eq!(
            planner_transform_allowed_paths(&PlannerHookPoint::GraphDecision),
            vec!["decision_summary"]
        );
        assert!(
            planner_transform_readonly_paths(&PlannerHookPoint::GraphDecision)
                .contains(&"decision_kind")
        );
        assert!(
            planner_transform_readonly_paths(&PlannerHookPoint::GraphDecision)
                .contains(&"target_phase")
        );
    }

    #[test]
    fn planner_transform_operation_requires_planner_facts_target_and_whitelisted_path() {
        let allowed = HookPatchOperation {
            target: HookPatchTarget::PlannerFacts,
            path: "selected_tool_call".to_string(),
            operation: HookPatchOperationKind::Set,
            value_summary: Some("workspace.read_file".to_string()),
            value_text: Some("{\"name\":\"workspace.read_file\"}".to_string()),
        };
        let denied = HookPatchOperation {
            target: HookPatchTarget::PlannerFacts,
            path: "decision_kind".to_string(),
            operation: HookPatchOperationKind::Set,
            value_summary: Some("continue".to_string()),
            value_text: Some("continue".to_string()),
        };

        assert!(planner_transform_operation_allowed(
            &PlannerHookPoint::ToolSelection,
            &allowed
        ));
        assert!(!planner_transform_operation_allowed(
            &PlannerHookPoint::GraphDecision,
            &denied
        ));
    }

    #[test]
    fn capability_mediation_transform_whitelist_keeps_identity_fields_readonly() {
        assert_eq!(
            capability_mediation_transform_allowed_paths(
                &CapabilityMediationHookPoint::CapabilityResolve
            ),
            vec!["request.arguments"]
        );
        assert!(capability_mediation_readonly_paths(
            &CapabilityMediationHookPoint::CapabilityResolve
        )
        .contains(&"request.capability_id"));
        assert!(capability_mediation_readonly_paths(
            &CapabilityMediationHookPoint::SkillToolActionsResolve
        )
        .contains(&"request.skill_id"));
    }

    #[test]
    fn capability_mediation_transform_operation_rejects_source_ingress_mutation() {
        let ingress_operation = HookPatchOperation {
            target: HookPatchTarget::CapabilityMediation,
            path: "request.arguments".to_string(),
            operation: HookPatchOperationKind::Merge,
            value_summary: Some("normalized ingress rewrite".to_string()),
            value_text: Some("{\"visibility\":\"hidden\"}".to_string()),
        };
        let skill_operation = HookPatchOperation {
            target: HookPatchTarget::CapabilityMediation,
            path: "request.arguments".to_string(),
            operation: HookPatchOperationKind::Merge,
            value_summary: Some("skill arguments".to_string()),
            value_text: Some("{\"topic\":\"hooks\"}".to_string()),
        };

        assert!(!capability_mediation_transform_operation_allowed(
            &CapabilityMediationHookPoint::SkillSourceIngress,
            &ingress_operation
        ));
        assert!(capability_mediation_transform_operation_allowed(
            &CapabilityMediationHookPoint::SkillToolActionsResolve,
            &skill_operation
        ));
    }
}
