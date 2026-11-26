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
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    pub fn publish(&self, event: AgentEvent) -> Result<usize, broadcast::error::SendError<AgentEvent>> {
        self.sender.send(event)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.sender.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for AgentEventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}
