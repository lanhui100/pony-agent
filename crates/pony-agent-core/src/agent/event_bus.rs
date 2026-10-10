use std::sync::Arc;
use tokio::sync::broadcast;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload")]
pub enum AgentEvent {
    SessionCreated { session_id: String },
    SessionArchived { session_id: String },
    TurnStarted { session_id: String, turn_id: String },
    TurnCompleted { session_id: String, turn_id: String },
    TurnFailed { session_id: String, turn_id: String, error: String },
    Custom { topic: String, data: serde_json::Value },
}

#[derive(Clone)]
pub struct AgentEventBus {
    sender: broadcast::Sender<AgentEvent>,
}

impl AgentEventBus {
    pub fn new(_capacity: usize) -> Self {
        unimplemented!("Stub: AgentEventBus::new")
    }

    pub fn publish(&self, _event: AgentEvent) -> Result<usize, broadcast::error::SendError<AgentEvent>> {
        unimplemented!("Stub: AgentEventBus::publish")
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        unimplemented!("Stub: AgentEventBus::subscribe")
    }
}
