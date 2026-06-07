use crate::agent::context::RetrievedContextState;
use crate::agent::hooks::RunControlHookEnvelope;
use crate::agent::planner::{GraphPlanner, GraphPlanningContext};
use crate::agent::runtime::TurnResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
            storage_path: None,
        }
    }

    pub fn persistent(storage_path: impl Into<PathBuf>) -> Self {
        let storage_path = storage_path.into();
        Self {
            runs: load_runs_from_path(&storage_path),
            storage_path: Some(storage_path),
        }
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

fn load_runs_from_path(path: &PathBuf) -> GraphRunMap {
    let Ok(contents) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str::<PersistedGraphRunStore>(&contents)
        .map(|persisted| persisted.runs)
        .unwrap_or_default()
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
    use crate::agent::context::{
        LongTermMemory, LongTermMemoryEntry, RunState, SessionContext, TranscriptContext,
        TurnContext,
    };
    use crate::agent::planner::DefaultGraphPlanner;
    use crate::agent::runtime::TurnResult;
    use std::fs;
    use std::path::PathBuf;
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
            },
            session_context: SessionContext {
                conversation_id: "session-1".to_string(),
                title: "新对话".to_string(),
                summary: "session summary".to_string(),
                recent_history: Vec::new(),
                recent_attachment_assets: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
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
}
