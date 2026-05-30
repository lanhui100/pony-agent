use crate::agent::telemetry::{TurnToolActivity, TurnTraceStep};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCheckpoint {
    pub turn_id: String,
    pub session_id: Option<String>,
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

    pub fn register_turn(&self, turn_id: &str, session_id: Option<&str>) {
        let now = now_timestamp_ms();
        let mut state = self.state.lock().expect("execution control lock poisoned");
        state.turns.insert(
            turn_id.to_string(),
            ExecutionCheckpoint {
                turn_id: turn_id.to_string(),
                session_id: session_id.map(str::to_string),
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
                error: None,
                started_at_ms: now,
                updated_at_ms: now,
                stop_requested_at_ms: None,
            },
        );
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

        registry.register_turn("turn-1", Some("session-1"));
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
        registry.register_turn("turn-1", Some("session-1"));
        std::thread::sleep(std::time::Duration::from_millis(2));
        registry.register_turn("turn-2", Some("session-1"));

        let checkpoint = registry
            .load_checkpoint(None, Some("session-1"))
            .expect("session checkpoint");

        assert_eq!(checkpoint.turn_id, "turn-2");
    }
}
