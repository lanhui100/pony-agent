use crate::agent::context::RetrievedContextState;
use crate::agent::hooks::RunControlHookEnvelope;
use crate::agent::planner::{GraphPlanner, GraphPlanningContext};
use crate::agent::runtime::TurnResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

const GRAPH_CONTRACT_VERSION: &str = "graph-run-contract-v1";
const GRAPH_STEP_TITLE_MAX_CHARS: usize = 48;

type GraphRunMap = HashMap<String, GraphRun>;

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedGraphRunStore {
    #[serde(default)]
    runs: GraphRunMap,
    #[serde(default)]
    ask_waits: BTreeMap<String, Vec<GraphAskWaitBinding>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunPhase {
    Ready,
    Running,
    WaitingUser,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunStopReason {
    UserStop,
    Timeout,
    BudgetExhausted,
    ConsecutiveError,
    RuntimeCancelled,
    RuntimeFailed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum GraphStepKind {
    Turn,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphDecisionKind {
    Continue,
    WaitUser,
    Pause,
    Complete,
    Fail,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphDecisionReason {
    RuntimeStillRunning,
    TurnCompletedAwaitingUser,
    PlannerRequestedContinue,
    TurnFailed,
    TurnCancelled,
    ExplicitPause,
    ExplicitCompletion,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct GraphStep {
    pub id: String,
    pub kind: GraphStepKind,
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub phase: GraphRunPhase,
    pub title: String,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphDecision {
    pub kind: GraphDecisionKind,
    pub reason: GraphDecisionReason,
    pub summary: String,
    pub target_phase: GraphRunPhase,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct GraphRun {
    pub id: String,
    pub goal: String,
    pub session_id: Option<String>,
    pub phase: GraphRunPhase,
    #[serde(default)]
    pub steps: Vec<GraphStep>,
    #[serde(default)]
    pub active_turn_id: Option<String>,
    #[serde(default)]
    pub last_completed_turn_id: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<GraphRunStopReason>,
    #[serde(default)]
    pub last_handoff: Option<GraphTurnHandoff>,
    #[serde(default)]
    pub resume_count: u32,
    pub last_decision: Option<GraphDecision>,
    #[serde(default)]
    pub control_boundary_evidence: Vec<GraphRunControlBoundaryEvidence>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunEventKind {
    Started,
    Updated,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunEvent {
    pub run_id: String,
    pub kind: GraphRunEventKind,
    pub phase: GraphRunPhase,
    pub summary: String,
    pub step_count: usize,
    pub updated_at_ms: u64,
    pub hook_point: Option<String>,
    pub canonical_event_type: Option<String>,
    pub canonical_phase: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunControlBoundaryEvidence {
    pub hook_point: String,
    pub canonical_event_type: String,
    pub canonical_phase: String,
    pub summary: String,
    pub hook_envelope: RunControlHookEnvelope,
    pub created_at_ms: u64,
}

/// An Ask (`PendingControlRequest`) wait bound to a graph run's `wait_user` suspension
/// (design.md Decision 5, PA-076 task 4.3/4.4). The binding is keyed by `request_id` + the
/// `expected_version` the host must answer with; the original assistant tool-call transcript is
/// preserved alongside the pending request so a reload never loses the open roundtrip.
///
/// Bindings are stored as a sidecar map in the [`GraphRunStore`] (keyed by `run_id`) rather than
/// as a field on [`GraphRun`], so the persisted graph contract stays backward compatible and the
/// `GraphRun` struct shape is untouched for consumers that construct it from a checkpoint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphAskWaitBinding {
    pub request_id: String,
    /// Version the host must present to answer/resume this Ask. Rejecting a different version is
    /// the graph's stale-version CAS guard.
    pub expected_version: u64,
    pub run_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Original assistant tool-call id; the resumed run injects exactly one terminal tool result
    /// for this id.
    pub call_id: String,
    /// Product/model-visible tool name of the originating Ask call.
    pub tool_name: String,
    /// Original assistant tool-call transcript persisted alongside the pending request.
    pub assistant_transcript: Value,
    pub created_at_ms: u64,
}

/// The result of resuming a bound Ask wait: the run has been moved back to `Ready` and the caller
/// receives exactly one terminal tool result to inject for the original `call_id` before the run
/// continues. There is deliberately no additional provider follow-up produced by the graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAskResumeOutcome {
    pub request_id: String,
    pub run_id: String,
    pub turn_id: String,
    /// Original assistant tool-call id the terminal result must be injected for.
    pub call_id: String,
    pub tool_name: String,
    /// The user's answer payload.
    pub answer: Value,
    /// The single terminal tool result for `call_id` after resume.
    pub terminal_result: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunLifecycle {
    pub run: GraphRun,
    pub event: GraphRunEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunAdvance {
    pub run: GraphRun,
    pub handoff: GraphTurnHandoff,
    pub decision: GraphDecision,
    pub event: GraphRunEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphRunCheckpoint {
    pub contract_version: String,
    pub run_id: String,
    pub goal: String,
    pub session_id: Option<String>,
    pub phase: GraphRunPhase,
    #[serde(default)]
    pub active_turn_id: Option<String>,
    #[serde(default)]
    pub last_completed_turn_id: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<GraphRunStopReason>,
    #[serde(default)]
    pub steps: Vec<GraphStep>,
    pub last_decision: Option<GraphDecision>,
    #[serde(default)]
    pub last_handoff: Option<GraphTurnHandoff>,
    #[serde(default)]
    pub resume_count: u32,
    #[serde(default)]
    pub control_boundary_evidence: Vec<GraphRunControlBoundaryEvidence>,
    pub resumable: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct GraphTurnHandoff {
    pub contract_version: String,
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_phase: String,
    pub checkpoint_status: Option<String>,
    pub checkpoint_phase: Option<String>,
    pub user_message: String,
    pub assistant_message: String,
    pub session_summary: String,
    pub conversation_id: String,
    pub session_turn_count: usize,
    pub run_id: Option<String>,
    pub run_phase: Option<String>,
    pub active_task_focus: Option<String>,
    pub acceptance_focus: Option<String>,
    pub closeout_focus: Option<String>,
    pub last_referenced_file: Option<String>,
    pub recent_attachment_asset_count: usize,
    pub long_term_memory_status: String,
    pub long_term_memory_entry_count: usize,
    pub trace_step_count: usize,
    pub tool_activity_count: usize,
    pub provider_name: String,
    pub provider_model: String,
}

pub struct GraphEngine {
    name: &'static str,
}

pub struct GraphRunStore {
    runs: GraphRunMap,
    /// Pending Ask wait bindings keyed by run id (design.md Decision 5). Persisted alongside the
    /// runs in the same JSON file; `#[serde(default)]` keeps old stores readable.
    ask_waits: BTreeMap<String, Vec<GraphAskWaitBinding>>,
    storage_path: Option<PathBuf>,
}

pub struct GraphRunner;

impl GraphEngine {
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn contract_version(&self) -> &str {
        GRAPH_CONTRACT_VERSION
    }

    #[allow(dead_code)]
    pub fn start_run(
        &self,
        run_id: impl Into<String>,
        goal: impl Into<String>,
        session_id: Option<&str>,
    ) -> GraphRun {
        let now = now_timestamp_ms();
        GraphRun {
            id: run_id.into(),
            goal: goal.into(),
            session_id: session_id.map(str::to_string),
            phase: GraphRunPhase::Ready,
            steps: Vec::new(),
            active_turn_id: None,
            last_completed_turn_id: None,
            stop_reason: None,
            last_handoff: None,
            resume_count: 0,
            last_decision: None,
            control_boundary_evidence: Vec::new(),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    #[allow(dead_code)]
    pub fn build_turn_handoff(
        &self,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        retrieved: &RetrievedContextState,
    ) -> GraphTurnHandoff {
        GraphTurnHandoff {
            contract_version: self.contract_version().to_string(),
            turn_id: turn_id.map(str::to_string),
            session_id: session_id
                .map(str::to_string)
                .or_else(|| Some(retrieved.session_context.conversation_id.clone())),
            turn_phase: result.phase.clone(),
            checkpoint_status: retrieved.run_state.execution_checkpoint_status.clone(),
            checkpoint_phase: retrieved.run_state.execution_checkpoint_phase.clone(),
            user_message: result.user_message.clone(),
            assistant_message: result.assistant_message.clone(),
            session_summary: retrieved.session_context.summary.clone(),
            conversation_id: retrieved.session_context.conversation_id.clone(),
            session_turn_count: retrieved.session_context.turn_count,
            run_id: retrieved.run_state.run_id.clone(),
            run_phase: retrieved.run_state.phase.clone(),
            active_task_focus: extract_active_task_focus(&retrieved.long_term_memory.entries),
            acceptance_focus: extract_acceptance_focus(&retrieved.long_term_memory.entries),
            closeout_focus: extract_closeout_focus(&retrieved.long_term_memory.entries),
            last_referenced_file: retrieved.session_context.last_referenced_file.clone(),
            recent_attachment_asset_count: retrieved.session_context.recent_attachment_assets.len(),
            long_term_memory_status: retrieved.long_term_memory.status.clone(),
            long_term_memory_entry_count: retrieved.long_term_memory.entries.len(),
            trace_step_count: result.trace_steps.len(),
            tool_activity_count: result.tool_activities.len(),
            provider_name: result.provider_name.clone(),
            provider_model: result.provider_model.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn decide_after_turn(&self, handoff: &GraphTurnHandoff) -> GraphDecision {
        if handoff
            .checkpoint_status
            .as_deref()
            .map(normalize_phase_label)
            == Some("running")
        {
            return GraphDecision {
                kind: GraphDecisionKind::Continue,
                reason: GraphDecisionReason::RuntimeStillRunning,
                summary: "当前 turn 尚未收口，graph 不开启下一轮，只继续观察 runtime 收口。"
                    .to_string(),
                target_phase: GraphRunPhase::Running,
            };
        }

        match normalize_handoff_phase(handoff) {
            "failed" => GraphDecision {
                kind: GraphDecisionKind::Fail,
                reason: GraphDecisionReason::TurnFailed,
                summary: "当前 turn 已失败，graph 应把该 run 收口到 failed。".to_string(),
                target_phase: GraphRunPhase::Failed,
            },
            "cancelled" => GraphDecision {
                kind: GraphDecisionKind::Cancel,
                reason: GraphDecisionReason::TurnCancelled,
                summary: "当前 turn 已取消，graph 应把该 run 收口到 cancelled。".to_string(),
                target_phase: GraphRunPhase::Cancelled,
            },
            _ => GraphDecision {
                kind: GraphDecisionKind::WaitUser,
                reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                summary:
                    "当前 turn 已完整收口；若没有更高层 planner 明确要求继续，graph 默认等待用户输入。"
                        .to_string(),
                target_phase: GraphRunPhase::WaitingUser,
            },
        }
    }

    #[allow(dead_code)]
    pub fn decide_after_turn_with_planner(
        &self,
        run: &GraphRun,
        handoff: &GraphTurnHandoff,
        planner: &dyn GraphPlanner,
    ) -> GraphDecision {
        if handoff
            .checkpoint_status
            .as_deref()
            .map(normalize_phase_label)
            == Some("running")
        {
            return self.decide_after_turn(handoff);
        }

        match normalize_handoff_phase(handoff) {
            "failed" | "cancelled" => self.decide_after_turn(handoff),
            _ => planner.decide_after_turn(GraphPlanningContext::from_run(run, handoff)),
        }
    }

    #[allow(dead_code)]
    pub fn pause_decision(&self, summary: impl Into<String>) -> GraphDecision {
        GraphDecision {
            kind: GraphDecisionKind::Pause,
            reason: GraphDecisionReason::ExplicitPause,
            summary: summary.into(),
            target_phase: GraphRunPhase::Paused,
        }
    }

    #[allow(dead_code)]
    pub fn complete_decision(&self, summary: impl Into<String>) -> GraphDecision {
        GraphDecision {
            kind: GraphDecisionKind::Complete,
            reason: GraphDecisionReason::ExplicitCompletion,
            summary: summary.into(),
            target_phase: GraphRunPhase::Completed,
        }
    }
}

impl GraphRunStore {
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
            ask_waits: BTreeMap::new(),
            storage_path: None,
        }
    }

    pub fn persistent(storage_path: impl Into<PathBuf>) -> Self {
        let storage_path = storage_path.into();
        let mut persisted = load_persisted_store(&storage_path);
        let mut runs = std::mem::take(&mut persisted.runs);
        let modified = reconcile_stale_runs(&mut runs);
        let store = Self {
            runs,
            ask_waits: persisted.ask_waits,
            storage_path: Some(storage_path),
        };
        if modified {
            store.persist_runs();
        }
        store
    }

    pub fn load_run(&self, run_id: &str) -> Option<GraphRun> {
        self.runs.get(run_id).cloned()
    }

    pub fn list_runs(&self) -> Vec<GraphRun> {
        let mut runs = self.runs.values().cloned().collect::<Vec<_>>();
        runs.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        runs
    }

    fn save_run(&mut self, run: GraphRun) -> GraphRun {
        self.runs.insert(run.id.clone(), run.clone());
        self.persist_runs();
        run
    }

    fn persist_runs(&self) {
        let Some(storage_path) = &self.storage_path else {
            return;
        };
        let Some(parent) = storage_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(serialized) = serde_json::to_string_pretty(&PersistedGraphRunStore {
            runs: self.runs.clone(),
            ask_waits: self.ask_waits.clone(),
        }) else {
            return;
        };
        let _ = fs::write(storage_path, serialized);
    }
}

impl GraphRunner {
    pub fn new() -> Self {
        Self
    }

    pub fn start_run(&self, store: &mut GraphRunStore, run: GraphRun) -> GraphRunLifecycle {
        let run = store.save_run(run);
        GraphRunLifecycle {
            event: build_run_event(
                &run,
                GraphRunEventKind::Started,
                "Graph run created and waiting for the first turn.".to_string(),
            ),
            run,
        }
    }

    pub fn begin_turn(
        &self,
        store: &mut GraphRunStore,
        run_id: &str,
        turn_id: &str,
        session_id: Option<&str>,
    ) -> Option<GraphRunLifecycle> {
        let mut run = store.load_run(run_id)?;
        if matches!(
            run.phase,
            GraphRunPhase::Completed | GraphRunPhase::Failed | GraphRunPhase::Cancelled
        ) {
            return None;
        }
        // A run with unresolved Ask waits is suspended at `wait_user`; it must be resumed through
        // the Ask path, never by beginning a fresh turn (design.md Decision 5).
        if has_unresolved_ask_waits(store, run_id) {
            return None;
        }

        run.phase = GraphRunPhase::Running;
        run.active_turn_id = Some(turn_id.to_string());
        run.stop_reason = None;
        run.updated_at_ms = now_timestamp_ms();
        if run.session_id.is_none() {
            run.session_id = session_id.map(str::to_string);
        }
        let run = store.save_run(run);
        Some(GraphRunLifecycle {
            event: build_run_event(
                &run,
                GraphRunEventKind::Updated,
                "Graph run entered running and is preparing the next turn.".to_string(),
            ),
            run,
        })
    }

    pub fn apply_turn_result(
        &self,
        store: &mut GraphRunStore,
        run_id: &str,
        handoff: GraphTurnHandoff,
        decision: GraphDecision,
    ) -> Option<GraphRunAdvance> {
        let mut run = store.load_run(run_id)?;
        // A run with unresolved Ask waits cannot consume a normal turn result; the Ask path owns
        // the resume until every binding is resolved (design.md Decision 5).
        if has_unresolved_ask_waits(store, run_id) {
            return None;
        }
        let effective_decision = match (&decision.kind, &run.stop_reason) {
            (GraphDecisionKind::Cancel, Some(GraphRunStopReason::UserStop)) => GraphDecision {
                kind: GraphDecisionKind::Pause,
                reason: GraphDecisionReason::ExplicitPause,
                summary: "Graph run stopped by user request and is waiting to resume.".to_string(),
                target_phase: GraphRunPhase::Paused,
            },
            _ => decision.clone(),
        };
        let step = GraphStep {
            id: format!("{}-step-{}", run.id, run.steps.len() + 1),
            kind: GraphStepKind::Turn,
            turn_id: handoff.turn_id.clone(),
            session_id: handoff.session_id.clone(),
            phase: effective_decision.target_phase.clone(),
            title: build_graph_step_title(&handoff.user_message),
            updated_at_ms: now_timestamp_ms(),
        };

        run.session_id = handoff
            .session_id
            .clone()
            .or(run.session_id.clone())
            .or_else(|| Some(handoff.conversation_id.clone()));
        run.phase = effective_decision.target_phase.clone();
        run.active_turn_id = None;
        run.last_completed_turn_id = handoff.turn_id.clone();
        run.last_handoff = Some(handoff.clone());
        run.stop_reason = match effective_decision.kind {
            GraphDecisionKind::Pause => run.stop_reason.or(Some(GraphRunStopReason::UserStop)),
            GraphDecisionKind::Fail => Some(GraphRunStopReason::RuntimeFailed),
            GraphDecisionKind::Cancel => Some(GraphRunStopReason::RuntimeCancelled),
            _ => None,
        };
        run.last_decision = Some(effective_decision.clone());
        run.steps.push(step);
        run.updated_at_ms = now_timestamp_ms();

        let run = store.save_run(run);
        let event = build_run_event(
            &run,
            event_kind_for_decision(&effective_decision),
            effective_decision.summary.clone(),
        );

        Some(GraphRunAdvance {
            run,
            handoff,
            decision: effective_decision,
            event,
        })
    }

    pub fn request_stop(
        &self,
        store: &mut GraphRunStore,
        run_id: &str,
        reason: GraphRunStopReason,
        summary: impl Into<String>,
    ) -> Option<GraphRunLifecycle> {
        let mut run = store.load_run(run_id)?;
        if matches!(
            run.phase,
            GraphRunPhase::Completed | GraphRunPhase::Failed | GraphRunPhase::Cancelled
        ) {
            return None;
        }
        let decision = GraphDecision {
            kind: GraphDecisionKind::Pause,
            reason: GraphDecisionReason::ExplicitPause,
            summary: summary.into(),
            target_phase: GraphRunPhase::Paused,
        };
        run.phase = GraphRunPhase::Paused;
        run.stop_reason = Some(reason);
        run.last_decision = Some(decision.clone());
        run.updated_at_ms = now_timestamp_ms();
        let run = store.save_run(run);
        Some(GraphRunLifecycle {
            event: build_run_event(&run, GraphRunEventKind::Paused, decision.summary),
            run,
        })
    }

    pub fn resume_run(
        &self,
        store: &mut GraphRunStore,
        run_id: &str,
        summary: impl Into<String>,
    ) -> Option<GraphRunLifecycle> {
        let mut run = store.load_run(run_id)?;
        if run.phase != GraphRunPhase::Paused || run.active_turn_id.is_some() {
            return None;
        }
        run.phase = GraphRunPhase::Ready;
        run.stop_reason = None;
        run.resume_count = run.resume_count.saturating_add(1);
        run.updated_at_ms = now_timestamp_ms();
        let run = store.save_run(run);
        Some(GraphRunLifecycle {
            event: build_run_event(&run, GraphRunEventKind::Updated, summary.into()),
            run,
        })
    }

    pub fn build_checkpoint(&self, run: &GraphRun) -> GraphRunCheckpoint {
        GraphRunCheckpoint {
            contract_version: GRAPH_CONTRACT_VERSION.to_string(),
            run_id: run.id.clone(),
            goal: run.goal.clone(),
            session_id: run.session_id.clone(),
            phase: run.phase.clone(),
            active_turn_id: run.active_turn_id.clone(),
            last_completed_turn_id: run.last_completed_turn_id.clone(),
            stop_reason: run.stop_reason.clone(),
            steps: run.steps.clone(),
            last_decision: run.last_decision.clone(),
            last_handoff: run.last_handoff.clone(),
            resume_count: run.resume_count,
            control_boundary_evidence: run.control_boundary_evidence.clone(),
            resumable: matches!(
                run.phase,
                GraphRunPhase::Ready | GraphRunPhase::WaitingUser | GraphRunPhase::Paused
            ),
            created_at_ms: run.created_at_ms,
            updated_at_ms: run.updated_at_ms,
        }
    }

    pub fn record_control_boundary_evidence(
        &self,
        store: &mut GraphRunStore,
        run_id: &str,
        evidence: GraphRunControlBoundaryEvidence,
    ) -> Option<GraphRun> {
        let mut run = store.load_run(run_id)?;
        run.control_boundary_evidence.push(evidence);
        run.updated_at_ms = now_timestamp_ms();
        Some(store.save_run(run))
    }

    // ── Ask wait / resume (design.md Decision 5, PA-076 task 4.3/4.4) ─────────────────────────

    /// Bind an Ask `PendingControlRequest` to the run's `wait_user` suspension. The run is moved to
    /// `WaitingUser` (keyed on `request_id` + `expected_version`) and the original assistant
    /// tool-call transcript is preserved alongside the pending request. Binding a request_id that
    /// is already pending for the run fails closed.
    pub fn bind_ask_wait(
        &self,
        store: &mut GraphRunStore,
        run_id: &str,
        binding: GraphAskWaitBinding,
    ) -> Result<GraphRun, String> {
        let mut run = store
            .load_run(run_id)
            .ok_or_else(|| format!("Graph run `{run_id}` not found."))?;
        if matches!(
            run.phase,
            GraphRunPhase::Completed | GraphRunPhase::Failed | GraphRunPhase::Cancelled
        ) {
            return Err(format!(
                "Graph run `{run_id}` is terminal and cannot bind an Ask wait."
            ));
        }
        let waits = store.ask_waits.entry(run_id.to_string()).or_default();
        if waits
            .iter()
            .any(|existing| existing.request_id == binding.request_id)
        {
            return Err(format!(
                "Ask wait `{}` is already bound to run `{run_id}`.",
                binding.request_id
            ));
        }
        waits.push(binding);
        run.phase = GraphRunPhase::WaitingUser;
        run.active_turn_id = None;
        run.stop_reason = None;
        run.last_decision = Some(GraphDecision {
            kind: GraphDecisionKind::WaitUser,
            reason: GraphDecisionReason::TurnCompletedAwaitingUser,
            summary: format!(
                "Ask `{}` awaits a host answer before the run resumes.",
                waits
                    .last()
                    .map(|item| item.request_id.as_str())
                    .unwrap_or("request")
            ),
            target_phase: GraphRunPhase::WaitingUser,
        });
        run.updated_at_ms = now_timestamp_ms();
        Ok(store.save_run(run))
    }

    /// Snapshot of every Ask wait currently bound to a run.
    pub fn list_ask_waits(&self, store: &GraphRunStore, run_id: &str) -> Vec<GraphAskWaitBinding> {
        store.ask_waits.get(run_id).cloned().unwrap_or_default()
    }

    /// Resolve a bound Ask wait and return exactly one terminal tool result for the original
    /// `call_id`. The `expected_version` must match the version the binding was created with —
    /// a stale version (e.g. the version after the request was consumed) is rejected. On success
    /// the binding is removed and the run moves back to `Ready` to continue; no additional
    /// provider follow-up is produced.
    pub fn resume_ask_wait(
        &self,
        store: &mut GraphRunStore,
        run_id: &str,
        request_id: &str,
        expected_version: u64,
        answer: Value,
    ) -> Result<GraphAskResumeOutcome, String> {
        let mut run = store
            .load_run(run_id)
            .ok_or_else(|| format!("Graph run `{run_id}` not found."))?;
        let waits = store.ask_waits.get_mut(run_id).ok_or_else(|| {
            format!("No Ask waits are bound to run `{run_id}`.")
        })?;
        let position = waits
            .iter()
            .position(|binding| binding.request_id == request_id)
            .ok_or_else(|| {
                format!("No pending Ask wait `{request_id}` is bound to run `{run_id}`.")
            })?;
        let binding = waits[position].clone();
        if binding.expected_version != expected_version {
            return Err(format!(
                "Ask wait `{request_id}` was bound at version {} but resume presented version {expected_version}; stale resume rejected.",
                binding.expected_version
            ));
        }
        waits.remove(position);

        run.phase = GraphRunPhase::Ready;
        run.active_turn_id = None;
        run.stop_reason = None;
        run.resume_count = run.resume_count.saturating_add(1);
        run.updated_at_ms = now_timestamp_ms();
        let _ = store.save_run(run);

        let terminal_result = json!({
            "toolCallId": binding.call_id,
            "toolName": binding.tool_name,
            "status": "ok",
            "output": {
                "ok": true,
                "requestId": binding.request_id,
                "version": binding.expected_version,
                "kind": "waiting_user",
                "answer": answer.clone(),
            },
        });
        Ok(GraphAskResumeOutcome {
            request_id: binding.request_id,
            run_id: binding.run_id,
            turn_id: binding.turn_id,
            call_id: binding.call_id,
            tool_name: binding.tool_name,
            answer,
            terminal_result,
        })
    }
}

fn has_unresolved_ask_waits(store: &GraphRunStore, run_id: &str) -> bool {
    store
        .ask_waits
        .get(run_id)
        .map(|waits| !waits.is_empty())
        .unwrap_or(false)
}

#[allow(dead_code)]
fn normalize_handoff_phase(handoff: &GraphTurnHandoff) -> &str {
    let result_phase = normalize_phase_label(&handoff.turn_phase);
    match result_phase {
        "ready" | "completed" => handoff
            .checkpoint_status
            .as_deref()
            .map(normalize_phase_label)
            .or_else(|| {
                handoff
                    .checkpoint_phase
                    .as_deref()
                    .map(normalize_phase_label)
            })
            .unwrap_or(result_phase),
        _ => result_phase,
    }
}

fn extract_active_task_focus(
    entries: &[crate::agent::context::LongTermMemoryEntry],
) -> Option<String> {
    let content = entries
        .iter()
        .find(|entry| entry.kind == "project_focus.active_task")
        .map(|entry| entry.content.trim())?;

    first_task_like_token(content).or_else(|| {
        if content.is_empty() {
            None
        } else {
            Some(content.to_string())
        }
    })
}

fn extract_acceptance_focus(
    entries: &[crate::agent::context::LongTermMemoryEntry],
) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.kind == "project_workflow.acceptance_gate")
        .map(|entry| entry.content.trim())
        .filter(|content| !content.is_empty())
        .map(str::to_string)
}

fn extract_closeout_focus(
    entries: &[crate::agent::context::LongTermMemoryEntry],
) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.kind == "project_workflow.closeout_requirement")
        .map(|entry| entry.content.trim())
        .filter(|content| !content.is_empty())
        .map(str::to_string)
}

fn first_task_like_token(text: &str) -> Option<String> {
    let mut current = String::new();
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
            continue;
        }

        if looks_like_task_id(&current) {
            return Some(current);
        }
        current.clear();
    }

    None
}

fn looks_like_task_id(token: &str) -> bool {
    let mut parts = token.split('-');
    let Some(prefix) = parts.next() else {
        return false;
    };
    let Some(number) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    (2..=6).contains(&prefix.len())
        && prefix.chars().all(|ch| ch.is_ascii_uppercase())
        && (1..=6).contains(&number.len())
        && number.chars().all(|ch| ch.is_ascii_digit())
}

#[allow(dead_code)]
fn normalize_phase_label(value: &str) -> &str {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "completed" => "completed",
        "ready" => "ready",
        "running" => "running",
        "queued" => "queued",
        "calling_model" => "calling_model",
        "calling_tool" => "calling_tool",
        "waiting_user" => "waiting_user",
        "paused" => "paused",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "ready",
    }
}

#[allow(dead_code)]
fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg_attr(test, allow(dead_code))]
pub fn default_graph_run_store_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".pony-agent"))
        .join("PonyAgent")
        .join("graph-runs.json")
}

fn load_persisted_store(path: &PathBuf) -> PersistedGraphRunStore {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<PersistedGraphRunStore>(&contents).ok())
        .unwrap_or_default()
}

/// 检测并修复因进程崩溃/异常关闭导致的 stale graph run 状态。
///
/// 如果 run 的 phase 为 Running 但 active_turn_id 仍存在，说明上一个
/// turn 执行中被中断（进程崩溃、强制关闭等），`apply_turn_result` 从未
/// 被调用。此时：
/// - 清除 `active_turn_id`（不存在进行中的 turn）
/// - 将 phase 降级为 `Paused`（可 resume 继续对话）
/// - 清除 `last_decision`（保留旧决策会导致 phase 与决策不一致）
/// - 不设 `stop_reason`（这不是用户主动停止，也不是运行时错误）
///
/// 幂等的：第二次调用对已修复的 run 不产生任何修改。
///
/// 返回 `true` 表示至少修复了一个 run。
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn reconcile_stale_runs(runs: &mut GraphRunMap) -> bool {
    let mut modified = false;
    for run in runs.values_mut() {
        let stale = run.phase == GraphRunPhase::Running;
        if stale {
            let turn_id = run.active_turn_id.take();
            run.phase = GraphRunPhase::Paused;
            run.last_decision = None;
            run.updated_at_ms = now_timestamp_ms();
            modified = true;
            if let Some(tid) = turn_id {
                eprintln!(
                    "[pony-agent] reconciled stale graph run `{}`: \
                     active turn `{tid}` interrupted by process exit; moved to Paused.",
                    run.id,
                );
            } else {
                eprintln!(
                    "[pony-agent] reconciled stale graph run `{}`: \
                     phase=Running with no active turn; moved to Paused.",
                    run.id,
                );
            }
        }
    }
    modified
}

fn build_run_event(run: &GraphRun, kind: GraphRunEventKind, summary: String) -> GraphRunEvent {
    let (hook_point, canonical_event_type, canonical_phase) =
        graph_run_hook_annotation_for_event(&kind, &run.phase);
    GraphRunEvent {
        run_id: run.id.clone(),
        kind,
        phase: run.phase.clone(),
        summary,
        step_count: run.steps.len(),
        updated_at_ms: run.updated_at_ms,
        hook_point,
        canonical_event_type,
        canonical_phase,
    }
}

fn graph_run_hook_annotation_for_event(
    kind: &GraphRunEventKind,
    phase: &GraphRunPhase,
) -> (Option<String>, Option<String>, Option<String>) {
    match (kind, phase) {
        (GraphRunEventKind::Started, GraphRunPhase::Ready | GraphRunPhase::Running) => (
            Some("run_start".to_string()),
            Some("graph_run.started".to_string()),
            Some(graph_run_phase_token(phase)),
        ),
        (GraphRunEventKind::Updated, GraphRunPhase::WaitingUser) => (
            Some("wait_user".to_string()),
            Some("graph_run.updated".to_string()),
            Some("waiting_user".to_string()),
        ),
        (GraphRunEventKind::Paused, GraphRunPhase::Paused) => (
            Some("run_paused".to_string()),
            Some("graph_run.paused".to_string()),
            Some("paused".to_string()),
        ),
        (GraphRunEventKind::Completed, GraphRunPhase::Completed) => (
            Some("run_completed".to_string()),
            Some("graph_run.completed".to_string()),
            Some("completed".to_string()),
        ),
        (GraphRunEventKind::Failed, GraphRunPhase::Failed) => (
            Some("run_failed".to_string()),
            Some("graph_run.failed".to_string()),
            Some("failed".to_string()),
        ),
        (GraphRunEventKind::Cancelled, GraphRunPhase::Cancelled) => (
            Some("run_cancelled".to_string()),
            Some("graph_run.cancelled".to_string()),
            Some("cancelled".to_string()),
        ),
        _ => (None, None, None),
    }
}

fn graph_run_phase_token(phase: &GraphRunPhase) -> String {
    match phase {
        GraphRunPhase::Ready => "ready",
        GraphRunPhase::Running => "running",
        GraphRunPhase::WaitingUser => "waiting_user",
        GraphRunPhase::Paused => "paused",
        GraphRunPhase::Completed => "completed",
        GraphRunPhase::Failed => "failed",
        GraphRunPhase::Cancelled => "cancelled",
    }
    .to_string()
}

fn event_kind_for_decision(decision: &GraphDecision) -> GraphRunEventKind {
    match &decision.kind {
        GraphDecisionKind::Pause => GraphRunEventKind::Paused,
        GraphDecisionKind::Complete => GraphRunEventKind::Completed,
        GraphDecisionKind::Fail => GraphRunEventKind::Failed,
        GraphDecisionKind::Cancel => GraphRunEventKind::Cancelled,
        GraphDecisionKind::Continue | GraphDecisionKind::WaitUser => GraphRunEventKind::Updated,
    }
}

fn build_graph_step_title(user_message: &str) -> String {
    let normalized = user_message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if normalized.is_empty() {
        return "未命名轮次".to_string();
    }
    truncate_chars(&normalized, GRAPH_STEP_TITLE_MAX_CHARS)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    let mut count = 0;
    for ch in text.chars() {
        if count >= max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
        count += 1;
    }
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ask_control::{answer_ask, list_pending_asks};
    use crate::agent::context::{
        LongTermMemory, LongTermMemoryEntry, RunState, SessionContext, TranscriptContext,
        TurnContext,
    };
    use crate::agent::dispatcher::{
        ControlRequestAuthorization, DispatchContext, GovernedDispatcher,
    };
    use crate::agent::planner::DefaultGraphPlanner;
    use crate::agent::runtime::TurnResult;
    use crate::agent::tool_runtime::{
        FakeClock, InvocationOrigin, PendingControlRequestKind, PendingControlRequestState,
        RuntimeClock, ToolDispatchRequest,
    };
    use crate::agent::tools::{
        ToolControlKind, ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata,
        ToolExecutionPolicy, ToolExposure, ToolHandlerProvenance, ToolIdentity, ToolKind,
        ToolPermissionDeclaration, ToolRegistrySnapshot,
    };
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_result(phase: &str) -> TurnResult {
        TurnResult {
            event_id: None,
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: phase.to_string(),
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
            user_message: "继续完成任务".to_string(),
            assistant_message: "当前轮已完成。".to_string(),
            trace_steps: Vec::new(),
            trace_timeline: Vec::new(),
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: Vec::new(),
            session_summary: "session summary".to_string(),
        }
    }

    fn sample_retrieved() -> RetrievedContextState {
        RetrievedContextState {
            turn_context: TurnContext {
                user_message: "继续完成任务".to_string(),
                images: Vec::new(),
                references_image: false,
                workspace_mode: None,
            },
            session_context: SessionContext {
                conversation_id: "session-1".to_string(),
                title: "新对话".to_string(),
                summary: "session summary".to_string(),
                recent_history: Vec::new(),
                recent_attachment_assets: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
                env_info: None,
            },
            run_state: RunState {
                run_id: Some("run-1".to_string()),
                phase: Some("waiting_user".to_string()),
                execution_checkpoint_status: Some("completed".to_string()),
                execution_checkpoint_phase: Some("ready".to_string()),
                ..RunState::default()
            },
            long_term_memory: LongTermMemory {
                status: "available".to_string(),
                summary: Some("Stored long-term memory facts are available.".to_string()),
                entries: vec![
                    LongTermMemoryEntry {
                        kind: "project_focus.active_task".to_string(),
                        content: "Current active task is PA-018.".to_string(),
                        source: "explicit_user_message".to_string(),
                        updated_at_ms: 1,
                    },
                    LongTermMemoryEntry {
                        kind: "project_workflow.acceptance_gate".to_string(),
                        content:
                            "Establish acceptance criteria and run a closeout audit before claiming delivery."
                                .to_string(),
                        source: "explicit_user_message".to_string(),
                        updated_at_ms: 2,
                    },
                    LongTermMemoryEntry {
                        kind: "project_workflow.closeout_requirement".to_string(),
                        content:
                            "Summarize changed files, verification performed, and unresolved risks at closeout."
                                .to_string(),
                        source: "explicit_user_message".to_string(),
                        updated_at_ms: 3,
                    },
                    LongTermMemoryEntry {
                        kind: "project_scope.task_boundary".to_string(),
                        content: "Do not expand scope into PA-024, PA-025.".to_string(),
                        source: "explicit_user_message".to_string(),
                        updated_at_ms: 4,
                    },
                ],
            },
            transcript: TranscriptContext::default(),
        }
    }

    fn temp_graph_store_path() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("pony-agent-graph-store-{stamp}"))
            .join("graph-runs.json")
    }

    #[test]
    fn graph_engine_builds_turn_handoff_from_runtime_artifacts() {
        let engine = GraphEngine::new("state-machine-v1");
        let handoff = engine.build_turn_handoff(
            Some("turn-1"),
            Some("session-1"),
            &sample_result("ready"),
            &sample_retrieved(),
        );

        assert_eq!(handoff.contract_version, GRAPH_CONTRACT_VERSION);
        assert_eq!(handoff.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(handoff.session_id.as_deref(), Some("session-1"));
        assert_eq!(handoff.conversation_id, "session-1");
        assert_eq!(handoff.run_id.as_deref(), Some("run-1"));
        assert_eq!(handoff.run_phase.as_deref(), Some("waiting_user"));
        assert_eq!(handoff.checkpoint_status.as_deref(), Some("completed"));
        assert_eq!(handoff.checkpoint_phase.as_deref(), Some("ready"));
        assert_eq!(handoff.active_task_focus.as_deref(), Some("PA-018"));
        assert_eq!(
            handoff.acceptance_focus.as_deref(),
            Some(
                "Establish acceptance criteria and run a closeout audit before claiming delivery."
            )
        );
        assert_eq!(
            handoff.closeout_focus.as_deref(),
            Some(
                "Summarize changed files, verification performed, and unresolved risks at closeout."
            )
        );
        assert_eq!(handoff.long_term_memory_status, "available");
        assert_eq!(handoff.provider_name, "OpenAI");
    }

    #[test]
    fn graph_engine_uses_continue_boundary_while_runtime_is_still_running() {
        let engine = GraphEngine::new("state-machine-v1");
        let mut retrieved = sample_retrieved();
        retrieved.run_state.execution_checkpoint_status = Some("running".to_string());
        retrieved.run_state.execution_checkpoint_phase = Some("calling_tool".to_string());
        let handoff = engine.build_turn_handoff(
            Some("turn-1"),
            Some("session-1"),
            &sample_result("calling_tool"),
            &retrieved,
        );

        let decision = engine.decide_after_turn(&handoff);
        assert_eq!(decision.kind, GraphDecisionKind::Continue);
        assert_eq!(decision.target_phase, GraphRunPhase::Running);
    }

    #[test]
    fn graph_engine_uses_wait_user_boundary_after_completed_turn() {
        let engine = GraphEngine::new("state-machine-v1");
        let handoff = engine.build_turn_handoff(
            Some("turn-1"),
            Some("session-1"),
            &sample_result("ready"),
            &sample_retrieved(),
        );

        let decision = engine.decide_after_turn(&handoff);
        assert_eq!(decision.kind, GraphDecisionKind::WaitUser);
        assert_eq!(decision.target_phase, GraphRunPhase::WaitingUser);
    }

    #[test]
    fn graph_engine_can_defer_completed_turn_to_graph_planner() {
        let engine = GraphEngine::new("state-machine-v1");
        let planner = DefaultGraphPlanner;
        let run = engine.start_run(
            "run-continue",
            "逐步排查 provider 配置问题并收口",
            Some("session-1"),
        );
        let handoff = engine.build_turn_handoff(
            Some("turn-1"),
            Some("session-1"),
            &sample_result("ready"),
            &sample_retrieved(),
        );

        let decision = engine.decide_after_turn_with_planner(&run, &handoff, &planner);
        assert_eq!(decision.kind, GraphDecisionKind::Continue);
        assert_eq!(
            decision.reason,
            GraphDecisionReason::PlannerRequestedContinue
        );
        assert_eq!(decision.target_phase, GraphRunPhase::Ready);
    }

    #[test]
    fn graph_engine_maps_failed_and_cancelled_turns_to_terminal_boundaries() {
        let engine = GraphEngine::new("state-machine-v1");

        let failed = engine.build_turn_handoff(
            Some("turn-1"),
            Some("session-1"),
            &sample_result("failed"),
            &sample_retrieved(),
        );
        let cancelled = engine.build_turn_handoff(
            Some("turn-2"),
            Some("session-1"),
            &sample_result("cancelled"),
            &sample_retrieved(),
        );

        assert_eq!(
            engine.decide_after_turn(&failed).kind,
            GraphDecisionKind::Fail
        );
        assert_eq!(
            engine.decide_after_turn(&cancelled).kind,
            GraphDecisionKind::Cancel
        );
    }

    #[test]
    fn graph_runner_can_start_run_and_record_waiting_user_turn() {
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        let created = runner.start_run(
            &mut store,
            engine.start_run("run-1", "完成任务", Some("session-1")),
        );

        assert_eq!(created.event.kind, GraphRunEventKind::Started);
        assert_eq!(created.run.phase, GraphRunPhase::Ready);
        assert_eq!(created.event.hook_point.as_deref(), Some("run_start"));
        assert_eq!(
            created.event.canonical_event_type.as_deref(),
            Some("graph_run.started")
        );
        assert_eq!(created.event.canonical_phase.as_deref(), Some("ready"));

        let running = runner
            .begin_turn(&mut store, "run-1", "turn-1", Some("session-1"))
            .expect("run should be running");
        assert_eq!(running.run.phase, GraphRunPhase::Running);
        assert_eq!(running.run.active_turn_id.as_deref(), Some("turn-1"));

        let handoff = engine.build_turn_handoff(
            Some("turn-1"),
            Some("session-1"),
            &sample_result("ready"),
            &sample_retrieved(),
        );
        let decision = engine.decide_after_turn(&handoff);
        let advance = runner
            .apply_turn_result(&mut store, "run-1", handoff, decision)
            .expect("run should advance");

        assert_eq!(advance.run.steps.len(), 1);
        assert_eq!(advance.run.phase, GraphRunPhase::WaitingUser);
        assert_eq!(advance.event.kind, GraphRunEventKind::Updated);
        assert_eq!(advance.event.hook_point.as_deref(), Some("wait_user"));
        assert_eq!(
            advance.event.canonical_event_type.as_deref(),
            Some("graph_run.updated")
        );
        assert_eq!(
            advance.event.canonical_phase.as_deref(),
            Some("waiting_user")
        );
        assert_eq!(
            advance.run.last_decision.as_ref().map(|item| &item.kind),
            Some(&GraphDecisionKind::WaitUser)
        );
        assert_eq!(
            advance.run.last_completed_turn_id.as_deref(),
            Some("turn-1")
        );
        assert!(advance.run.last_handoff.is_some());
    }

    #[test]
    fn graph_runner_records_continue_decision_without_auto_looping_turns() {
        let engine = GraphEngine::new("state-machine-v1");
        let planner = DefaultGraphPlanner;
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run(
                "run-auto-continue",
                "逐步排查 provider 配置问题并收口",
                Some("session-1"),
            ),
        );

        let running = runner
            .begin_turn(&mut store, "run-auto-continue", "turn-1", Some("session-1"))
            .expect("run should be running");
        let handoff = engine.build_turn_handoff(
            Some("turn-1"),
            Some("session-1"),
            &sample_result("ready"),
            &sample_retrieved(),
        );
        let decision = engine.decide_after_turn_with_planner(&running.run, &handoff, &planner);
        let advance = runner
            .apply_turn_result(&mut store, "run-auto-continue", handoff, decision)
            .expect("run should advance");

        assert_eq!(advance.decision.kind, GraphDecisionKind::Continue);
        assert_eq!(advance.run.phase, GraphRunPhase::Ready);
        assert_eq!(advance.run.active_turn_id, None);
        assert_eq!(advance.run.steps.len(), 1);
    }

    #[test]
    fn graph_run_store_lists_latest_updated_run_first() {
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run("run-a", "目标 A", Some("session-a")),
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        runner.start_run(
            &mut store,
            engine.start_run("run-b", "目标 B", Some("session-b")),
        );

        let runs = store.list_runs();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, "run-b");
        assert_eq!(runs[1].id, "run-a");
    }

    #[test]
    fn graph_runner_can_pause_resume_and_build_checkpoint() {
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run("run-stop", "pause then resume", Some("session-stop")),
        );

        let paused = runner
            .request_stop(
                &mut store,
                "run-stop",
                GraphRunStopReason::UserStop,
                "stop requested",
            )
            .expect("run should pause");
        assert_eq!(paused.run.phase, GraphRunPhase::Paused);
        assert_eq!(paused.run.stop_reason, Some(GraphRunStopReason::UserStop));
        assert_eq!(paused.event.hook_point.as_deref(), Some("run_paused"));
        assert_eq!(
            paused.event.canonical_event_type.as_deref(),
            Some("graph_run.paused")
        );
        assert_eq!(paused.event.canonical_phase.as_deref(), Some("paused"));

        let checkpoint = runner.build_checkpoint(&paused.run);
        assert!(checkpoint.resumable);
        assert_eq!(checkpoint.phase, GraphRunPhase::Paused);
        assert_eq!(checkpoint.stop_reason, Some(GraphRunStopReason::UserStop));

        let resumed = runner
            .resume_run(&mut store, "run-stop", "resume requested")
            .expect("run should resume");
        assert_eq!(resumed.run.phase, GraphRunPhase::Ready);
        assert_eq!(resumed.run.resume_count, 1);
        assert_eq!(resumed.run.stop_reason, None);
        assert!(resumed.event.hook_point.is_none());
    }

    #[test]
    fn graph_checkpoint_can_preserve_non_user_stop_reasons() {
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run("run-timeout", "timeout stop", Some("session-timeout")),
        );

        let paused = runner
            .request_stop(
                &mut store,
                "run-timeout",
                GraphRunStopReason::Timeout,
                "timeout reached",
            )
            .expect("run should pause");
        let checkpoint = runner.build_checkpoint(&paused.run);

        assert_eq!(paused.run.stop_reason, Some(GraphRunStopReason::Timeout));
        assert_eq!(checkpoint.stop_reason, Some(GraphRunStopReason::Timeout));
        assert!(checkpoint.resumable);
    }

    #[test]
    fn persistent_graph_run_store_roundtrips_checkpointable_state() {
        let path = temp_graph_store_path();
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::persistent(path.clone());
        runner.start_run(
            &mut store,
            engine.start_run("run-persist", "persist checkpoint", Some("session-persist")),
        );
        let paused = runner
            .request_stop(
                &mut store,
                "run-persist",
                GraphRunStopReason::UserStop,
                "persist stop",
            )
            .expect("run should pause");
        let evidence = GraphRunControlBoundaryEvidence {
            hook_point: "stop_requested".to_string(),
            canonical_event_type: "graph_run.stop_requested".to_string(),
            canonical_phase: "running".to_string(),
            summary: "persisted stop request".to_string(),
            hook_envelope: crate::agent::hooks::RunControlHookEnvelope {
                session_id: Some("session-persist".to_string()),
                run_id: Some("run-persist".to_string()),
                phase: "running".to_string(),
                command: crate::agent::hooks::ExecutionControlCommandKind::StopGraphRun,
                source: "graph.test".to_string(),
                checkpoint_kind: Some("runtime_control".to_string()),
                recovery_mode: Some("replay_required".to_string()),
                resumable: false,
                replayable: false,
            },
            created_at_ms: paused.run.updated_at_ms,
        };
        let persisted = runner
            .record_control_boundary_evidence(&mut store, "run-persist", evidence.clone())
            .expect("control boundary evidence should persist");
        assert_eq!(persisted.control_boundary_evidence.len(), 1);
        drop(store);

        let reloaded = GraphRunStore::persistent(path.clone());
        let run = reloaded
            .load_run("run-persist")
            .expect("persisted run should exist");
        let checkpoint = runner.build_checkpoint(&run);

        assert_eq!(run.phase, GraphRunPhase::Paused);
        assert_eq!(run.stop_reason, Some(GraphRunStopReason::UserStop));
        assert_eq!(run.control_boundary_evidence, vec![evidence.clone()]);
        assert_eq!(checkpoint.run_id, paused.run.id);
        assert_eq!(checkpoint.control_boundary_evidence, vec![evidence]);
        assert!(checkpoint.resumable);

        let mut reloaded = reloaded;
        let resumed = runner
            .resume_run(&mut reloaded, "run-persist", "resume after reload")
            .expect("persisted run should resume");
        assert_eq!(resumed.run.phase, GraphRunPhase::Ready);
        assert_eq!(resumed.run.resume_count, 1);

        let _ = fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    // ------------------------------------------------------------------
    // reconcile_stale_runs tests
    // ------------------------------------------------------------------

    fn stale_run(run_id: &str, phase: GraphRunPhase) -> GraphRun {
        GraphRun {
            id: run_id.to_string(),
            goal: "test".to_string(),
            session_id: Some("session-1".to_string()),
            phase,
            steps: Vec::new(),
            active_turn_id: Some("turn-1".to_string()),
            last_completed_turn_id: Some("turn-0".to_string()),
            stop_reason: None,
            last_handoff: None,
            resume_count: 0,
            last_decision: Some(GraphDecision {
                kind: GraphDecisionKind::Continue,
                reason: GraphDecisionReason::PlannerRequestedContinue,
                summary: "previous decision".to_string(),
                target_phase: GraphRunPhase::Running,
            }),
            control_boundary_evidence: Vec::new(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    fn healthy_run(run_id: &str, phase: GraphRunPhase) -> GraphRun {
        GraphRun {
            id: run_id.to_string(),
            goal: "test".to_string(),
            session_id: Some("session-1".to_string()),
            phase,
            steps: Vec::new(),
            active_turn_id: None,
            last_completed_turn_id: Some("turn-1".to_string()),
            stop_reason: None,
            last_handoff: None,
            resume_count: 1,
            last_decision: Some(GraphDecision {
                kind: GraphDecisionKind::Pause,
                reason: GraphDecisionReason::ExplicitPause,
                summary: "stopped".to_string(),
                target_phase: GraphRunPhase::Paused,
            }),
            control_boundary_evidence: Vec::new(),
            created_at_ms: 1000,
            updated_at_ms: 2000,
        }
    }

    #[test]
    fn reconcile_stale_runs_fixes_running_with_active_turn() {
        let mut runs = GraphRunMap::new();
        runs.insert("run-1".into(), stale_run("run-1", GraphRunPhase::Running));

        let modified = reconcile_stale_runs(&mut runs);
        assert!(modified, "should have modified the stale run");

        let run = runs.get("run-1").unwrap();
        assert_eq!(run.phase, GraphRunPhase::Paused, "phase should be Paused");
        assert_eq!(run.active_turn_id, None, "active_turn_id should be cleared");
        assert_eq!(run.last_decision, None, "last_decision should be cleared");
        assert_eq!(run.stop_reason, None, "stop_reason should remain None");
        assert!(
            run.updated_at_ms > 1000,
            "updated_at_ms should be refreshed"
        );
    }

    #[test]
    fn reconcile_stale_runs_fixes_running_without_active_turn() {
        let mut runs = GraphRunMap::new();
        let mut run = stale_run("run-1", GraphRunPhase::Running);
        run.active_turn_id = None; // Running but no active turn — safety net
        runs.insert("run-1".into(), run);

        let modified = reconcile_stale_runs(&mut runs);
        assert!(modified, "should have modified the stale run");

        let run = runs.get("run-1").unwrap();
        assert_eq!(run.phase, GraphRunPhase::Paused);
        assert_eq!(run.last_decision, None);
    }

    #[test]
    fn reconcile_stale_runs_skips_terminal_runs() {
        let mut runs = GraphRunMap::new();

        for (phase_key, phase) in [
            ("completed", GraphRunPhase::Completed),
            ("failed", GraphRunPhase::Failed),
            ("cancelled", GraphRunPhase::Cancelled),
        ] {
            runs.insert(
                format!("run-{phase_key}"),
                stale_run(&format!("run-{phase_key}"), phase),
            );
        }

        let modified = reconcile_stale_runs(&mut runs);
        assert!(!modified, "terminal runs should not be modified");
    }

    #[test]
    fn reconcile_stale_runs_skips_healthy_paused_run() {
        let mut runs = GraphRunMap::new();
        runs.insert("run-1".into(), healthy_run("run-1", GraphRunPhase::Paused));

        let modified = reconcile_stale_runs(&mut runs);
        assert!(!modified, "healthy Paused run should not be modified");
    }

    #[test]
    fn reconcile_stale_runs_skips_healthy_waiting_run() {
        let mut runs = GraphRunMap::new();
        runs.insert(
            "run-1".into(),
            healthy_run("run-1", GraphRunPhase::WaitingUser),
        );

        let modified = reconcile_stale_runs(&mut runs);
        assert!(!modified, "healthy WaitingUser run should not be modified");
    }

    #[test]
    fn reconcile_stale_runs_is_idempotent() {
        let mut runs = GraphRunMap::new();
        runs.insert("run-1".into(), stale_run("run-1", GraphRunPhase::Running));

        // First call
        reconcile_stale_runs(&mut runs);
        let snapshot = runs.clone();

        // Second call — should be no-op
        let modified = reconcile_stale_runs(&mut runs);
        assert!(!modified, "second call should not modify anything");
        assert_eq!(
            runs, snapshot,
            "state should be identical after second call"
        );
    }

    #[test]
    fn reconcile_stale_runs_skips_ready_run() {
        let mut runs = GraphRunMap::new();
        runs.insert("run-1".into(), healthy_run("run-1", GraphRunPhase::Ready));

        let modified = reconcile_stale_runs(&mut runs);
        assert!(!modified, "Ready run should not be modified");
    }

    // ------------------------------------------------------------------
    // Ask wait / resume binding (design.md Decision 5, task 4.3/4.4)
    // ------------------------------------------------------------------

    fn ask_descriptor() -> ToolDescriptor {
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

    fn ask_registry() -> Arc<ToolRegistrySnapshot> {
        Arc::new(
            ToolRegistrySnapshot::from_descriptors("test-ask-snapshot", vec![ask_descriptor()])
                .expect("ask registry must build"),
        )
    }

    /// Persist a real `Interaction` pending request through `dispatch_governed` with a
    /// host-mediated Ask descriptor.
    fn persist_ask_via_dispatch(
        dispatcher: &GovernedDispatcher,
    ) -> (String, crate::agent::tool_runtime::PendingControlRequest) {
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
        let control = outcome.control_outcome.expect("ask control outcome");
        assert_eq!(control.kind, ToolControlKind::WaitingHost);
        let pending = dispatcher
            .pending_request(&control.request_id)
            .expect("persisted pending ask");
        (control.request_id, pending)
    }

    fn sample_ask_binding(
        run_id: &str,
        request_id: &str,
        expected_version: u64,
        call_id: &str,
        session_id: Option<&str>,
    ) -> GraphAskWaitBinding {
        GraphAskWaitBinding {
            request_id: request_id.to_string(),
            expected_version,
            run_id: run_id.to_string(),
            turn_id: "turn-1".to_string(),
            session_id: session_id.map(str::to_string),
            call_id: call_id.to_string(),
            tool_name: "Ask".to_string(),
            assistant_transcript: json!({
                "role": "assistant",
                "toolCalls": [{ "id": call_id, "type": "function", "function": { "name": "Ask" } }],
            }),
            created_at_ms: 1_000,
        }
    }

    #[test]
    fn graph_bind_ask_wait_suspends_run_and_duplicate_binding_fails_closed() {
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run("run-ask", "ask flow", Some("session-1")),
        );

        let binding =
            sample_ask_binding("run-ask", "control-000000000001", 1, "call-1", Some("session-1"));
        let bound = runner
            .bind_ask_wait(&mut store, "run-ask", binding.clone())
            .expect("bind ask wait");
        assert_eq!(bound.phase, GraphRunPhase::WaitingUser);
        assert_eq!(bound.active_turn_id, None);
        assert_eq!(
            bound.last_decision.as_ref().map(|item| &item.kind),
            Some(&GraphDecisionKind::WaitUser)
        );

        let waits = runner.list_ask_waits(&store, "run-ask");
        assert_eq!(waits, vec![binding.clone()]);

        assert!(
            runner.bind_ask_wait(&mut store, "run-ask", binding).is_err(),
            "a duplicate request_id binding must fail closed"
        );
        assert!(runner.list_ask_waits(&store, "run-ask").len() == 1);
    }

    #[test]
    fn ask_wait_resume_injects_exactly_one_terminal_result_for_original_call_id() {
        let dispatcher =
            GovernedDispatcher::new(ask_registry(), Arc::new(FakeClock::new(1_000)));
        let (request_id, pending) = persist_ask_via_dispatch(&dispatcher);
        assert_eq!(pending.request_kind, PendingControlRequestKind::Interaction);
        assert_eq!(pending.call_id, "call-1");

        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run("run-ask", "ask flow", Some("session-1")),
        );
        runner
            .bind_ask_wait(
                &mut store,
                "run-ask",
                sample_ask_binding(
                    "run-ask",
                    &request_id,
                    pending.version,
                    &pending.call_id,
                    pending.session_id.as_deref(),
                ),
            )
            .expect("bind ask wait");

        // The host observes the pending Ask and answers it through the Ask control path.
        let asks = list_pending_asks(&dispatcher);
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].request_id, request_id);
        let authorization =
            ControlRequestAuthorization::for_request(&pending, Some(json!("continue")));
        let consumed = answer_ask(&dispatcher, &request_id, &authorization)
            .expect("answer consumes the ask");
        assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);

        // Resume: exactly one terminal tool result for the original call id, then the run
        // continues at Ready. No additional provider follow-up is produced by the graph.
        let outcome = runner
            .resume_ask_wait(
                &mut store,
                "run-ask",
                &request_id,
                pending.version,
                json!("continue"),
            )
            .expect("resume ask wait");
        assert_eq!(outcome.call_id, pending.call_id);
        assert_eq!(outcome.request_id, request_id);
        assert_eq!(outcome.turn_id, "turn-1");
        assert_eq!(outcome.answer, json!("continue"));
        assert_eq!(
            outcome.terminal_result["toolCallId"].as_str(),
            Some(pending.call_id.as_str())
        );
        assert_eq!(outcome.terminal_result["toolName"].as_str(), Some("Ask"));
        assert_eq!(
            outcome.terminal_result["output"]["answer"].as_str(),
            Some("continue")
        );
        assert!(
            outcome.terminal_result.is_object(),
            "the terminal result is a single result, not a batch"
        );

        let run = store.load_run("run-ask").expect("run present");
        assert_eq!(run.phase, GraphRunPhase::Ready);
        assert_eq!(run.resume_count, 1);
        assert!(runner.list_ask_waits(&store, "run-ask").is_empty());
    }

    #[test]
    fn ask_wait_resume_rejects_stale_version_and_keeps_the_binding() {
        let dispatcher =
            GovernedDispatcher::new(ask_registry(), Arc::new(FakeClock::new(1_000)));
        let (request_id, pending) = persist_ask_via_dispatch(&dispatcher);

        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run("run-ask", "ask flow", Some("session-1")),
        );
        runner
            .bind_ask_wait(
                &mut store,
                "run-ask",
                sample_ask_binding(
                    "run-ask",
                    &request_id,
                    pending.version,
                    &pending.call_id,
                    pending.session_id.as_deref(),
                ),
            )
            .expect("bind ask wait");

        let error = runner
            .resume_ask_wait(
                &mut store,
                "run-ask",
                &request_id,
                pending.version + 1,
                json!("continue"),
            )
            .expect_err("a stale version must be rejected");
        assert!(error.contains("stale"), "{error}");
        assert_eq!(
            runner.list_ask_waits(&store, "run-ask").len(),
            1,
            "a rejected resume must leave the binding intact"
        );
        assert_eq!(
            store.load_run("run-ask").expect("run").phase,
            GraphRunPhase::WaitingUser
        );
    }

    #[test]
    fn ask_wait_bindings_persist_across_store_reload() {
        let path = temp_graph_store_path();
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::persistent(path.clone());
        runner.start_run(
            &mut store,
            engine.start_run(
                "run-ask-persist",
                "persist ask wait",
                Some("session-persist"),
            ),
        );
        runner
            .bind_ask_wait(
                &mut store,
                "run-ask-persist",
                sample_ask_binding(
                    "run-ask-persist",
                    "control-000000000001",
                    1,
                    "call-1",
                    Some("session-persist"),
                ),
            )
            .expect("bind ask wait");
        drop(store);

        let reloaded = GraphRunStore::persistent(path.clone());
        let waits = runner.list_ask_waits(&reloaded, "run-ask-persist");
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].request_id, "control-000000000001");
        assert_eq!(waits[0].expected_version, 1);
        let run = reloaded.load_run("run-ask-persist").expect("reloaded run");
        assert_eq!(run.phase, GraphRunPhase::WaitingUser);
        assert!(runner.build_checkpoint(&run).resumable);

        let _ = fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn begin_turn_and_apply_turn_result_are_rejected_while_an_ask_wait_is_pending() {
        let engine = GraphEngine::new("state-machine-v1");
        let runner = GraphRunner::new();
        let mut store = GraphRunStore::new();
        runner.start_run(
            &mut store,
            engine.start_run("run-ask", "ask flow", Some("session-1")),
        );
        runner
            .bind_ask_wait(
                &mut store,
                "run-ask",
                sample_ask_binding("run-ask", "control-1", 1, "call-1", Some("session-1")),
            )
            .expect("bind ask wait");

        assert!(
            runner
                .begin_turn(&mut store, "run-ask", "turn-2", Some("session-1"))
                .is_none(),
            "begin_turn must be refused while an Ask wait is pending"
        );
        let handoff = engine.build_turn_handoff(
            Some("turn-2"),
            Some("session-1"),
            &sample_result("ready"),
            &sample_retrieved(),
        );
        let decision = engine.decide_after_turn(&handoff);
        assert!(
            runner
                .apply_turn_result(&mut store, "run-ask", handoff, decision)
                .is_none(),
            "apply_turn_result must be refused while an Ask wait is pending"
        );
        assert_eq!(
            store.load_run("run-ask").expect("run").phase,
            GraphRunPhase::WaitingUser
        );
    }
}
