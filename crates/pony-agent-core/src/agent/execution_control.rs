use crate::agent::hooks::PersistedEffectEvidence;
use crate::agent::telemetry::{TurnToolActivity, TurnTraceStep};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCheckpoint {
    pub contract_version: String,
    pub turn_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub checkpoint_kind: String,
    pub recovery_mode: String,
    pub projected_runtime_phase: String,
    pub submission_command: Option<String>,
    pub resumable: bool,
    pub replayable: bool,
    pub status: String,
    pub phase: String,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub completed_hops: usize,
    pub max_hops: usize,
    pub active_tool_name: Option<String>,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
    #[serde(default)]
    pub persisted_effect_evidence: Vec<PersistedEffectEvidence>,
    pub error: Option<String>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub stop_requested_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopTurnResponse {
    pub turn_id: String,
    pub accepted: bool,
    pub state: String,
}

#[derive(Default)]
struct RegistryState {
    turns: HashMap<String, ExecutionCheckpoint>,
}

pub struct ExecutionControlRegistry {
    state: Mutex<RegistryState>,
}

impl ExecutionControlRegistry {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(RegistryState::default()),
        }
    }

    pub fn register_turn(&self, turn_id: &str, session_id: Option<&str>, run_id: Option<&str>) {
        let now = now_timestamp_ms();
        let mut state = self.state.lock().expect("execution control lock poisoned");
        let mut checkpoint = ExecutionCheckpoint {
            contract_version: execution_checkpoint_contract_version().to_string(),
            turn_id: turn_id.to_string(),
            session_id: session_id.map(str::to_string),
            run_id: run_id.map(str::to_string),
            checkpoint_kind: "runtime_control".to_string(),
            recovery_mode: "replay_required".to_string(),
            projected_runtime_phase: "connecting".to_string(),
            submission_command: None,
            resumable: false,
            replayable: false,
            status: "running".to_string(),
            phase: "queued".to_string(),
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            completed_hops: 0,
            max_hops: 0,
            active_tool_name: None,
            trace_steps: Vec::new(),
            tool_activities: Vec::new(),
            persisted_effect_evidence: Vec::new(),
            error: None,
            started_at_ms: now,
            updated_at_ms: now,
            stop_requested_at_ms: None,
        };
        refresh_execution_checkpoint_projection(&mut checkpoint);
        state.turns.insert(turn_id.to_string(), checkpoint);
    }

    pub fn update<F>(&self, turn_id: &str, update: F)
    where
        F: FnOnce(&mut ExecutionCheckpoint),
    {
        let mut state = self.state.lock().expect("execution control lock poisoned");
        let Some(checkpoint) = state.turns.get_mut(turn_id) else {
            return;
        };
        update(checkpoint);
        checkpoint.contract_version = execution_checkpoint_contract_version().to_string();
        refresh_execution_checkpoint_projection(checkpoint);
        checkpoint.updated_at_ms = now_timestamp_ms();
    }

    pub fn request_stop(&self, turn_id: &str) -> StopTurnResponse {
        let mut state = self.state.lock().expect("execution control lock poisoned");
        let Some(checkpoint) = state.turns.get_mut(turn_id) else {
            return StopTurnResponse {
                turn_id: turn_id.to_string(),
                accepted: false,
                state: "not_found".to_string(),
            };
        };

        if checkpoint.status != "running" {
            return StopTurnResponse {
                turn_id: turn_id.to_string(),
                accepted: false,
                state: "already_terminal".to_string(),
            };
        }

        if checkpoint.stop_requested_at_ms.is_none() {
            checkpoint.stop_requested_at_ms = Some(now_timestamp_ms());
            checkpoint.updated_at_ms = checkpoint
                .stop_requested_at_ms
                .unwrap_or(checkpoint.updated_at_ms);
        }

        StopTurnResponse {
            turn_id: turn_id.to_string(),
            accepted: true,
            state: "running".to_string(),
        }
    }

    pub fn is_stop_requested(&self, turn_id: &str) -> bool {
        let state = self.state.lock().expect("execution control lock poisoned");
        state
            .turns
            .get(turn_id)
            .and_then(|checkpoint| checkpoint.stop_requested_at_ms)
            .is_some()
    }

    pub fn load_checkpoint(
        &self,
        turn_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Option<ExecutionCheckpoint> {
        let state = self.state.lock().expect("execution control lock poisoned");
        if let Some(turn_id) = turn_id {
            return state.turns.get(turn_id).cloned();
        }

        let session_id = session_id?;
        state
            .turns
            .values()
            .filter(|checkpoint| checkpoint.session_id.as_deref() == Some(session_id))
            .max_by_key(|checkpoint| checkpoint.updated_at_ms)
            .cloned()
    }
}

pub fn execution_checkpoint_contract_version() -> &'static str {
    "execution-checkpoint-v1"
}

pub fn refresh_execution_checkpoint_projection(checkpoint: &mut ExecutionCheckpoint) {
    checkpoint.projected_runtime_phase = projected_runtime_phase_for(checkpoint);
    checkpoint.submission_command = projected_submission_command_for(checkpoint);
}

fn projected_submission_command_for(checkpoint: &ExecutionCheckpoint) -> Option<String> {
    if checkpoint.checkpoint_kind != "recovery" {
        return None;
    }

    if checkpoint.recovery_mode == "replay_required" {
        return Some("start_graph_run_stream".to_string());
    }

    if checkpoint.run_id.is_none() {
        return None;
    }

    let normalized_phase = normalize_phase_token(&checkpoint.phase);
    let normalized_status = normalize_phase_token(&checkpoint.status);
    if normalized_phase == "failed"
        || normalized_phase == "cancelled"
        || normalized_status == "failed"
        || normalized_status == "cancelled"
    {
        return None;
    }

    if normalized_phase == "paused" {
        return Some("resume_graph_run_stream".to_string());
    }

    if (checkpoint.resumable && normalized_status == "ready")
        || normalized_phase == "ready"
        || normalized_phase == "waiting_user"
        || normalized_phase == "completed"
    {
        return Some("continue_graph_run_stream".to_string());
    }

    None
}

