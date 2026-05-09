use crate::agent::graph::GraphEngine;

pub struct AgentRuntime {
    graph: GraphEngine,
}

impl AgentRuntime {
    pub fn new() -> Self {
        Self {
            graph: GraphEngine::new("state-machine-v1"),
        }
    }

    pub fn name(&self) -> &'static str {
        "rust-core"
    }

    pub fn graph_engine(&self) -> &str {
        self.graph.name()
    }
}