fn projected_runtime_phase_for(checkpoint: &ExecutionCheckpoint) -> String {
    if checkpoint.checkpoint_kind == "recovery" {
        let normalized_status = normalize_phase_token(&checkpoint.status);
        if normalized_status == "failed" {
            return "failed".to_string();
        }
        if normalized_status == "cancelled" {
            return "cancelled".to_string();
        }
        return "ready".to_string();
    }

    if let Some(phase) = normalize_runtime_phase_value(&checkpoint.phase) {
        return phase.to_string();
    }

    if checkpoint.active_tool_name.is_some()
        || checkpoint
            .tool_activities
            .iter()
            .any(|tool| tool.status == "running")
    {
        return "calling_tool".to_string();
    }

    if let Some(phase) = map_lifecycle_phase_to_runtime_phase(&checkpoint.phase) {
        return phase.to_string();
    }

    let normalized_status = normalize_phase_token(&checkpoint.status);
    if normalized_status == "cancelled" {
        return "cancelled".to_string();
    }
    if normalized_status == "failed" {
        return "failed".to_string();
    }
    "calling_model".to_string()
}

fn normalize_runtime_phase_value(phase: &str) -> Option<&'static str> {
    match normalize_phase_token(phase).as_str() {
        "idle" => Some("idle"),
        "connecting" => Some("connecting"),
        "ready" => Some("ready"),
        "completed" => Some("completed"),
        "cancelled" => Some("cancelled"),
        "calling_model" => Some("calling_model"),
        "calling_tool" => Some("calling_tool"),
        "failed" => Some("failed"),
        _ => None,
    }
}

fn map_lifecycle_phase_to_runtime_phase(phase: &str) -> Option<&'static str> {
    match normalize_phase_token(phase).as_str() {
        "created" | "preparing" | "building_context" | "checkpointing" | "queued" => {
            Some("connecting")
        }
        "calling_model" | "streaming_response" | "tool_result_integrating" => Some("calling_model"),
        "executing_tool" => Some("calling_tool"),
        "completed" => Some("completed"),
        "failed" => Some("failed"),
        "cancelled" => Some("cancelled"),
        other => normalize_runtime_phase_value(other),
    }
}

fn normalize_phase_token(value: &str) -> String {
    value.trim().to_lowercase().replace('-', "_")
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

    #[test]
    fn request_stop_rejects_missing_and_terminal_turns() {
        let registry = ExecutionControlRegistry::new();

        let missing = registry.request_stop("missing");
        assert!(!missing.accepted);
        assert_eq!(missing.state, "not_found");

        registry.register_turn("turn-1", Some("session-1"), None);
        registry.update("turn-1", |checkpoint| {
            checkpoint.status = "completed".to_string();
            checkpoint.phase = "ready".to_string();
        });

        let terminal = registry.request_stop("turn-1");
        assert!(!terminal.accepted);
        assert_eq!(terminal.state, "already_terminal");
    }

    #[test]
    fn load_checkpoint_can_fallback_to_latest_session_turn() {
        let registry = ExecutionControlRegistry::new();
        registry.register_turn("turn-1", Some("session-1"), None);
        std::thread::sleep(std::time::Duration::from_millis(2));
        registry.register_turn("turn-2", Some("session-1"), None);

        let checkpoint = registry
            .load_checkpoint(None, Some("session-1"))
            .expect("session checkpoint");

        assert_eq!(checkpoint.turn_id, "turn-2");
    }

    #[test]
    fn recovery_checkpoint_projection_exposes_submission_command_and_projected_phase() {
        let mut checkpoint = ExecutionCheckpoint {
            contract_version: execution_checkpoint_contract_version().to_string(),
            turn_id: "turn-recovery".to_string(),
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            checkpoint_kind: "recovery".to_string(),
            recovery_mode: "persisted_effect".to_string(),
            projected_runtime_phase: String::new(),
            submission_command: None,
            resumable: true,
            replayable: true,
            status: "ready".to_string(),
            phase: "paused".to_string(),
            provider_requested_name: None,
            provider_name: None,
            provider_protocol: None,
            provider_model: None,
            provider_source: None,
            provider_mode: None,
            fallback_reason: None,
            completed_hops: 0,
            max_hops: 0,
            active_tool_name: None,
            trace_steps: Vec::new(),
            tool_activities: Vec::new(),
            persisted_effect_evidence: Vec::new(),
            error: None,
            started_at_ms: 0,
            updated_at_ms: 0,
            stop_requested_at_ms: None,
        };

        refresh_execution_checkpoint_projection(&mut checkpoint);
        assert_eq!(checkpoint.projected_runtime_phase, "ready");
        assert_eq!(
            checkpoint.submission_command.as_deref(),
            Some("resume_graph_run_stream")
        );

        checkpoint.phase = "ready".to_string();
        refresh_execution_checkpoint_projection(&mut checkpoint);
        assert_eq!(
            checkpoint.submission_command.as_deref(),
            Some("continue_graph_run_stream")
        );

        checkpoint.recovery_mode = "replay_required".to_string();
        refresh_execution_checkpoint_projection(&mut checkpoint);
        assert_eq!(
            checkpoint.submission_command.as_deref(),
            Some("start_graph_run_stream")
        );
    }
}
